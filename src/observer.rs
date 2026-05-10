use coap_lite::{
    block_handler::BlockValue, CoapOption, CoapRequest, MessageClass, MessageType, ObserveOption,
    Packet, RequestType as Method, ResponseType as Status,
};
use futures::{
    stream::{Fuse, SelectNextSome},
    StreamExt,
};
use log::{debug, warn};
use std::{
    collections::{hash_map::Entry, HashMap, HashSet},
    net::SocketAddr,
    sync::Arc,
    time::Duration,
};
use tokio::time::interval;
use tokio_stream::wrappers::IntervalStream;

use crate::server::Responder;

const DEFAULT_UNACKNOWLEDGE_MESSAGE_TRY_TIMES: usize = 10;

pub struct Observer {
    registers: HashMap<String, RegisterItem>,
    resources: HashMap<String, ResourceItem>,
    register_resources: HashMap<String, RegisterResourceItem>,
    unacknowledge_messages: HashMap<u16, UnacknowledgeMessageItem>,
    current_message_id: u16,
    timer: Fuse<IntervalStream>,
}

#[derive(Debug)]
struct RegisterItem {
    register_resources: HashSet<String>,
}

#[derive(Debug)]
struct ResourceItem {
    payload: Arc<Vec<u8>>,
    register_resources: HashSet<String>,
    sequence: u32,
    etag: Vec<u8>,
}

struct RegisterResourceItem {
    pub(crate) registered_responder: Arc<dyn Responder>,
    pub(crate) resource: String,
    pub(crate) token: Vec<u8>,
    pub(crate) unacknowledge_message: Option<u16>,
    pub(crate) preferred_block_size: Option<usize>,
}

#[derive(Debug)]
struct UnacknowledgeMessageItem {
    register_resource: String,
    try_times: usize,
}

/// Encodes a usize as a CoAP uint (big-endian, variable length up to 4 bytes).
/// The value 0 is encoded as an empty byte slice. Values larger than 2^32-1 are saturated.
pub(crate) fn encode_coap_uint(value: usize) -> Vec<u8> {
    (value.min(u32::MAX as usize) as u32)
        .to_be_bytes()
        .iter()
        .skip_while(|&&b| b == 0)
        .copied()
        .collect()
}

impl Observer {
    /// Creates an observer with channel to send message.
    pub fn new() -> Self {
        Self {
            registers: HashMap::new(),
            resources: HashMap::new(),
            register_resources: HashMap::new(),
            unacknowledge_messages: HashMap::new(),
            current_message_id: 0,
            timer: IntervalStream::new(interval(Duration::from_secs(1))).fuse(),
        }
    }

