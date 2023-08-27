use alloc::string::String;
use alloc::vec::Vec;
use coap_lite::{
    block_handler::{extending_splice, BlockValue, RequestCacheKey},
    error::HandlingError,
    CoapOption, CoapRequest, CoapResponse, ObserveOption, Packet, RequestType as Method,
    ResponseType as Status,
};
use core::mem;
use core::ops::Deref;
use log::*;
use lru_time_cache::LruCache;
use regex::Regex;
use std::io::{Error, ErrorKind, Result};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, ToSocketAddrs, UdpSocket};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;
use url::Url;

const DEFAULT_RECEIVE_TIMEOUT: u64 = 1; // 1s

enum ObserveMessage {
    Terminate,
}

pub struct CoAPClient {
    socket: UdpSocket,
    peer_addr: SocketAddr,
    observe_sender: Option<mpsc::Sender<ObserveMessage>>,
    observe_thread: Option<thread::JoinHandle<()>>,
    block2_states: LruCache<RequestCacheKey<SocketAddr>, BlockState>,
    block1_size: usize,
    message_id: u16,
}

impl CoAPClient {
    /// Create a CoAP client with the specific source and peer address.
    pub fn new_with_specific_source<A: ToSocketAddrs, B: ToSocketAddrs>(
        bind_addr: A,
        peer_addr: B,
    ) -> Result<CoAPClient> {
        const MAX_PAYLOAD_BLOCK: usize = 1024;
        peer_addr
            .to_socket_addrs()
            .and_then(|mut iter| match iter.next() {
                Some(paddr) => UdpSocket::bind(bind_addr).and_then(|s| {
                    s.set_read_timeout(Some(Duration::new(DEFAULT_RECEIVE_TIMEOUT, 0)))
                        .and_then(|_| {
                            Ok(CoAPClient {
                                socket: s,
                                peer_addr: paddr,
                                observe_sender: None,
                                observe_thread: None,
                                block2_states: LruCache::with_expiry_duration(Duration::from_secs(
                                    120,
                                )),
                                block1_size: MAX_PAYLOAD_BLOCK,
                                message_id: 0,
                            })
                        })
                }),
                None => Err(Error::new(ErrorKind::Other, "no address")),
            })
    }

    /// Create a CoAP client with the peer address.
    pub fn new<A: ToSocketAddrs>(addr: A) -> Result<CoAPClient> {
        addr.to_socket_addrs()
            .and_then(|mut iter| match iter.next() {
                Some(SocketAddr::V4(_)) => Self::new_with_specific_source("0.0.0.0:0", addr),
                Some(SocketAddr::V6(_)) => Self::new_with_specific_source(":::0", addr),
                None => Err(Error::new(ErrorKind::Other, "no address")),
            })
    }

    /// Execute a single get request with a coap url
    pub fn get(url: &str) -> Result<CoapResponse> {
        Self::request(url, Method::Get, None)
    }

    /// Execute a single get request with a coap url and a specific timeout.
    pub fn get_with_timeout(url: &str, timeout: Duration) -> Result<CoapResponse> {
        Self::request_with_timeout(url, Method::Get, None, timeout)
    }

    /// Execute a single post request with a coap url
    pub fn post(url: &str, data: Vec<u8>) -> Result<CoapResponse> {
        Self::request(url, Method::Post, Some(data))
    }

    /// Execute a single post request with a coap url
    pub fn post_with_timeout(url: &str, data: Vec<u8>, timeout: Duration) -> Result<CoapResponse> {
        Self::request_with_timeout(url, Method::Post, Some(data), timeout)
    }

    /// Execute a put request with a coap url
    pub fn put(url: &str, data: Vec<u8>) -> Result<CoapResponse> {
        Self::request(url, Method::Put, Some(data))
    }

    /// Execute a single put request with a coap url
    pub fn put_with_timeout(url: &str, data: Vec<u8>, timeout: Duration) -> Result<CoapResponse> {
        Self::request_with_timeout(url, Method::Put, Some(data), timeout)
    }

