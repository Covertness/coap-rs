use std::io::{Error, ErrorKind, Result};
use std::net::{SocketAddr, ToSocketAddrs, UdpSocket};
use std::time::Duration;
use std::thread;
use std::sync::{Arc, RwLock, mpsc};
use url::Url;
use num;
use rand::{random, thread_rng, Rng};
use log::*;
use super::message::packet::{Packet, ObserveOption};
use super::message::header::MessageType;
use super::message::response::{CoAPResponse, Status};
use super::message::request::{CoAPRequest, Method};
use super::message::IsMessage;
use regex::Regex;

const DEFAULT_RECEIVE_TIMEOUT: u64 = 1; // 1s

enum ObserveMessage {
    Terminate,
}

pub struct CoAPClient {
    socket: UdpSocket,
    peer_addr: SocketAddr,
    observe_sender: Option<mpsc::Sender<ObserveMessage>>,
    observe_thread: Option<thread::JoinHandle<()>>,
    message_id: Arc<RwLock<u16>>,
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
                                message_id: Arc::new(RwLock::new(thread_rng().gen_range(0, num::pow(2u32, 16)) as u16)),
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

    /// Execute a get request
    pub fn get(url: &str) -> Result<CoAPResponse> {
        Self::get_with_timeout(url, Duration::new(DEFAULT_RECEIVE_TIMEOUT, 0))
    }

    /// Execute a get request with the coap url and a specific timeout.
    pub fn get_with_timeout(url: &str, timeout: Duration) -> Result<CoAPResponse> {
        let (domain, port, path) = Self::parse_coap_url(url)?;

        let mut packet = CoAPRequest::new();
        packet.set_path(path.as_str());

        let client = Self::new((domain.as_str(), port))?;
        client.send(&packet)?;

        client.set_receive_timeout(Some(timeout))?;
        match client.receive() {
            Ok(receive_packet) => Ok(receive_packet),
            Err(e) => Err(e),
        }
    }

    /// Observe a resource with the handler
    pub fn observe<H: FnMut(Packet) + Send + 'static>(&mut self, resource_path: &str, mut handler: H) -> Result<()> {
        // TODO: support observe multi resources at the same time
        let mut register_packet = CoAPRequest::new();
        register_packet.set_observe(vec![ObserveOption::Register as u8]);
        register_packet.set_message_id(Self::gen_message_id(&self.message_id));
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
        let message_id = self.message_id.clone();

        let observe_thread = thread::spawn(move || loop {
            match Self::receive_from_socket(&socket) {
                Ok(packet) => {
                    let receive_packet = CoAPRequest::from_packet(packet, &peer_addr);

                    handler(receive_packet.message);

                    if let Some(response) = receive_packet.response {
                        let mut packet = Packet::new();
                        packet.header.set_type(response.message.header.get_type());
                        packet.header.set_message_id(response.message.header.get_message_id());
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
                    let mut deregister_packet = CoAPRequest::new();
                    deregister_packet.set_message_id(Self::gen_message_id(&message_id));
                    deregister_packet.set_observe(vec![ObserveOption::Deregister as u8]);
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

    /// Execute a request with the coap url and a specific timeout. Default timeout is 5s.
    #[deprecated(since = "0.6.0", note = "please use `get_with_timeout` instead")]
    pub fn request_with_timeout(url: &str, timeout: Option<Duration>) -> Result<CoAPResponse> {
        let (domain, port, path) = Self::parse_coap_url(url)?;

        let mut packet = CoAPRequest::new();
        packet.set_version(1);
        packet.set_type(MessageType::Confirmable);
        packet.set_method(Method::Get);

        let message_id = thread_rng().gen_range(0, num::pow(2u32, 16)) as u16;
        packet.set_message_id(message_id);

        let mut token: Vec<u8> = vec![1, 1, 1, 1];
        for x in token.iter_mut() {
            *x = random()
        }
        packet.set_token(token.clone());
        packet.set_path(path.as_str());

        let client = r#try!(Self::new((domain.as_str(), port)));
        r#try!(client.send(&packet));

        r#try!(client.set_receive_timeout(timeout));
        match client.receive() {
            Ok(receive_packet) => {
                if receive_packet.get_message_id() == message_id
                    && *receive_packet.get_token() == token
                {
                    return Ok(receive_packet);
                } else {
                    return Err(Error::new(ErrorKind::Other, "receive invalid data"));
                }
            }
            Err(e) => Err(e),
        }
    }

    /// Execute a request with the coap url.
    #[deprecated(since = "0.6.0", note = "please use `get` instead")]
    pub fn request(url: &str) -> Result<CoAPResponse> {
        Self::get_with_timeout(url, Duration::new(DEFAULT_RECEIVE_TIMEOUT, 0))
    }

    /// Execute a request.
    pub fn send(&self, request: &CoAPRequest) -> Result<()> {
        Self::send_with_socket(&self.socket, &self.peer_addr, &request.message)
    }

    /// Receive a response.
    pub fn receive(&self) -> Result<CoAPResponse> {
        let packet = Self::receive_from_socket(&self.socket)?;
        Ok(CoAPResponse { message: packet })
    }

    /// Set the receive timeout.
    pub fn set_receive_timeout(&self, dur: Option<Duration>) -> Result<()> {
        self.socket.set_read_timeout(dur)
    }

    fn send_with_socket(socket: &UdpSocket, peer_addr: &SocketAddr, message: &Packet) -> Result<()> {
        match message.to_bytes() {
            Ok(bytes) => {
                let size = r#try!(socket.send_to(&bytes[..], peer_addr));
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

        let (nread, _src) = r#try!(socket.recv_from(&mut buf));
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

    fn gen_message_id(message_id: &Arc<RwLock<u16>>) -> u16 {
        let mut message_id = message_id.write().expect("Could not lock message id for increment. Fatal error.");
        *message_id += 1;
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
    use std::time::Duration;
    use std::io::ErrorKind;
    use super::super::message::request::CoAPRequest;
    use super::super::message::response::CoAPResponse;
    use super::super::server::CoAPServer;

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

    fn request_handler(_: CoAPRequest) -> Option<CoAPResponse> {
        None
    }

    #[test]
    fn test_get() {
        let mut server = CoAPServer::new("127.0.0.1:5686").unwrap();  // may fail if port happens to be in use!
        server.handle(request_handler).unwrap();

        let error = CoAPClient::get("coap://127.0.0.1:5686/Rust")
            .unwrap_err();
        if cfg!(windows) {
            assert_eq!(error.kind(), ErrorKind::TimedOut);
        } else {
            assert_eq!(error.kind(), ErrorKind::WouldBlock);
        }
    }

    #[test]
    fn test_get_timeout() {
        let mut server = CoAPServer::new("127.0.0.1:5684").unwrap();  // may fail if port happens to be in use!
        server.handle(request_handler).unwrap();

        let error = CoAPClient::get_with_timeout("coap://127.0.0.1:5684/Rust", Duration::new(1, 0))
            .unwrap_err();
        if cfg!(windows) {
            assert_eq!(error.kind(), ErrorKind::TimedOut);
        } else {
            assert_eq!(error.kind(), ErrorKind::WouldBlock);
        }
    }
}