    /// poll the observer's timer.
    pub fn select_next_some(&mut self) -> SelectNextSome<'_, Fuse<IntervalStream>> {
        self.timer.select_next_some()
    }

    /// Checks if the given client endpoint is currently observing the specified resource.
    pub fn is_observing(&self, addr: &SocketAddr, path: &String) -> bool {
        let key = Self::format_register_resource(addr, path);
        self.register_resources.contains_key(&key)
    }

    /// Returns a shared reference to the resource payload and its current ETag.
    /// Used by the server to serve remaining blocks consistently during a block-wise transfer.
    pub fn get_resource_payload_and_etag(&self, path: &String) -> Option<(Arc<Vec<u8>>, Vec<u8>)> {
        self.resources
            .get(path)
            .map(|r| (r.payload.clone(), r.etag.clone()))
    }

    /// Computes a non-cryptographic ETag using FNV-1a hash for change detection only.
    /// This ETag is not suitable for security purposes (e.g., integrity validation against attackers).
    fn compute_etag(payload: &[u8]) -> Vec<u8> {
        let mut hash: u64 = 0xcbf29ce484222325;
        for &byte in payload {
            hash ^= byte as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        }
        hash.to_be_bytes()[..4].to_vec()
    }

    /// filter the requests belong to the observer. store the responder in case it is needed
    /// returns whether the request should be forwarded to the handler
    pub async fn request_handler(
        &mut self,
        request: &mut CoapRequest<SocketAddr>,
        responder: Arc<dyn Responder>,
    ) -> bool {
        match request.message.header.get_type() {
            MessageType::Acknowledgement => {
                self.acknowledge(request);
                return false;
            }
            MessageType::Reset => {
                self.reset_notification(request);
                return false;
            }
            _ => {}
        }

        match (request.get_method(), request.get_observe_flag()) {
            (&Method::Get, Some(observe_option)) => match observe_option {
                Ok(ObserveOption::Register) => {
                    self.register(request, responder).await;
                    return false;
                }
                Ok(ObserveOption::Deregister) => {
                    self.deregister(request);
                    return true;
                }
                _ => return true,
            },
            (&Method::Put, _) => {
                self.resource_changed(request).await;
                return true;
            }
            _ => return true,
        }
    }

    /// trigger send the unacknowledge messages.
    pub async fn timer_handler(&mut self) {
        let register_resource_keys: Vec<String>;
        {
            register_resource_keys = self
                .unacknowledge_messages
                .iter()
                .map(|(_, msg)| msg.register_resource.clone())
                .collect();
        }

        for register_resource_key in register_resource_keys {
            if self.try_unacknowledge_message(&register_resource_key) {
                self.notify_register_with_newest_resource(&register_resource_key)
                    .await;
            }
        }
    }

    async fn register(
        &mut self,
        request: &mut CoapRequest<SocketAddr>,
        responder: Arc<dyn Responder>,
    ) {
        let register_address = responder.address();
        let resource_path = request.get_path();

        debug!("register {} {}", register_address, resource_path);

        // reply NotFound if resource doesn't exist
        if !self.resources.contains_key(&resource_path) {
            if let Some(ref response) = request.response.take() {
                let mut response2 = response.clone();
                response2.set_status(Status::NotFound);
                let msg_serial = response2.message.to_bytes();
                if let Ok(b) = msg_serial {
                    responder.respond(b).await;
                }
            }
            return;
        }

        let preferred_block_size = request
            .message
            .get_first_option_as::<BlockValue>(CoapOption::Block2)
            .and_then(|x: Result<BlockValue, _>| x.ok())
            .map(|b: BlockValue| b.size());

        self.record_register_resource(
            responder.clone(),
            &resource_path,
            &request.message.get_token(),
            preferred_block_size,
        );

        let resource = self.resources.get(&resource_path).unwrap();

        if let Some(response) = request.response.take() {
            let mut response2 = response.clone();
            response2.message.set_observe_value(resource.sequence);
            response2
                .message
                .header
                .set_type(MessageType::NonConfirmable);
            response2
                .message
                .add_option(CoapOption::ETag, resource.etag.clone());

            let total_size = resource.payload.len();
            response2
                .message
                .add_option(CoapOption::Size2, encode_coap_uint(total_size));

            if let Some(block_size) = preferred_block_size {
                if resource.payload.len() > block_size {
                    let block = BlockValue::new(0, true, block_size).expect("valid block size");
                    response2
                        .message
                        .add_option_as::<BlockValue>(CoapOption::Block2, block);
                    response2.message.payload = resource.payload[..block_size].to_vec();
                } else {
                    response2.message.payload = resource.payload.to_vec();
                }
            } else {
                response2.message.payload = resource.payload.to_vec();
            }

            if let Ok(b) = response2.message.to_bytes() {
                responder.respond(b).await;
            }
        }
    }

    fn deregister(&mut self, request: &CoapRequest<SocketAddr>) {
        let register_address = request.source.unwrap();
        let resource_path = request.get_path();

        debug!("deregister {} {}", register_address, resource_path);

        self.remove_register_resource(
            &register_address,
            &resource_path,
            &request.message.get_token(),
        );
    }

    /// handle reset message from client.
    /// according to rfc 7641 section 4.5, if a client rejects a notification with a reset message,
    /// the server must remove the associated entry from the list of observers.
    fn reset_notification(&mut self, request: &CoapRequest<SocketAddr>) {
        let message_id = request.message.header.message_id;

        // Validate Message ID
        let Some(unack_item) = self.unacknowledge_messages.get(&message_id) else {
            debug!("Reset received for unknown Message ID: {}", message_id);
            return;
        };

        // Verify source endpoint
        let key = &unack_item.register_resource;
        let Some(reg_item) = self.register_resources.get(key) else {
            debug!("Reset received for unknown registration key: {}", key);
            return;
        };

        let expected_address = reg_item.registered_responder.address();
        if request.source != Some(expected_address) {
            warn!(
                "Received RST for MID {} from unexpected source {:?}, expected {}. Ignoring.",
                message_id, request.source, expected_address
            );
            return;
        }

        // Extract necessary information for cleanup
        // We clone data here to release the immutable borrows on self before mutation.
        let address = expected_address;
        let resource_path = reg_item.resource.clone();
        let token = reg_item.token.clone();
        let register_resource_key = key.clone();

        // Remove the mapping between MID and resource
        self.unacknowledge_messages.remove(&message_id);

        // Clear the pending message reference in the registration to avoid double-deletion
        if let Some(item) = self.register_resources.get_mut(&register_resource_key) {
            item.unacknowledge_message = None;
        }

        debug!(
            "Reset received from {} for resource {}, removing observer",
            address, resource_path
        );

        // Remove the observer from the registry
        self.remove_register_resource(&address, &resource_path, &token);
    }

    async fn resource_changed(&mut self, request: &CoapRequest<SocketAddr>) {
        let resource_path = request.get_path();
        let ref resource_payload = request.message.payload;

        debug!("resource_changed {} {:?}", resource_path, resource_payload);

        let register_resource_keys: Vec<String>;
        {
            let resource = self.record_resource(&resource_path, &resource_payload);
            register_resource_keys = resource
                .register_resources
                .iter()
                .map(|k| k.clone())
                .collect();
        }

        for register_resource_key in register_resource_keys {
            self.gen_message_id();
            self.notify_register_with_newest_resource(&register_resource_key)
                .await;
            self.record_unacknowledge_message(&register_resource_key);
        }
    }

    fn acknowledge(&mut self, request: &CoapRequest<SocketAddr>) {
        self.remove_unacknowledge_message(
            &request.message.header.message_id,
            &request.message.get_token(),
        );
    }

    fn record_register_resource(
        &mut self,
        responder: Arc<dyn Responder>,
        path: &String,
        token: &[u8],
        preferred_block_size: Option<usize>,
    ) {
        let resource = self.resources.get_mut(path).unwrap();
        let register_key = responder;

        let register_resource_key = Self::format_register_resource(&register_key.address(), path);

        self.register_resources
            .entry(register_resource_key.clone())
            .and_modify(|existing| {
                // Clear any pending unacknowledged message to prevent stale timeout logic
                if let Some(old_msg_id) = existing.unacknowledge_message.take() {
                    self.unacknowledge_messages.remove(&old_msg_id);
                }

                // Refresh the observation state with new parameters
                existing.registered_responder = register_key.clone();
                existing.token = token.into();
                existing.preferred_block_size = preferred_block_size;
            })
            .or_insert_with(|| RegisterResourceItem {
                registered_responder: register_key.clone(),
                resource: path.clone(),
                token: token.into(),
                unacknowledge_message: None,
                preferred_block_size,
            });

        resource
            .register_resources
            .replace(register_resource_key.clone());
        match self.registers.entry(register_key.address().to_string()) {
            Entry::Occupied(register) => {
                register
                    .into_mut()
                    .register_resources
                    .replace(register_resource_key);
            }
            Entry::Vacant(v) => {
                let mut register = RegisterItem {
                    register_resources: HashSet::new(),
                };
                register.register_resources.insert(register_resource_key);

                v.insert(register);
            }
        };
    }

    fn remove_register_resource(
        &mut self,
        address: &SocketAddr,
        path: &String,
        token: &[u8],
    ) -> bool {
        let register_resource_key = Self::format_register_resource(&address, path);

        if let Some(register_resource) = self.register_resources.get(&register_resource_key) {
            if register_resource.token != *token {
                return false;
            }

            if let Some(unacknowledge_message) = register_resource.unacknowledge_message {
                self.unacknowledge_messages
                    .remove(&unacknowledge_message)
                    .unwrap();
            }

            assert_eq!(
                self.resources
                    .get_mut(path)
                    .unwrap()
                    .register_resources
                    .remove(&register_resource_key),
                true
            );

            let remove_register;
            {
                let register = self
                    .registers
                    .get_mut(&register_resource.registered_responder.address().to_string())
                    .unwrap();
                assert_eq!(
                    register.register_resources.remove(&register_resource_key),
                    true
                );
                remove_register = register.register_resources.len() == 0;
            }

            if remove_register {
                self.registers
                    .remove(&register_resource.registered_responder.address().to_string());
            }
        }

        self.register_resources.remove(&register_resource_key);
        return true;
    }

    fn record_resource(&mut self, path: &String, payload: &Vec<u8>) -> &ResourceItem {
        match self.resources.entry(path.clone()) {
            Entry::Occupied(resource) => {
                let r = resource.into_mut();
                r.sequence += 1;
                r.payload = Arc::new(payload.clone());
                r.etag = Self::compute_etag(payload);
                r
            }
            Entry::Vacant(v) => v.insert(ResourceItem {
                payload: Arc::new(payload.clone()),
                register_resources: HashSet::new(),
                sequence: 0,
                etag: Self::compute_etag(payload),
            }),
        }
    }

    fn record_unacknowledge_message(&mut self, register_resource_key: &String) {
        let message_id = self.current_message_id;

        let register_resource = self
            .register_resources
            .get_mut(register_resource_key)
            .unwrap();
        if let Some(old_message_id) = register_resource.unacknowledge_message {
            self.unacknowledge_messages.remove(&old_message_id);
        }

        register_resource.unacknowledge_message = Some(message_id);
        self.unacknowledge_messages.insert(
            message_id,
            UnacknowledgeMessageItem {
                register_resource: register_resource_key.clone(),
                try_times: 1,
            },
        );
    }

    fn try_unacknowledge_message(&mut self, register_resource_key: &String) -> bool {
        let register_resource = self
            .register_resources
            .get_mut(register_resource_key)
            .unwrap();
        let ref message_id = register_resource.unacknowledge_message.unwrap();

        let try_again;
        {
            let unacknowledge_message = self.unacknowledge_messages.get_mut(message_id).unwrap();
            if unacknowledge_message.try_times > DEFAULT_UNACKNOWLEDGE_MESSAGE_TRY_TIMES {
                try_again = false;
            } else {
                unacknowledge_message.try_times += 1;
                try_again = true;
            }
        }

        if !try_again {
            warn!(
                "unacknowledge_message try times exceeded  {}",
                register_resource_key
            );

            register_resource.unacknowledge_message = None;
            self.unacknowledge_messages.remove(message_id);
        }

        return try_again;
    }

    fn remove_unacknowledge_message(&mut self, message_id: &u16, token: &[u8]) {
        if let Some(message) = self.unacknowledge_messages.get_mut(message_id) {
            let register_resource = self
                .register_resources
                .get_mut(&message.register_resource)
                .unwrap();
            if register_resource.token != *token {
                return;
            }

            register_resource.unacknowledge_message = None;
        }

        self.unacknowledge_messages.remove(message_id);
    }

    /// Notifies a specific registered observer about a resource change.
    ///
    /// Note on Architecture: The payload truncation and Block2 option injection for the
    /// first block are handled directly within `Observer` rather than the generic
    /// `intercept_response` in `server.rs`. This is a deliberate design choice to maintain
    /// single-responsibility in a push-based context: asynchronous notifications bypass the
    /// main server request/response loop and are sent directly via the `Responder`.
    /// Centralizing the "first block generation" logic here avoids duplicating the truncation
    /// code for both synchronous registrations and asynchronous notifications.
    async fn notify_register_with_newest_resource(&mut self, register_resource_key: &String) {
        let message_id = self.current_message_id;

        debug!("notify {} {}", register_resource_key, message_id);

        let ref mut message = Packet::new();
        message.header.set_type(MessageType::Confirmable);
        message.header.code = MessageClass::Response(Status::Content);

        let register_resource = self.register_resources.get(register_resource_key).unwrap();
        let resource = self.resources.get(&register_resource.resource).unwrap();

        message.set_token(register_resource.token.clone());
        message.set_observe_value(resource.sequence);
        message.header.message_id = message_id;
        message.add_option(CoapOption::ETag, resource.etag.clone());

        let total_size = resource.payload.len();
        message.add_option(CoapOption::Size2, encode_coap_uint(total_size));

        if let Some(block_size) = register_resource.preferred_block_size {
            if resource.payload.len() > block_size {
                let block = BlockValue::new(0, true, block_size).expect("valid block size");
                message.add_option_as::<BlockValue>(CoapOption::Block2, block);
                message.payload = resource.payload[..block_size].to_vec();
            } else {
                message.payload = resource.payload.to_vec();
            }
        } else {
            message.payload = resource.payload.to_vec();
        }

        if let Ok(b) = message.to_bytes() {
            debug!("notify register with newest resource {:?}", &b);
            register_resource.registered_responder.respond(b).await;
        }
    }

    fn gen_message_id(&mut self) -> u16 {
        self.current_message_id += 1;
        return self.current_message_id;
    }

    fn format_register_resource(address: &SocketAddr, path: &String) -> String {
        format!("{}${}", address, path)
    }
}