    /// Execute a single delete request with a coap url
    pub fn delete(url: &str) -> Result<CoapResponse> {
        Self::request(url, Method::Delete, None)
    }

    /// Execute a single delete request with a coap url
    pub fn delete_with_timeout(url: &str, timeout: Duration) -> Result<CoapResponse> {
        Self::request_with_timeout(url, Method::Delete, None, timeout)
    }

    /// Execute a single request (GET, POST, PUT, DELETE) with a coap url
    pub fn request(url: &str, method: Method, data: Option<Vec<u8>>) -> Result<CoapResponse> {
        let (domain, port, path, queries) = Self::parse_coap_url(url)?;
        let mut client = Self::new((domain.as_str(), port))?;
        client.request_path(&path, method, data, queries, Some(domain))
    }

    /// Execute a single request (GET, POST, PUT, DELETE) with a coap url and a specfic timeout
    pub fn request_with_timeout(
        url: &str,
        method: Method,
        data: Option<Vec<u8>>,
        timeout: Duration,
    ) -> Result<CoapResponse> {
        let (domain, port, path, queries) = Self::parse_coap_url(url)?;
        let mut client = Self::new((domain.as_str(), port))?;
        client.request_path_with_timeout(&path, method, data, queries, Some(domain), timeout)
    }

    /// Execute a request (GET, POST, PUT, DELETE)
    pub fn request_path(
        &mut self,
        path: &str,
        method: Method,
        data: Option<Vec<u8>>,
        queries: Option<Vec<u8>>,
        domain: Option<String>,
    ) -> Result<CoapResponse> {
        self.request_path_with_timeout(
            path,
            method,
            data,
            queries,
            domain,
            Duration::new(DEFAULT_RECEIVE_TIMEOUT, 0),
        )
    }

    /// Execute a request (GET, POST, PUT, DELETE) with a specfic timeout. This method will
    /// try to use block1 requests with a block size of 1024 by default.
    pub fn request_path_with_timeout(
        &mut self,
        path: &str,
        method: Method,
        data: Option<Vec<u8>>,
        queries: Option<Vec<u8>>,
        domain: Option<String>,
        timeout: Duration,
    ) -> Result<CoapResponse> {
        let mut request = CoapRequest::new();
        request.set_method(method);
        request.set_path(path);
        if let Some(q) = queries {
            request.message.add_option(CoapOption::UriQuery, q);
        }
        if let Some(d) = domain {
            request
                .message
                .add_option(CoapOption::UriHost, d.as_str().as_bytes().to_vec());
        }
        request.message.header.message_id = Self::gen_message_id(&mut self.message_id);

        match data {
            Some(data) => request.message.payload = data,
            None => (),
        }

        self.set_receive_timeout(Some(timeout))?;
        self.send2(&mut request)?;
        self.receive2(&mut request)
    }
    pub fn set_broadcast(&self, value: bool) -> Result<()> {
        self.socket.set_broadcast(value)
    }

