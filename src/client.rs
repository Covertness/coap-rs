use std::io::{Error, ErrorKind, Result};
use std::net::{SocketAddr, ToSocketAddrs, UdpSocket, IpAddr, Ipv4Addr, Ipv6Addr};
use std::time::Duration;
use std::thread;
use std::sync::mpsc;
use url::Url;
use log::*;
use regex::Regex;
use coap_lite::{
    ObserveOption,
    RequestType as Method,
    ResponseType as Status,
    Packet, CoapRequest, CoapResponse
};

const DEFAULT_RECEIVE_TIMEOUT: u64 = 1; // 1s

enum ObserveMessage {
    Terminate,
}

pub struct CoAPClient {
    socket: UdpSocket,
    peer_addr: SocketAddr,
    observe_sender: Option<mpsc::Sender<ObserveMessage>>,
    observe_thread: Option<thread::JoinHandle<()>>,
}

impl CoAPClient {
    /// Create a CoAP client with the specific source and peer address.
    pub fn new_with_specific_source<A: ToSocketAddrs, B: ToSocketAddrs>(
        bind_addr: A,
        peer_addr: B,
    ) -> Result<CoAPClient> {
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
        Self::get_with_timeout(url, Duration::new(DEFAULT_RECEIVE_TIMEOUT, 0))
    }

    /// Execute a single get request with a coap url and a specific timeout.
    pub fn get_with_timeout(url: &str, timeout: Duration) -> Result<CoapResponse> {
        let (domain, port, path) = Self::parse_coap_url(url)?;

        let mut packet = CoapRequest::new();
        packet.set_path(path.as_str());

        let client = Self::new((domain.as_str(), port))?;
        client.send(&packet)?;

        client.set_receive_timeout(Some(timeout))?;
        match client.receive() {
            Ok(receive_packet) => Ok(receive_packet),
            Err(e) => Err(e),
        }
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
        let (domain, port, path) = Self::parse_coap_url(url)?;
        let client = Self::new((domain, port))?;
        client.request_path(&path, method, data)
    }

    /// Execute a single request (GET, POST, PUT, DELETE) with a coap url and a specfic timeout
    pub fn request_with_timeout(url: &str, method: Method, data: Option<Vec<u8>>, timeout: Duration) -> Result<CoapResponse> {
        let (domain, port, path) = Self::parse_coap_url(url)?;
        let client = Self::new((domain, port))?;
        client.request_path_with_timeout(&path, method, data, timeout)
    }

    /// Execute a request (GET, POST, PUT, DELETE)
    pub fn request_path(&self, path: &str, method: Method, data: Option<Vec<u8>>) -> Result<CoapResponse> {
        self.request_path_with_timeout(path, method, data, Duration::new(DEFAULT_RECEIVE_TIMEOUT, 0))
    }

    /// Execute a request (GET, POST, PUT, DELETE) with a specfic timeout
    pub fn request_path_with_timeout(&self, path: &str, method: Method, data: Option<Vec<u8>>, timeout: Duration) -> Result<CoapResponse> {
        let mut request = CoapRequest::new();
        request.set_method(method);
        request.set_path(path);
    
        match data {
            Some(data) => request.message.payload = data,
            None => (),
        }

        self.set_receive_timeout(Some(timeout))?;
        self.send(&request).unwrap();
        self.receive()
    }

    pub fn set_broadcast(&self, value: bool) -> Result<()> {
        self.socket.set_broadcast(value)
    }

    /// Observe a resource with the handler
    pub fn observe<H: FnMut(Packet) + Send + 'static>(&mut self, resource_path: &str, mut handler: H) -> Result<()> {
        // TODO: support observe multi resources at the same time
        let mut message_id: u16 = 0;
        let mut register_packet = CoapRequest::new();
        register_packet.message.set_observe(vec![ObserveOption::Register as u8]);
        register_packet.message.header.message_id = Self::gen_message_id(&mut message_id);
        register_packet.set_path(resource_path);

        self.send(&register_packet)?;