#[cfg(test)]
mod test {

    use crate::request::RequestBuilder;

    use super::super::*;
    use super::*;
    use async_trait::async_trait;
    use coap_lite::CoapResponse;
    use std::io::ErrorKind;
    use tokio::sync::mpsc;

    async fn request_handler(
        mut req: Box<CoapRequest<SocketAddr>>,
    ) -> Box<CoapRequest<SocketAddr>> {
        match req.get_method() {
            &coap_lite::RequestType::Get => {
                let observe_option = req.get_observe_flag().unwrap().unwrap();
                assert_eq!(observe_option, ObserveOption::Deregister);
            }
            &coap_lite::RequestType::Put => {}
            _ => panic!("unexpected request"),
        }

        match req.response {
            Some(ref mut response) => {
                response.message.payload = b"OK".to_vec();
            }
            _ => {}
        };
        return req;
    }

    fn decode_uint(data: &[u8]) -> usize {
        data.iter().fold(0, |acc, &b| (acc << 8) | b as usize)
    }

    #[tokio::test]
    async fn test_observe() {
        let path = "/test";
        let payload1 = b"data1".to_vec();
        let payload2 = b"data2".to_vec();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let (tx2, mut rx2) = mpsc::unbounded_channel();
        let mut step = 1;

        let server_port = server::test::spawn_server("127.0.0.1:0", request_handler)
            .recv()
            .await
            .unwrap();

        let server_address = &format!("127.0.0.1:{}", server_port);

        let client = UdpCoAPClient::new(server_address).await.unwrap();

        tx.send(step).unwrap();
        let mut request = CoapRequest::new();

        request.set_method(coap_lite::RequestType::Put);
        request.set_path(path);
        request.message.set_token(vec![1]);

        request.message.payload = payload1.clone();
        let _ = client.send(request.clone()).await.unwrap();

        let payload1_clone = payload1.clone();
        let payload2_clone = payload2.clone();

        let client2 = client.clone();

        let mut receive_step = 1;
        client
            .observe(path, move |msg| {
                match rx.try_recv() {
                    Ok(n) => receive_step = n,
                    _ => debug!("receive_step rx error"),
                }
                debug!("receive on client: {:?}", &msg);

                match receive_step {
                    1 => assert_eq!(msg.payload, payload1_clone),
                    2 => {
                        assert_eq!(msg.payload, payload2_clone);
                        tx2.send(()).unwrap();
                    }
                    _ => panic!("unexpected step"),
                }
            })
            .await
            .unwrap();

        step = 2;
        debug!("on step 2");
        tx.send(step).unwrap();

        request.message.payload = payload2.clone();
        request.message.set_token(vec![2]);

        let _ = client2.send(request).await.unwrap();
        assert_eq!(
            tokio::time::timeout(Duration::new(5, 0), rx2.recv())
                .await
                .unwrap(),
            Some(())
        );
    }
    #[tokio::test]
    async fn test_unobserve() {
        let path = "/test";
        let payload1 = b"data1".to_vec();
        let payload2 = b"data2".to_vec();

        let server_port = server::test::spawn_server("127.0.0.1:0", request_handler)
            .recv()
            .await
            .unwrap();

        let server_address = &format!("127.0.0.1:{}", server_port);

        let client = UdpCoAPClient::new(server_address).await.unwrap();

        let client3 = client.clone();

        let mut request = RequestBuilder::new(path, coap_lite::RequestType::Put)
            .token(Some(vec![1]))
            .data(Some(payload1.clone()))
            .build();
        let _ = client.send(request.clone()).await.unwrap();

        let payload1_clone = payload1.clone();
        let unobserve = client
            .observe(path, move |msg| {
                assert_eq!(msg.payload, payload1_clone);
            })
            .await
            .unwrap();

        unobserve.send(client::ObserveMessage::Terminate).unwrap();
        request.message.payload = payload2.clone();

        let _ = client3.send(request).await.unwrap();
    }