    pub fn observe<H: FnMut(Packet) + Send + 'static>(
        &mut self,
        resource_path: &str,
        handler: H,
    ) -> Result<()> {
        self.observe_with_timeout(
            resource_path,
            handler,
            Duration::new(DEFAULT_RECEIVE_TIMEOUT, 0),
        )
    }

    /// Observe a resource with the handler and specified timeout
    pub fn observe_with_timeout<H: FnMut(Packet) + Send + 'static>(
        &mut self,
        resource_path: &str,
        mut handler: H,
        timeout: Duration,
    ) -> Result<()> {
        // TODO: support observe multi resources at the same time
        let mut message_id = self.message_id;
        let mut register_packet = CoapRequest::new();
        register_packet.set_observe_flag(ObserveOption::Register);
        register_packet.message.header.message_id = Self::gen_message_id(&mut message_id);
        register_packet.set_path(resource_path);

        self.send(&register_packet)?;

        self.set_receive_timeout(Some(timeout))?;
        let response = self.receive()?;
        if *response.get_status() != Status::Content {
            return Err(Error::new(ErrorKind::NotFound, "the resource not found"));
        }

        handler(response.message);

        let socket;
        match self.socket.try_clone() {
            Ok(good_socket) => socket = good_socket,
            Err(_) => return Err(Error::new(ErrorKind::Other, "network error")),
        }
        let peer_addr = self.peer_addr.clone();
        let (observe_sender, observe_receiver) = mpsc::channel();
        let observe_path = String::from(resource_path);

        let observe_thread = thread::spawn(move || loop {
            match Self::receive_from_socket(&socket) {
                Ok((packet, _src)) => {
                    let receive_packet = CoapRequest::from_packet(packet, &peer_addr);

                    handler(receive_packet.message);

                    if let Some(response) = receive_packet.response {
                        let mut packet = Packet::new();
                        packet.header.set_type(response.message.header.get_type());
                        packet.header.message_id = response.message.header.message_id;
                        packet.set_token(response.message.get_token().into());

                        match Self::send_with_socket(&socket, &peer_addr, &packet) {
                            Ok(_) => (),
                            Err(e) => {
                                warn!("reply ack failed {}", e)
                            }
                        }
                    }
                }
                Err(e) => match e.kind() {
                    ErrorKind::WouldBlock => {
                        info!("Observe timeout");
                    }
                    _ => warn!("observe failed {:?}", e),
                },
            };

            match observe_receiver.try_recv() {
                Ok(ObserveMessage::Terminate) => {
                    let mut deregister_packet = CoapRequest::<SocketAddr>::new();
                    deregister_packet.message.header.message_id =
                        Self::gen_message_id(&mut message_id);
                    deregister_packet.set_observe_flag(ObserveOption::Deregister);
                    deregister_packet.set_path(observe_path.as_str());

                    Self::send_with_socket(&socket, &peer_addr, &deregister_packet.message)
                        .unwrap();
                    Self::receive_from_socket(&socket).unwrap();
                    break;
                }
                _ => continue,
            }
        });
        self.observe_sender = Some(observe_sender);
        self.observe_thread = Some(observe_thread);

        return Ok(());
    }

    /// Stop observing
    pub fn unobserve(&mut self) {
        match self.observe_sender.take() {
            Some(ref sender) => {
                sender.send(ObserveMessage::Terminate).unwrap();

                self.observe_thread.take().map(|g| g.join().unwrap());
            }
            _ => {}
        }
    }

    /// Execute a request.
    pub fn send(&self, request: &CoapRequest<SocketAddr>) -> Result<()> {
        Self::send_with_socket(&self.socket, &self.peer_addr, &request.message)
    }

    /// send a request supporting block1 option based on the block size set in the client
    pub fn send2(&mut self, request: &mut CoapRequest<SocketAddr>) -> Result<()> {
        let request_length = request.message.payload.len();
        if request_length <= self.block1_size {
            return self.send(request);
        }
        let payload = std::mem::take(&mut request.message.payload);
        let mut it = payload.chunks(self.block1_size).enumerate().peekable();
        while let Some((idx, elem)) = it.next() {
            let more_blocks = it.peek().is_some();
            let block = BlockValue::new(idx, more_blocks, self.block1_size)
                .map_err(|_| Error::new(ErrorKind::Other, "could not set block size"))?;

            request.message.clear_option(CoapOption::Block1);
            request
                .message
                .add_option_as::<BlockValue>(CoapOption::Block1, block.clone());
            request.message.payload = elem.to_vec();

            request.message.header.message_id = Self::gen_message_id(&mut self.message_id);
            self.send(request)?;
            // continue receiving responses until last element
            if it.peek().is_some() {
                let resp = self.receive()?;
                let maybe_block1 = resp
                    .message
                    .get_first_option_as::<BlockValue>(CoapOption::Block1)
                    .ok_or(Error::new(
                        ErrorKind::Unsupported,
                        "endpoint does not support blockwise transfers. Try setting block1_size to a larger value",
                    ))?;
                let block1_resp = maybe_block1.map_err(|_| {
                    Error::new(
                        ErrorKind::InvalidData,
                        "endpoint responded with invalid block",
                    )
                })?;
                //TODO: negotiate smaller block size
                if block1_resp.size_exponent != block.size_exponent {
                    return Err(Error::new(
                        ErrorKind::Unsupported,
                        "negotiating block size is currently unsupported",
                    ));
                }
            }
        }
        Ok(())
    }
    /// Send a request to all CoAP devices.
    /// - IPv4 AllCoAP multicast address is '224.0.1.187'
    /// - IPv6 AllCoAp multicast addresses are 'ff0?::fd'
    /// Parameter segment is used with IPv6 to determine the first octet.
    /// It's value can be between 0x0 and 0xf. To address multiple segments,
    /// you have to call send_all_coap for each of the segments.
    pub fn send_all_coap(&self, request: &CoapRequest<SocketAddr>, segment: u8) -> Result<()> {
        assert!(segment <= 0xf);
        let addr = match self.peer_addr {
            SocketAddr::V4(val) => {
                SocketAddr::new(IpAddr::V4(Ipv4Addr::new(224, 0, 1, 187)), val.port())
            }
            SocketAddr::V6(val) => SocketAddr::new(
                IpAddr::V6(Ipv6Addr::new(
                    0xff00 + segment as u16,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0xfd,
                )),
                val.port(),
            ),
        };

        match request.message.to_bytes() {
            Ok(bytes) => {
                let size = self.socket.send_to(&bytes[..], addr)?;
                if size == bytes.len() {
                    Ok(())
                } else {
                    Err(Error::new(ErrorKind::Other, "send length error"))
                }
            }
            Err(_) => Err(Error::new(ErrorKind::InvalidInput, "packet error")),
        }
    }

    /// Receive a response.
    pub fn receive(&self) -> Result<CoapResponse> {
        let (packet, _src) = Self::receive_from_socket(&self.socket)?;
        Ok(CoapResponse { message: packet })
    }

    /// Receive a response support block-wise.
    pub fn receive2(&mut self, request: &mut CoapRequest<SocketAddr>) -> Result<CoapResponse> {
        loop {
            let (packet, _src) = Self::receive_from_socket(&self.socket)?;
            request.response = CoapResponse::new(&request.message);
            let response = request
                .response
                .as_mut()
                .ok_or_else(|| Error::new(ErrorKind::Interrupted, "packet error"))?;
            response.message = packet;
            match self.intercept_response(request) {
                Ok(true) => {
                    self.send(request)?;
                }
                Err(err) => {
                    error!("intercept response error: {:?}", err);
                    return Err(Error::new(ErrorKind::Interrupted, "packet error"));
                }
                Ok(false) => {
                    break;
                }
            }
        }
        Ok(CoapResponse {
            message: request.response.as_ref().unwrap().message.clone(),
        })
    }

    /// Receive a response.
    pub fn receive_from(&self) -> Result<(CoapResponse, SocketAddr)> {
        let (packet, src) = Self::receive_from_socket(&self.socket)?;
        Ok((CoapResponse { message: packet }, src))
    }

    /// Set the receive timeout.
    pub fn set_receive_timeout(&self, dur: Option<Duration>) -> Result<()> {
        self.socket.set_read_timeout(dur)
    }

    /// Set the maximum size for a block1 request. Default is 1024 bytes
    pub fn set_block1_size(&mut self, block1_max_bytes: usize) {
        self.block1_size = block1_max_bytes;
    }

    fn send_with_socket(
        socket: &UdpSocket,
        peer_addr: &SocketAddr,
        message: &Packet,
    ) -> Result<()> {
        let message_bytes = message
            .to_bytes()
            .map_err(|_| Error::new(ErrorKind::InvalidInput, "packet error"))?;
        let size = socket.send_to(&message_bytes[..], peer_addr)?;
        if size == message_bytes.len() {
            Ok(())
        } else {
            Err(Error::new(ErrorKind::Other, "send length error"))
        }
    }

    fn receive_from_socket(socket: &UdpSocket) -> Result<(Packet, SocketAddr)> {
        let mut buf = [0; 1500];

        let (nread, src) = socket.recv_from(&mut buf)?;
        match Packet::from_bytes(&buf[..nread]) {
            Ok(packet) => Ok((packet, src)),
            Err(_) => Err(Error::new(ErrorKind::InvalidInput, "packet error")),
        }
    }

    fn parse_coap_url(url: &str) -> Result<(String, u16, String, Option<Vec<u8>>)> {
        let url_params = match Url::parse(url) {
            Ok(url_params) => url_params,
            Err(_) => return Err(Error::new(ErrorKind::InvalidInput, "url error")),
        };

        let host = match url_params.host_str() {
            Some("") => return Err(Error::new(ErrorKind::InvalidInput, "host error")),
            Some(h) => h,
            None => return Err(Error::new(ErrorKind::InvalidInput, "host error")),
        };
        let host = Regex::new(r"^\[(.*?)]$")
            .unwrap()
            .replace(&host, "$1")
            .to_string();

        let port = match url_params.port() {
            Some(p) => p,
            None => 5683,
        };

        let path = url_params.path().to_string();

        let queries = url_params.query().map(|q| q.as_bytes().to_vec());

        return Ok((host.to_string(), port, path, queries));
    }

    fn gen_message_id(message_id: &mut u16) -> u16 {
        (*message_id) += 1;
        return *message_id;
    }

    fn intercept_response(
        &mut self,
        request: &mut CoapRequest<SocketAddr>,
    ) -> std::result::Result<bool, HandlingError> {
        let state = self
            .block2_states
            .entry(request.deref().into())
            .or_insert(BlockState::default());

        let block2_handled = Self::maybe_handle_response_block2(request, state)?;
        if block2_handled {
            return Ok(true);
        }

        Ok(false)
    }

    fn maybe_handle_response_block2(
        request: &mut CoapRequest<SocketAddr>,
        state: &mut BlockState,
    ) -> std::result::Result<bool, HandlingError> {
        let response = request.response.as_ref().unwrap();
        let maybe_block2 = response
            .message
            .get_first_option_as::<BlockValue>(CoapOption::Block2)
            .and_then(|x| x.ok());

        if let Some(block2) = maybe_block2 {
            if state.cached_payload.is_none() {
                state.cached_payload = Some(Vec::new());
            }
            let cached_payload = state.cached_payload.as_mut().unwrap();

            let payload_offset = usize::from(block2.num) * block2.size();
            extending_splice(
                cached_payload,
                payload_offset..payload_offset + block2.size(),
                response.message.payload.iter().copied(),
                16 * 1024,
            )
            .map_err(HandlingError::internal)?;

            if block2.more {
                request.message.clear_option(CoapOption::Block2);
                let mut next_block2 = block2.clone();
                next_block2.num += 1;
                request
                    .message
                    .add_option_as::<BlockValue>(CoapOption::Block2, next_block2);
                return Ok(true);
            } else {
                let cached_payload = mem::take(&mut state.cached_payload).unwrap();
                request.response.as_mut().unwrap().message.payload = cached_payload;
            }
        }

        Ok(false)
    }
}

