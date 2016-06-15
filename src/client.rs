use std::io::{Result, Error, ErrorKind};
use std::net::{ToSocketAddrs, SocketAddr, UdpSocket};
use std::time::Duration;
use url::{UrlParser, SchemeType};
use num;
use rand::{thread_rng, random, Rng};
use packet::{Packet, PacketType, OptionType};

const DEFAULT_RECEIVE_TIMEOUT: u64 = 5;  // 5s

pub struct CoAPClient {
    socket: UdpSocket,
    peer_addr: SocketAddr,
}

impl CoAPClient {
	/// Create a CoAP client with the peer address.
	pub fn new<A: ToSocketAddrs>(addr: A) -> Result<CoAPClient> {
		addr.to_socket_addrs().and_then(|mut iter| {
			match iter.next() {
				Some(SocketAddr::V4(a)) => {
					UdpSocket::bind("0.0.0.0:0").and_then(|s| {
						s.set_read_timeout(Some(Duration::new(DEFAULT_RECEIVE_TIMEOUT, 0))).and_then(|_| {
							Ok(CoAPClient {
								socket: s,
								peer_addr: SocketAddr::V4(a),
							})
						})
					})
				},
				Some(SocketAddr::V6(a)) => {
					UdpSocket::bind(":::0").and_then(|s| {
						s.set_read_timeout(Some(Duration::new(DEFAULT_RECEIVE_TIMEOUT, 0))).and_then(|_| {
							Ok(CoAPClient {
								socket: s,
								peer_addr: SocketAddr::V6(a),
							})
						})
					})
				},
				None => Err(Error::new(ErrorKind::Other, "no address"))
			}
		})
	}

	/// Execute a request with the coap url and a specific timeout. Default timeout is 5s.
	pub fn request_with_timeout(url: &str, timeout: Option<Duration>) -> Result<Packet> {
		let mut url_parser = UrlParser::new();
		url_parser.scheme_type_mapper(Self::coap_scheme_type_mapper);

		match url_parser.parse(url) {
			Ok(url_params) => {
				let mut packet = Packet::new();
				packet.header.set_version(1);
				packet.header.set_type(PacketType::Confirmable);
				packet.header.set_code("0.01");

				let message_id = thread_rng().gen_range(0, num::pow(2u32, 16)) as u16;
				packet.header.set_message_id(message_id);

				let mut token: Vec<u8> = vec!(1, 1, 1, 1);
				for x in token.iter_mut() {
				    *x = random()
				}
				packet.set_token(token.clone());

				let domain = match url_params.domain() {
					Some(d) => d,
					None => return Err(Error::new(ErrorKind::InvalidInput, "domain error"))
				};
				let port = url_params.port_or_default().unwrap();

				if let Some(path) = url_params.path() {
					for p in path.iter() {
						packet.add_option(OptionType::UriPath, p.clone().into_bytes().to_vec());
					}
				};

				let client = try!(Self::new((domain, port)));
				try!(client.send(&packet));

				try!(client.set_receive_timeout(timeout));
				match client.receive() {
				 	Ok(receive_packet) => {
				 		if receive_packet.header.get_message_id() == message_id
				 			&& *receive_packet.get_token() == token {
				 				return Ok(receive_packet)
				 			} else {
				 				return Err(Error::new(ErrorKind::Other, "receive invalid data"))
				 			}
				 	},
				 	Err(e) => Err(e)
				}
			},
			Err(_) => Err(Error::new(ErrorKind::InvalidInput, "url error"))
		}
	}

	/// Execute a request with the coap url.
	pub fn request(url: &str) -> Result<Packet> {
		Self::request_with_timeout(url, Some(Duration::new(DEFAULT_RECEIVE_TIMEOUT, 0)))
	}

	/// Execute a request.
	pub fn send(&self, packet: &Packet) -> Result<()> {
		match packet.to_bytes() {
			Ok(bytes) => {
				let size = try!(self.socket.send_to(&bytes[..], self.peer_addr));
				if size == bytes.len() {
					Ok(())
				} else {
					Err(Error::new(ErrorKind::Other, "send length error"))
				}
			},
			Err(_) => Err(Error::new(ErrorKind::InvalidInput, "packet error"))
		}
	}

	/// Receive a response.
	pub fn receive(&self) -> Result<Packet> {
		let mut buf = [0; 1500];

		let (nread, _src) = try!(self.socket.recv_from(&mut buf));
		match Packet::from_bytes(&buf[..nread]) {
			Ok(packet) => Ok(packet),
			Err(_) => Err(Error::new(ErrorKind::InvalidInput, "packet error"))
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
	use packet::Packet;
	use server::CoAPServer;

	#[test]
	fn test_request_error_url() {
		assert!(CoAPClient::request("http://127.0.0.1").is_err());
		assert!(CoAPClient::request("coap://127.0.0.").is_err());
		assert!(CoAPClient::request("127.0.0.1").is_err());
	}

	fn request_handler(_: Packet) -> Option<Packet> {
		None
	}

	#[test]
	fn test_request_timeout() {
		let mut server = CoAPServer::new("127.0.0.1:5684").unwrap();
		server.handle(request_handler).unwrap();

		let error = CoAPClient::request_with_timeout("coap://127.0.0.1:5684/Rust", Some(Duration::new(1, 0))).unwrap_err();
		if cfg!(windows) {
			assert_eq!(error.kind(), ErrorKind::TimedOut);
		} else {
			assert_eq!(error.kind(), ErrorKind::WouldBlock);
		}
	}
}