    #[tokio::test]
    async fn test_observe_without_resource() {
        let path = "/test";

        let server_port = server::test::spawn_server("127.0.0.1:0", request_handler)
            .recv()
            .await
            .unwrap();

        let client = UdpCoAPClient::new(format!("127.0.0.1:{}", server_port))
            .await
            .unwrap();
        let error = client.observe(path, |_msg| {}).await.unwrap_err();
        assert_eq!(error.kind(), ErrorKind::NotFound);
    }

    #[tokio::test]
    async fn test_observe_cancelled_by_rst() {
        use async_trait::async_trait;
        use tokio::sync::mpsc::{unbounded_channel, UnboundedSender};

        let mut observer = Observer::new();
        let path = "test";
        let addr: SocketAddr = "127.0.0.1:5683".parse().unwrap();

        // Mock Responder to intercept outgoing packets (notifications)
        struct MockResponder {
            addr: SocketAddr,
            tx: UnboundedSender<Vec<u8>>,
        }

        #[async_trait]
        impl Responder for MockResponder {
            async fn respond(&self, bytes: Vec<u8>) {
                self.tx.send(bytes).unwrap();
            }
            fn address(&self) -> SocketAddr {
                self.addr
            }
        }

        // Setup: Create a resource and a channel to capture notifications
        observer.resources.insert(
            path.to_string(),
            ResourceItem {
                payload: Arc::new(vec![]),
                register_resources: HashSet::new(),
                sequence: 0,
                etag: vec![],
            },
        );

        let (tx, mut rx) = unbounded_channel();
        let responder = Arc::new(MockResponder { addr, tx });

        // Register the observer
        let mut register_req = CoapRequest::<SocketAddr>::new();
        register_req.set_path(path);
        register_req.set_method(Method::Get);
        register_req.message.set_token(vec![1, 2, 3, 4]);
        register_req.set_observe_flag(ObserveOption::Register);
        register_req.source = Some(addr);

        observer
            .request_handler(&mut register_req, responder.clone())
            .await;

        let key = format!("{}${}", addr, path);
        assert!(observer.register_resources.contains_key(&key));

        // Trigger a resource change (PUT) to cause a notification
        let mut put_req = CoapRequest::<SocketAddr>::new();
        put_req.set_path(path);
        put_req.set_method(Method::Put);
        put_req.message.payload = b"data1".to_vec();
        put_req.source = Some(addr);

        observer
            .request_handler(&mut put_req, responder.clone())
            .await;

        // Intercept the notification to get its Message ID
        let notification_bytes = rx.recv().await.unwrap();
        let notification_pkt = Packet::from_bytes(&notification_bytes).unwrap();
        assert_eq!(notification_pkt.header.get_type(), MessageType::Confirmable);
        let mid = notification_pkt.header.message_id;

        // Simulate the client rejecting the notification with a Reset message
        let mut rst_req = CoapRequest::<SocketAddr>::new();
        rst_req.message.header.set_type(MessageType::Reset);
        rst_req.message.header.set_code("0.00");
        rst_req.message.header.message_id = mid;
        rst_req.source = Some(addr);

        observer
            .request_handler(&mut rst_req, responder.clone())
            .await;

        // Verify that the observer has been removed from the list
        assert!(!observer.register_resources.contains_key(&key));

        // Trigger another resource change and ensure no new notification is sent
        let mut put_req2 = CoapRequest::<SocketAddr>::new();
        put_req2.set_path(path);
        put_req2.set_method(Method::Put);
        put_req2.message.payload = b"data2".to_vec();
        put_req2.source = Some(addr);

        observer
            .request_handler(&mut put_req2, responder.clone())
            .await;

        // Use timeout to verify the channel is empty (no notification sent)
        let result = tokio::time::timeout(Duration::from_millis(100), rx.recv()).await;
        assert!(
            result.is_err(),
            "Expected no notification after RST cancellation"
        );
    }