impl Drop for CoAPClient {
    fn drop(&mut self) {
        self.unobserve();
    }
}

#[derive(Debug, Clone, Default)]
pub struct BlockState {
    cached_payload: Option<Vec<u8>>,
}

#[cfg(test)]
mod test {

    use super::super::*;
    use super::*;
    use std::io::ErrorKind;
    use std::time::Duration;

    #[test]
    fn test_parse_coap_url_good_url() {
        assert!(CoAPClient::parse_coap_url("coap://127.0.0.1").is_ok());
        assert!(CoAPClient::parse_coap_url("coap://127.0.0.1:5683").is_ok());
        assert!(CoAPClient::parse_coap_url("coap://[::1]").is_ok());
        assert!(CoAPClient::parse_coap_url("coap://[::1]:5683").is_ok());
        assert!(CoAPClient::parse_coap_url("coap://[bbbb::9329:f033:f558:7418]").is_ok());
        assert!(CoAPClient::parse_coap_url("coap://[bbbb::9329:f033:f558:7418]:5683").is_ok());
        assert!(CoAPClient::parse_coap_url("coap://127.0.0.1/?hello=world").is_ok());
    }

    #[test]
    fn test_parse_coap_url_bad_url() {
        assert!(CoAPClient::parse_coap_url("coap://127.0.0.1:65536").is_err());
        assert!(CoAPClient::parse_coap_url("coap://").is_err());
        assert!(CoAPClient::parse_coap_url("coap://:5683").is_err());
        assert!(CoAPClient::parse_coap_url("127.0.0.1").is_err());
    }

