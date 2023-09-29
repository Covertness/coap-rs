use coap_lite::{
    CoapRequest, MessageClass, MessageType, ObserveOption, Packet, RequestType as Method,
    ResponseType as Status,
};
use futures::{
    stream::{Fuse, SelectNextSome},
    StreamExt,
};
use log::{debug, warn};
use std::{
    collections::{hash_map::Entry, HashMap, HashSet},
    net::SocketAddr,
    time::Duration,
};
use tokio::time::interval;
use tokio_stream::wrappers::IntervalStream;

use super::server::MessageSender;

const DEFAULT_UNACKNOWLEDGE_MESSAGE_TRY_TIMES: usize = 10;

pub struct Observer {
    registers: HashMap<String, RegisterItem>,
    resources: HashMap<String, ResourceItem>,
    register_resources: HashMap<String, RegisterResourceItem>,
    unacknowledge_messages: HashMap<u16, UnacknowledgeMessageItem>,
    tx_sender: MessageSender,
    current_message_id: u16,
    timer: Fuse<IntervalStream>,
}

#[derive(Debug)]
struct RegisterItem {
    register_resources: HashSet<String>,
}

#[derive(Debug)]
struct ResourceItem {
    payload: Vec<u8>,
    register_resources: HashSet<String>,
    sequence: u32,
}

#[derive(Debug)]
struct RegisterResourceItem {
    register: String,
    resource: String,
    token: Vec<u8>,
    unacknowledge_message: Option<u16>,
}

#[derive(Debug)]
struct UnacknowledgeMessageItem {
    register_resource: String,
    try_times: usize,
}

impl Observer {
    /// Creates an observer with channel to send message.
    pub fn new(tx_sender: MessageSender) -> Observer {
        Observer {
            registers: HashMap::new(),
            resources: HashMap::new(),
            register_resources: HashMap::new(),
            unacknowledge_messages: HashMap::new(),
            tx_sender: tx_sender,
            current_message_id: 0,
            timer: IntervalStream::new(interval(Duration::from_secs(1))).fuse(),
        }
    }

    /// poll the observer's timer.
    pub fn select_next_some(&mut self) -> SelectNextSome<Fuse<IntervalStream>> {
        self.timer.select_next_some()
    }