    use tokio::sync::Mutex;

    struct MockResponder {
        addr: SocketAddr,
        last_sent: Arc<Mutex<Option<Vec<u8>>>>,
    }

    impl MockResponder {
        fn new(addr: SocketAddr) -> Self {
            Self {
                addr,
                last_sent: Arc::new(Mutex::new(None)),
            }
        }

        async fn get_last_sent(&self) -> Option<Vec<u8>> {
            self.last_sent.lock().await.clone()
        }
    }

    #[async_trait]
    impl Responder for MockResponder {
        async fn respond(&self, bytes: Vec<u8>) {
            *self.last_sent.lock().await = Some(bytes);
        }
        fn address(&self) -> SocketAddr {
            self.addr
        }
    }

    #[tokio::test]
    async fn test_observer_block_size_boundaries_and_preferences() {
        let mut observer = Observer::new();
        let path = "test";
        let addr1: SocketAddr = "127.0.0.1:5001".parse().unwrap();
        let addr2: SocketAddr = "127.0.0.1:5002".parse().unwrap();

        // 1. Prepare a resource exactly equal to block_size (1024 bytes)
        let payload_exact = vec![0xAA; 1024];
        let mut put_req = CoapRequest::<SocketAddr>::new();
        put_req.set_method(Method::Put);
        put_req.set_path(path);
        put_req.message.payload = payload_exact.clone();
        put_req.source = Some(addr1);
        observer
            .request_handler(&mut put_req, Arc::new(MockResponder::new(addr1)))
            .await;

        // 2. Client 1 registers, expecting block size 1024
        let mut reg_req1 = CoapRequest::<SocketAddr>::new();
        reg_req1.set_method(Method::Get);
        reg_req1.set_path(path);
        reg_req1.set_observe_flag(ObserveOption::Register);
        reg_req1.message.add_option_as::<BlockValue>(
            CoapOption::Block2,
            BlockValue::new(0, false, 1024).unwrap(),
        );
        reg_req1.source = Some(addr1);
        reg_req1.response = Some(CoapResponse {
            message: Packet::new(),
        });

        let responder1 = Arc::new(MockResponder::new(addr1));
        observer
            .request_handler(&mut reg_req1, responder1.clone())
            .await;

        let resp1_bytes = responder1.get_last_sent().await.unwrap();
        let resp1_pkt = Packet::from_bytes(&resp1_bytes).unwrap();

        assert_eq!(resp1_pkt.payload.len(), 1024);
        assert!(
            resp1_pkt.get_option(CoapOption::Block2).is_none(),
            "Should not add Block2 if exactly equal"
        );

        // Size2 assertion for Client 1 registration
        let size2_opt = resp1_pkt.get_first_option(CoapOption::Size2);
        assert!(
            size2_opt.is_some(),
            "Size2 missing in register response (no block2)"
        );
        let size2_val = decode_uint(size2_opt.unwrap());
        assert_eq!(size2_val, 1024, "Size2 value mismatch");

        // 3. Prepare larger resource (1500 bytes)
        let payload_large = vec![0xBB; 1500];
        let mut put_req2 = CoapRequest::<SocketAddr>::new();
        put_req2.set_method(Method::Put);
        put_req2.set_path(path);
        put_req2.message.payload = payload_large.clone();
        put_req2.source = Some(addr1);
        observer
            .request_handler(&mut put_req2, Arc::new(MockResponder::new(addr1)))
            .await;

        // 4. Client 2 registers, expecting block size 512
        let mut reg_req2 = CoapRequest::<SocketAddr>::new();
        reg_req2.set_method(Method::Get);
        reg_req2.set_path(path);
        reg_req2.set_observe_flag(ObserveOption::Register);
        reg_req2.message.add_option_as::<BlockValue>(
            CoapOption::Block2,
            BlockValue::new(0, false, 512).unwrap(),
        );
        reg_req2.message.set_token(vec![2]);
        reg_req2.source = Some(addr2);
        reg_req2.response = Some(CoapResponse {
            message: Packet::new(),
        });

        let responder2 = Arc::new(MockResponder::new(addr2));
        observer
            .request_handler(&mut reg_req2, responder2.clone())
            .await;

        let resp2_bytes = responder2.get_last_sent().await.unwrap();
        let resp2_pkt = Packet::from_bytes(&resp2_bytes).unwrap();

        assert_eq!(resp2_pkt.payload.len(), 512);
        let block2_opt = resp2_pkt
            .get_first_option_as::<BlockValue>(CoapOption::Block2)
            .unwrap()
            .unwrap();
        assert_eq!(block2_opt.size(), 512);
        assert!(block2_opt.more, "Should have more blocks");

        // Size2 assertion for Client 2 registration
        let size2_opt2 = resp2_pkt.get_first_option(CoapOption::Size2);
        assert!(
            size2_opt2.is_some(),
            "Size2 missing in register response with block2"
        );
        let size2_val2 = decode_uint(size2_opt2.unwrap());
        assert_eq!(
            size2_val2, 1500,
            "Size2 value mismatch in block2 registration"
        );

        // 5. Trigger a resource change (PUT) to cause notification to Client 1
        let mut put_req3 = CoapRequest::<SocketAddr>::new();
        put_req3.set_method(Method::Put);
        put_req3.set_path(path);
        put_req3.message.payload = vec![0xCC; 1500];
        put_req3.source = Some(addr1);
        observer
            .request_handler(&mut put_req3, responder1.clone())
            .await;

        let resp1_notify_bytes = responder1.get_last_sent().await.unwrap();
        let resp1_notify_pkt = Packet::from_bytes(&resp1_notify_bytes).unwrap();

        assert_eq!(resp1_notify_pkt.payload.len(), 1024);
        let notify_block2 = resp1_notify_pkt
            .get_first_option_as::<BlockValue>(CoapOption::Block2)
            .unwrap()
            .unwrap();
        assert_eq!(notify_block2.size(), 1024);

        // Size2 assertion for notification
        let size2_opt_notify = resp1_notify_pkt.get_first_option(CoapOption::Size2);
        assert!(size2_opt_notify.is_some(), "Size2 missing in notification");
        let size2_val_notify = decode_uint(size2_opt_notify.unwrap());
        assert_eq!(
            size2_val_notify, 1500,
            "Size2 value mismatch in notification"
        );
    }