    async fn request_handler(_: CoapRequest<SocketAddr>) -> Option<CoapResponse> {
        None
    }

    #[test]
    fn test_parse_queries() {
        if let Ok((_, _, _, Some(queries))) =
            CoAPClient::parse_coap_url("coap://127.0.0.1/?hello=world&test1=test2")
        {
            assert_eq!("hello=world&test1=test2".as_bytes().to_vec(), queries);
        } else {
            error!("Parse Queries failed");
        }
    }

    #[test]
    fn test_get_url() {
        let resp = CoAPClient::get("coap://coap.me:5683/hello").unwrap();
        assert_eq!(resp.message.payload, b"world".to_vec());
    }

    #[test]
    fn test_get_url_timeout() {
        let server_port = server::test::spawn_server("127.0.0.1:0", request_handler)
            .recv()
            .unwrap();

        let error = CoAPClient::get_with_timeout(
            &format!("coap://127.0.0.1:{}/Rust", server_port),
            Duration::new(1, 0),
        )
        .unwrap_err();
        if cfg!(windows) {
            assert_eq!(error.kind(), ErrorKind::TimedOut);
        } else {
            assert_eq!(error.kind(), ErrorKind::WouldBlock);
        }
    }

    #[test]
    fn test_get() {
        let domain = "coap.me";
        let mut client = CoAPClient::new((domain, 5683)).unwrap();
        let resp = client
            .request_path("/hello", Method::Get, None, None, Some(domain.to_string()))
            .unwrap();
        assert_eq!(resp.message.payload, b"world".to_vec());
    }
    #[test]
    fn test_post_url() {
        let resp = CoAPClient::post("coap://coap.me:5683/validate", b"world".to_vec()).unwrap();
        assert_eq!(resp.message.payload, b"POST OK".to_vec());
        let resp = CoAPClient::post("coap://coap.me:5683/validate", b"test".to_vec()).unwrap();
        assert_eq!(resp.message.payload, b"POST OK".to_vec());
    }