        self.set_receive_timeout(Some(Duration::new(DEFAULT_RECEIVE_TIMEOUT, 0)))?;
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
                Ok(packet) => {
                    let receive_packet = CoapRequest::from_packet(packet, &peer_addr);

                    handler(receive_packet.message);

                    if let Some(response) = receive_packet.response {
                        let mut packet = Packet::new();
                        packet.header.set_type(response.message.header.get_type());
                        packet.header.message_id = response.message.header.message_id;
                        packet.set_token(response.message.get_token().clone());

                        match Self::send_with_socket(&socket, &peer_addr, &packet) {
                            Ok(_) => (),
                            Err(e) => {
                                warn!("reply ack failed {}", e)
                            }
                        }
                    }
                },
                Err(e) => {
                    match e.kind() {
                        ErrorKind::WouldBlock => (),                          // timeout
                        _ => warn!("observe failed {:?}", e),
                    }
                },
            };

            match observe_receiver.try_recv() {
                Ok(ObserveMessage::Terminate) => {
                    let mut deregister_packet = CoapRequest::<SocketAddr>::new();
                    deregister_packet.message.header.message_id = Self::gen_message_id(&mut message_id);
                    deregister_packet.message.set_observe(vec![ObserveOption::Deregister as u8]);
                    deregister_packet.set_path(observe_path.as_str());

                    Self::send_with_socket(&socket, &peer_addr, &deregister_packet.message).unwrap();
                    Self::receive_from_socket(&socket).unwrap();
                    break;
                },
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
            },
            SocketAddr::V6(val) => {
                SocketAddr::new(IpAddr::V6(Ipv6Addr::new(0xff00 + segment as u16, 0, 0, 0, 0, 0, 0, 0xfd)), val.port())
            },
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
        let packet = Self::receive_from_socket(&self.socket)?;
        Ok(CoapResponse { message: packet })
    }

    /// Set the receive timeout.
    pub fn set_receive_timeout(&self, dur: Option<Duration>) -> Result<()> {
        self.socket.set_read_timeout(dur)
    }

    fn send_with_socket(socket: &UdpSocket, peer_addr: &SocketAddr, message: &Packet) -> Result<()> {
        match message.to_bytes() {
            Ok(bytes) => {
                let size = socket.send_to(&bytes[..], peer_addr)?;
                if size == bytes.len() {
                    Ok(())
                } else {
                    Err(Error::new(ErrorKind::Other, "send length error"))
                }
            }
            Err(_) => Err(Error::new(ErrorKind::InvalidInput, "packet error")),
        }
    }

    fn receive_from_socket(socket: &UdpSocket) -> Result<Packet> {
        let mut buf = [0; 1500];

        let (nread, _src) = socket.recv_from(&mut buf)?;
        match Packet::from_bytes(&buf[..nread]) {
            Ok(packet) => Ok(packet),
            Err(_) => Err(Error::new(ErrorKind::InvalidInput, "packet error")),
        }
    }

    fn parse_coap_url(url: &str) -> Result<(String, u16, String)> {
        let url_params = match Url::parse(url) {
            Ok(url_params) => url_params,
            Err(_) => return Err(Error::new(ErrorKind::InvalidInput, "url error")),
        };

        let host = match url_params.host_str() {
            Some("") => return Err(Error::new(ErrorKind::InvalidInput, "host error")),
            Some(h) => h,
            None => return Err(Error::new(ErrorKind::InvalidInput, "host error")),
        };
        let host = Regex::new(r"^\[(.*?)]$").unwrap().replace(&host, "$1").to_string();

        let port = match url_params.port() {
            Some(p) => p,
            None => 5683,
        };

        let path = url_params.path().to_string();

        return Ok((host.to_string(), port, path));
    }

    fn gen_message_id(message_id: &mut u16) -> u16 {
        (*message_id) += 1;
        return *message_id;
    }
}

impl Drop for CoAPClient {
    fn drop(&mut self) {
        self.unobserve();
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use super::super::*;
    use std::time::Duration;
    use std::io::ErrorKind;

    #[test]
    fn test_parse_coap_url_good_url() {
        assert!(CoAPClient::parse_coap_url("coap://127.0.0.1").is_ok());
        assert!(CoAPClient::parse_coap_url("coap://127.0.0.1:5683").is_ok());
        assert!(CoAPClient::parse_coap_url("coap://[::1]").is_ok());
        assert!(CoAPClient::parse_coap_url("coap://[::1]:5683").is_ok());
        assert!(CoAPClient::parse_coap_url("coap://[bbbb::9329:f033:f558:7418]").is_ok());
        assert!(CoAPClient::parse_coap_url("coap://[bbbb::9329:f033:f558:7418]:5683").is_ok());
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
    fn test_get_url() {
        let resp = CoAPClient::get("coap://coap.me:5683/hello")
            .unwrap();
        assert_eq!(resp.message.payload, b"world".to_vec());
    }
 
    #[test]
    fn test_get_url_timeout() {
        let server_port = server::test::spawn_server(request_handler).recv().unwrap();

        let error = CoAPClient::get_with_timeout(&format!("coap://127.0.0.1:{}/Rust", server_port), Duration::new(1, 0))
            .unwrap_err();
        if cfg!(windows) {
            assert_eq!(error.kind(), ErrorKind::TimedOut);
        } else {
            assert_eq!(error.kind(), ErrorKind::WouldBlock);
        }
    }

    #[test]
    fn test_get() {
        let client = CoAPClient::new(("coap.me", 5683)).unwrap();
        let resp = client.request_path("/hello", Method::Get, None).unwrap();
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
        let client = CoAPClient::new(("coap.me", 5683)).unwrap();
        let resp = client.request_path("/validate", Method::Post, Some(b"world".to_vec())).unwrap();
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
        let client = CoAPClient::new(("coap.me", 5683)).unwrap();
        let resp = client.request_path("/create1", Method::Put, Some(b"world".to_vec())).unwrap();
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
        let client = CoAPClient::new(("coap.me", 5683)).unwrap();
        let resp = client.request_path("/validate", Method::Delete, None).unwrap();
        assert_eq!(resp.message.payload, b"DELETE OK".to_vec());
    }

    #[test]
    fn test_set_broadcast() {
        let client = CoAPClient::new(("127.0.0.1", 5683)).unwrap();
        assert!(client.set_broadcast(true).is_ok());
        assert!(client.set_broadcast(false).is_ok());
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
        request.message.header.set_type(coap_lite::MessageType::NonConfirmable);
        request.message.payload = b"Discovery".to_vec();

        let client = CoAPClient::new(("127.0.0.1", 5683)).unwrap();
        assert!(client.send_all_coap(&request, 0).is_ok());
        let client = CoAPClient::new(("::1", 5683)).unwrap();
        assert!(client.send_all_coap(&request, 0x1).is_ok());
    }
}