    #[test]
    fn test_encode_coap_uint() {
        assert_eq!(encode_coap_uint(0), vec![] as Vec<u8>);
        assert_eq!(encode_coap_uint(1), vec![0x01]);
        assert_eq!(encode_coap_uint(255), vec![0xFF]);
        assert_eq!(encode_coap_uint(256), vec![0x01, 0x00]);
        assert_eq!(encode_coap_uint(0xFFFFFFFF), vec![0xFF, 0xFF, 0xFF, 0xFF]);
        #[cfg(target_pointer_width = "64")]
        {
            assert_eq!(encode_coap_uint(0x100000000), vec![0xFF, 0xFF, 0xFF, 0xFF]);
        }
    }

    #[tokio::test]
    async fn test_observe_empty_resource_includes_size2() {
        let mut observer = Observer::new();
        let path = "empty";
        let addr: SocketAddr = "127.0.0.1:5003".parse().unwrap();

        // Create an empty resource via PUT
        let empty_payload = vec![];
        let mut put_req = CoapRequest::<SocketAddr>::new();
        put_req.set_method(Method::Put);
        put_req.set_path(path);
        put_req.message.payload = empty_payload.clone();
        put_req.source = Some(addr);
        observer
            .request_handler(&mut put_req, Arc::new(MockResponder::new(addr)))
            .await;

        // Register to observe the empty resource
        let mut reg_req = CoapRequest::<SocketAddr>::new();
        reg_req.set_method(Method::Get);
        reg_req.set_path(path);
        reg_req.set_observe_flag(ObserveOption::Register);
        reg_req.source = Some(addr);
        reg_req.response = Some(CoapResponse {
            message: Packet::new(),
        });
        let responder = Arc::new(MockResponder::new(addr));
        observer
            .request_handler(&mut reg_req, responder.clone())
            .await;

        let resp_bytes = responder.get_last_sent().await.unwrap();
        let resp_pkt = Packet::from_bytes(&resp_bytes).unwrap();

        // Verify Size2 option is present and value is 0
        let size2_opt = resp_pkt.get_first_option(CoapOption::Size2);
        assert!(
            size2_opt.is_some(),
            "Size2 option missing for empty resource"
        );
        let size2_val = decode_uint(size2_opt.unwrap());
        assert_eq!(size2_val, 0, "Size2 value should be 0 for empty payload");
    }