    #[test]
    fn test_post() {
        let domain = "coap.me";
        let mut client = CoAPClient::new((domain, 5683)).unwrap();
        let resp = client
            .request_path(
                "/validate",
                Method::Post,
                Some(b"world".to_vec()),
                None,
                Some(domain.to_string()),
            )
            .unwrap();
        assert_eq!(resp.message.payload, b"POST OK".to_vec());
    }

    #[test]
    fn test_put_url() {
        let resp = CoAPClient::put("coap://coap.me:5683/create1", b"world".to_vec()).unwrap();
        assert_eq!(resp.message.payload, b"Created".to_vec());
        let resp = CoAPClient::put("coap://coap.me:5683/create1", b"test".to_vec()).unwrap();
        assert_eq!(resp.message.payload, b"Created".to_vec());
    }

    #[test]
    fn test_put() {
        let domain = "coap.me";
        let mut client = CoAPClient::new((domain, 5683)).unwrap();
        let resp = client
            .request_path(
                "/create1",
                Method::Put,
                Some(b"world".to_vec()),
                None,
                Some(domain.to_string()),
            )
            .unwrap();
        assert_eq!(resp.message.payload, b"Created".to_vec());
    }

    #[test]
    fn test_delete_url() {
        let resp = CoAPClient::delete("coap://coap.me:5683/validate").unwrap();
        assert_eq!(resp.message.payload, b"DELETE OK".to_vec());
        let resp = CoAPClient::delete("coap://coap.me:5683/validate").unwrap();
        assert_eq!(resp.message.payload, b"DELETE OK".to_vec());
    }