    /// filter the requests belong to the observer.
    pub async fn request_handler(&mut self, request: &CoapRequest<SocketAddr>) -> bool {
        if request.message.header.get_type() == MessageType::Acknowledgement {
            self.acknowledge(request);
            return false;
        }

        match (request.get_method(), request.get_observe_flag()) {
            (&Method::Get, Some(observe_option)) => match observe_option {
                Ok(ObserveOption::Register) => {
                    self.register(request).await;
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

    async fn register(&mut self, request: &CoapRequest<SocketAddr>) {
        let register_address = request.source.unwrap();
        let resource_path = request.get_path();

        debug!("register {} {}", register_address, resource_path);

        // reply NotFound if resource doesn't exist
        if !self.resources.contains_key(&resource_path) {
            if let Some(ref response) = request.response {
                let mut response2 = response.clone();
                response2.set_status(Status::NotFound);
                self.send_message(&register_address, &response2.message)
                    .await;
            }
            return;
        }

        self.record_register_resource(
            &register_address,
            &resource_path,
            &request.message.get_token(),
        );

        let resource = self.resources.get(&resource_path).unwrap();

        if let Some(ref response) = request.response {
            let mut response2 = response.clone();
            response2.message.payload = resource.payload.clone();
            response2.message.set_observe_value(resource.sequence);
            response2
                .message
                .header
                .set_type(MessageType::NonConfirmable);
            self.send_message(&register_address, &response2.message)
                .await;
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

    fn record_register_resource(&mut self, address: &SocketAddr, path: &String, token: &[u8]) {
        let resource = self.resources.get_mut(path).unwrap();
        let register_key = Self::format_register(&address);
        let register_resource_key = Self::format_register_resource(&address, path);

        self.register_resources
            .entry(register_resource_key.clone())
            .or_insert(RegisterResourceItem {
                register: register_key.clone(),
                resource: path.clone(),
                token: token.into(),
                unacknowledge_message: None,
            });
        resource
            .register_resources
            .replace(register_resource_key.clone());
        match self.registers.entry(register_key) {
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
                let register = self.registers.get_mut(&register_resource.register).unwrap();
                assert_eq!(
                    register.register_resources.remove(&register_resource_key),
                    true
                );
                remove_register = register.register_resources.len() == 0;
            }

            if remove_register {
                self.registers.remove(&register_resource.register);
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
                r.payload = payload.clone();
                return r;
            }
            Entry::Vacant(v) => {
                return v.insert(ResourceItem {
                    payload: payload.clone(),
                    register_resources: HashSet::new(),
                    sequence: 0,
                });
            }
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

    async fn notify_register_with_newest_resource(&mut self, register_resource_key: &String) {
        let message_id = self.current_message_id;

        debug!("notify {} {}", register_resource_key, message_id);

        let ref mut message = Packet::new();
        message.header.set_type(MessageType::Confirmable);
        message.header.code = MessageClass::Response(Status::Content);

        let address: SocketAddr;
        {
            let register_resource = self.register_resources.get(register_resource_key).unwrap();
            let resource = self.resources.get(&register_resource.resource).unwrap();

            message.set_token(register_resource.token.clone());
            message.set_observe_value(resource.sequence);
            message.header.message_id = message_id;
            message.payload = resource.payload.clone();

            address = register_resource.register.parse().unwrap();
        }

        self.send_message(&address, &message).await;
    }

    async fn send_message(&mut self, address: &SocketAddr, message: &Packet) {
        debug!("send_message {:?} {:?}", address, message);
        self.tx_sender.send((message.clone(), *address)).unwrap();
    }

    fn gen_message_id(&mut self) -> u16 {
        self.current_message_id += 1;
        return self.current_message_id;
    }

    fn format_register(address: &SocketAddr) -> String {
        format!("{}", address)
    }

    fn format_register_resource(address: &SocketAddr, path: &String) -> String {
        format!("{}${}", address, path)
    }
}

#[cfg(test)]
mod test {
    use super::super::*;
    use super::*;
    use coap_lite::CoapResponse;
    use std::{io::ErrorKind, sync::mpsc, time::Duration};

    async fn request_handler(req: CoapRequest<SocketAddr>) -> Option<CoapResponse> {
        match req.get_method() {
            &coap_lite::RequestType::Get => {
                let observe_option = req.get_observe_flag().unwrap().unwrap();
                assert_eq!(observe_option, ObserveOption::Deregister);
            }
            &coap_lite::RequestType::Put => {}
            _ => panic!("unexpected request"),
        }

        match req.response {
            Some(mut response) => {
                response.message.payload = b"OK".to_vec();
                Some(response)
            }
            _ => None,
        }
    }

    #[test]
    fn test_observe() {
        let path = "/test";
        let payload1 = b"data1".to_vec();
        let payload2 = b"data2".to_vec();
        let (tx, rx) = mpsc::channel();
        let (tx2, rx2) = mpsc::channel();
        let mut step = 1;

        let server_port = server::test::spawn_server("127.0.0.1:0", request_handler)
            .recv()
            .unwrap();

        let server_address = &format!("127.0.0.1:{}", server_port);

        let mut client = CoAPClient::new(server_address).unwrap();

        tx.send(step).unwrap();
        let mut request = CoapRequest::new();
        request.set_method(coap_lite::RequestType::Put);
        request.set_path(path);
        request.message.payload = payload1.clone();
        client.send(&request).unwrap();
        client.receive().unwrap();

        let payload1_clone = payload1.clone();
        let payload2_clone = payload2.clone();

        let mut receive_step = 1;
        client
            .observe(path, move |msg| {
                match rx.try_recv() {
                    Ok(n) => receive_step = n,
                    _ => (),
                }

                match receive_step {
                    1 => assert_eq!(msg.payload, payload1_clone),
                    2 => {
                        assert_eq!(msg.payload, payload2_clone);
                        tx2.send(()).unwrap();
                    }
                    _ => panic!("unexpected step"),
                }
            })
            .unwrap();

        step = 2;
        tx.send(step).unwrap();

        request.message.payload = payload2.clone();

        let client2 = CoAPClient::new(server_address).unwrap();
        client2.send(&request).unwrap();
        client2.receive().unwrap();
        assert_eq!(rx2.recv_timeout(Duration::new(5, 0)).unwrap(), ());
    }

    #[test]
    fn test_unobserve() {
        let path = "/test";
        let payload1 = b"data1".to_vec();
        let payload2 = b"data2".to_vec();

        let server_port = server::test::spawn_server("127.0.0.1:0", request_handler)
            .recv()
            .unwrap();

        let server_address = &format!("127.0.0.1:{}", server_port);

        let mut client = CoAPClient::new(server_address).unwrap();

        let mut request = CoapRequest::new();
        request.set_method(coap_lite::RequestType::Put);
        request.set_path(path);
        request.message.payload = payload1.clone();
        client.send(&request).unwrap();
        client.receive().unwrap();

        let payload1_clone = payload1.clone();
        client
            .observe(path, move |msg| {
                assert_eq!(msg.payload, payload1_clone);
            })
            .unwrap();

        client.unobserve();

        request.message.payload = payload2.clone();

        let client3 = CoAPClient::new(server_address).unwrap();
        client3.send(&request).unwrap();
        client3.receive().unwrap();
    }

    #[test]
    fn test_observe_without_resource() {
        let path = "/test";

        let server_port = server::test::spawn_server("127.0.0.1:0", request_handler)
            .recv()
            .unwrap();

        let mut client = CoAPClient::new(format!("127.0.0.1:{}", server_port)).unwrap();
        let error = client.observe(path, |_msg| {}).unwrap_err();
        assert_eq!(error.kind(), ErrorKind::NotFound);
    }
}