    #[tokio::test]
    async fn test_observe_reregistration_updates_token_and_block_size() {
        let mut observer = Observer::new();
        let path = "test_update";
        let addr: SocketAddr = "127.0.0.1:6001".parse().unwrap();

        // Create a resource larger than block sizes to trigger block-wise transfer
        let payload = vec![0xAA; 1500];
        let mut put_req = CoapRequest::<SocketAddr>::new();
        put_req.set_method(Method::Put);
        put_req.set_path(path);
        put_req.message.payload = payload.clone();
        put_req.source = Some(addr);
        observer
            .request_handler(&mut put_req, Arc::new(MockResponder::new(addr)))
            .await;

        // Initial registration with Token A and Block2 preference 1024
        let mut reg_req1 = CoapRequest::<SocketAddr>::new();
        reg_req1.set_method(Method::Get);
        reg_req1.set_path(path);
        reg_req1.set_observe_flag(ObserveOption::Register);
        reg_req1.message.set_token(vec![1]); // Token A
        reg_req1.message.add_option_as::<BlockValue>(
            CoapOption::Block2,
            BlockValue::new(0, false, 1024).unwrap(),
        );
        reg_req1.source = Some(addr);
        reg_req1.response = Some(CoapResponse {
            message: Packet::new(),
        });

        let responder1 = Arc::new(MockResponder::new(addr));
        observer
            .request_handler(&mut reg_req1, responder1.clone())
            .await;

        // Re-registration from the same client with Token B and Block2 preference 512
        let mut reg_req2 = CoapRequest::<SocketAddr>::new();
        reg_req2.set_method(Method::Get);
        reg_req2.set_path(path);
        reg_req2.set_observe_flag(ObserveOption::Register);
        reg_req2.message.set_token(vec![2]); // Token B
        reg_req2.message.add_option_as::<BlockValue>(
            CoapOption::Block2,
            BlockValue::new(0, false, 512).unwrap(), // New preference
        );
        reg_req2.source = Some(addr);
        reg_req2.response = Some(CoapResponse {
            message: Packet::new(),
        });

        let responder2 = Arc::new(MockResponder::new(addr));
        observer
            .request_handler(&mut reg_req2, responder2.clone())
            .await;

        // Trigger a resource change to force a notification
        let mut put_req2 = CoapRequest::<SocketAddr>::new();
        put_req2.set_method(Method::Put);
        put_req2.set_path(path);
        put_req2.message.payload = vec![0xBB; 1500];
        put_req2.source = Some(addr);
        observer
            .request_handler(&mut put_req2, responder2.clone())
            .await;

        // Fetch the notification sent by the observer
        let resp_bytes = responder2.get_last_sent().await.unwrap();
        let resp_pkt = Packet::from_bytes(&resp_bytes).unwrap();

        // Verify the server used the updated state (Token B and Block size 512)
        assert_eq!(
            resp_pkt.get_token(),
            vec![2],
            "Server should use the updated token from re-registration"
        );
        assert_eq!(
            resp_pkt.payload.len(),
            512,
            "Server should use the updated block size preference from re-registration"
        );

        let block2_opt = resp_pkt
            .get_first_option_as::<BlockValue>(CoapOption::Block2)
            .unwrap()
            .unwrap();
        assert_eq!(block2_opt.size(), 512);
    }
}
