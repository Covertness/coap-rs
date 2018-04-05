use std::io::{Result, Error, ErrorKind};
use std::net::{ToSocketAddrs, SocketAddr, UdpSocket};
use std::time::Duration;
use url::{UrlParser, SchemeType};
use num;
use rand::{thread_rng, random, Rng};
use message::packet::Packet;
use message::header::MessageType;
use message::response::CoAPResponse;
use message::request::{CoAPRequest, Method};
use message::IsMessage;

const DEFAULT_RECEIVE_TIMEOUT: u64 = 5;  // 5s

pub struct CoAPClient {
    socket: UdpSocket,
    peer_addr: SocketAddr,
}

impl CoAPClient {
    /// Create a CoAP client with the specific source and peer address.
    pub fn new_with_specific_source<A: ToSocketAddrs, B: ToSocketAddrs>(bind_addr: A, peer_addr: B) -> Result<CoAPClient> {
        peer_addr.to_socket_addrs().and_then(|mut iter| {
            match iter.next() {
                Some(paddr) => {
                    UdpSocket::bind(bind_addr).and_then(|s| {
                        s.set_read_timeout(Some(Duration::new(DEFAULT_RECEIVE_TIMEOUT, 0)))
                            .and_then(|_| {
                                Ok(CoAPClient {
                                    socket: s,
                                    peer_addr: paddr,
                                })
                            })
                    })
                }
                None => Err(Error::new(ErrorKind::Other, "no address")),
            }
        })
    }

    /// Create a CoAP client with the peer address.
    pub fn new<A: ToSocketAddrs>(addr: A) -> Result<CoAPClient> {
        addr.to_socket_addrs().and_then(|mut iter| {
            match iter.next() {
                Some(SocketAddr::V4(_)) => {
                    Self::new_with_specific_source("0.0.0.0:0", addr)
                }
                Some(SocketAddr::V6(_)) => {
                    Self::new_with_specific_source(":::0", addr)
                }
                None => Err(Error::new(ErrorKind::Other, "no address")),
            }
        })
    }

    /// Execute a get request
    pub fn get(url: &str) -> Result<CoAPResponse> {
        Self::get_with_timeout(url, Duration::new(DEFAULT_RECEIVE_TIMEOUT, 0))
    }

    /// Execute a get request with the coap url and a specific timeout.
    pub fn get_with_timeout(url: &str, timeout: Duration) -> Result<CoAPResponse> {
        let mut url_parser = UrlParser::new();
        url_parser.scheme_type_mapper(Self::coap_scheme_type_mapper);

        match url_parser.parse(url) {
            Ok(url_params) => {
                let mut packet = CoAPRequest::new();

                let domain = match url_params.domain() {
                    Some(d) => d,
                    None => return Err(Error::new(ErrorKind::InvalidInput, "domain error")),
                };
                let port = match url_params.port_or_default() {
                    Some(p) => p,
                    None => return Err(Error::new(ErrorKind::InvalidInput, "port error")),
                };

                match url_params.serialize_path() {
                    Some(path) => packet.set_path(path.as_str()),
                    None => return Err(Error::new(ErrorKind::InvalidInput, "uri error")),
                };

                let client = try!(Self::new((domain, port)));
                try!(client.send(&packet));

                try!(client.set_receive_timeout(Some(timeout)));
                match client.receive() {
                    Ok(receive_packet) => Ok(receive_packet),
                    Err(e) => Err(e),
                }
            }
            Err(_) => Err(Error::new(ErrorKind::InvalidInput, "url error")),
        }
    }

    /// Execute a request with the coap url and a specific timeout. Default timeout is 5s.
    #[deprecated(since="0.6.0", note="please use `get_with_timeout` instead")]
    pub fn request_with_timeout(url: &str, timeout: Option<Duration>) -> Result<CoAPResponse> {
        let mut url_parser = UrlParser::new();
        url_parser.scheme_type_mapper(Self::coap_scheme_type_mapper);

        match url_parser.parse(url) {
            Ok(url_params) => {
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

                let domain = match url_params.domain() {
                    Some(d) => d,
                    None => return Err(Error::new(ErrorKind::InvalidInput, "domain error")),
                };
                let port = url_params.port_or_default().unwrap();

                if let Some(path) = url_params.serialize_path() {
                    packet.set_path(path.as_str());
                };

                let client = try!(Self::new((domain, port)));
                try!(client.send(&packet));

                try!(client.set_receive_timeout(timeout));
                match client.receive() {
                    Ok(receive_packet) => {
                        if receive_packet.get_message_id() == message_id &&
                           *receive_packet.get_token() == token {
                            return Ok(receive_packet);
                        } else {
                            return Err(Error::new(ErrorKind::Other, "receive invalid data"));
                        }
                    }
                    Err(e) => Err(e),
                }
            }
            Err(_) => Err(Error::new(ErrorKind::InvalidInput, "url error")),
        }
    }

    /// Execute a request with the coap url.
    #[deprecated(since="0.6.0", note="please use `get` instead")]
    pub fn request(url: &str) -> Result<CoAPResponse> {
        Self::get_with_timeout(url, Duration::new(DEFAULT_RECEIVE_TIMEOUT, 0))
    }

    /// Execute a request.
    pub fn send(&self, request: &CoAPRequest) -> Result<()> {
        match request.message.to_bytes() {
            Ok(bytes) => {
                let size = try!(self.socket.send_to(&bytes[..], self.peer_addr));
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
    pub fn receive(&self) -> Result<CoAPResponse> {
        let mut buf = [0; 1500];

        let (nread, _src) = try!(self.socket.recv_from(&mut buf));
        match Packet::from_bytes(&buf[..nread]) {
            Ok(packet) => Ok(CoAPResponse { message: packet }),
            Err(_) => Err(Error::new(ErrorKind::InvalidInput, "packet error")),
        }
    }

    /// Set the receive timeout.
    pub fn set_receive_timeout(&self, dur: Option<Duration>) -> Result<()> {
        self.socket.set_read_timeout(dur)
    }

    fn coap_scheme_type_mapper(scheme: &str) -> SchemeType {
        match scheme {
            "coap" => SchemeType::Relative(5683),
            _ => SchemeType::NonRelative,
        }
    }
}


#[cfg(test)]
mod test {
    use super::*;
    use std::time::Duration;
    use std::io::ErrorKind;
    use message::request::CoAPRequest;
    use message::response::CoAPResponse;
    use server::CoAPServer;

    #[test]
    fn test_get_error_url() {
        assert!(CoAPClient::get("http://127.0.0.1").is_err());
        assert!(CoAPClient::get("coap://127.0.0.").is_err());
        assert!(CoAPClient::get("127.0.0.1").is_err());
    }

    fn request_handler(_: CoAPRequest) -> Option<CoAPResponse> {
        None
    }

    #[test]
    fn test_get_timeout() {
        let mut server = CoAPServer::new("127.0.0.1:5684").unwrap();
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