    #[test]
    fn test_delete() {
        let domain = "coap.me";
        let mut client = CoAPClient::new((domain, 5683)).unwrap();
        let resp = client
            .request_path(
                "/validate",
                Method::Delete,
                None,
                None,
                Some(domain.to_string()),
            )
            .unwrap();
        assert_eq!(resp.message.payload, b"DELETE OK".to_vec());
    }

    #[test]
    fn test_set_broadcast() {
        let client = CoAPClient::new(("127.0.0.1", 5683)).unwrap();
        assert!(client.set_broadcast(true).is_ok());
        assert!(client.set_broadcast(false).is_ok());
    }

    #[test]
    #[ignore]
    fn test_set_broadcast_v6() {
        let client = CoAPClient::new(("::1", 5683)).unwrap();
        assert!(client.set_broadcast(true).is_ok());
        assert!(client.set_broadcast(false).is_ok());
    }

    #[test]
    fn test_send_all_coap() {
        // prepare the Non-confirmable request with the broadcast message
        let mut request: CoapRequest<SocketAddr> = CoapRequest::new();
        request.set_method(Method::Get);
        request.set_path("/");
        request
            .message
            .header
            .set_type(coap_lite::MessageType::NonConfirmable);
        request.message.payload = b"Discovery".to_vec();

        let client = CoAPClient::new(("127.0.0.1", 5683)).unwrap();
        assert!(client.send_all_coap(&request, 0).is_ok());
    }

    #[test]
    fn test_change_block_option() {
        // this test is a little finnicky because it relies on the configuration
        // of the reception endpoint. It tries to send a payload larger than the
        // default using a block option, this request is expected to fail because
        // the endpoint does not support block requests. Afterwards, we change the
        // maximum block size and thus expect the request to work.
        const PAYLOAD_STR: &str = "this is a payload";
        let mut large_payload = vec![];
        while large_payload.len() < 1024 {
            large_payload.extend_from_slice(PAYLOAD_STR.as_bytes());
        }
        let domain = "coap.me";
        let mut client = CoAPClient::new((domain, 5683)).unwrap();
        let resp = client.request_path(
            "/large-create",
            Method::Put,
            Some(large_payload.clone()),
            None,
            Some(domain.to_string()),
        );
        let err = resp.unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Unsupported);
        //we now set the block size to make sure it is sent in a single request
        client.set_block1_size(10_000_000);

        let resp = client
            .request_path(
                "/large-create",
                Method::Post,
                Some(large_payload.clone()),
                None,
                Some(domain.to_string()),
            )
            .unwrap();
        assert_eq!(*resp.get_status(), Status::Created);
    }
    #[test]
    #[ignore]
    fn test_send_all_coap_v6() {
        // prepare the Non-confirmable request with the broadcast message
        let mut request: CoapRequest<SocketAddr> = CoapRequest::new();
        request.set_method(Method::Get);
        request.set_path("/");
        request
            .message
            .header
            .set_type(coap_lite::MessageType::NonConfirmable);
        request.message.payload = b"Discovery".to_vec();

        let client = CoAPClient::new(("::1", 5683)).unwrap();
        assert!(client.send_all_coap(&request, 0x4).is_ok());
    }
}
