use std::io::{Result, Error, ErrorKind};
use std::net::{ToSocketAddrs, SocketAddr, UdpSocket};
use url::{UrlParser, SchemeType};
use num;
use rand::{thread_rng, random, Rng};
use packet::{Packet, PacketType, OptionType};

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
						Ok(CoAPClient {
							socket: s,
							peer_addr: SocketAddr::V4(a),
						})
					})
				},
				Some(SocketAddr::V6(a)) => {
					UdpSocket::bind(":::0").and_then(|s| {
						Ok(CoAPClient {
							socket: s,
							peer_addr: SocketAddr::V6(a),
						})
					})
				},
				None => Err(Error::new(ErrorKind::Other, "no address"))
			}
		})
	}

	/// Execute a request with the coap url.
	pub fn request(url: &str) -> Result<Packet> {
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

	/// Response the client with the specifc payload.
	pub fn reply(&self, request_packet: &Packet, payload: Vec<u8>) -> Result<()> {
		let mut packet = Packet::new();

		packet.header.set_version(1);
		let response_type = match request_packet.header.get_type() {
			PacketType::Confirmable => PacketType::Acknowledgement,
			PacketType::NonConfirmable => PacketType::NonConfirmable,
			_ => return Err(Error::new(ErrorKind::InvalidInput, "request type error"))
		};
		packet.header.set_type(response_type);
		packet.header.set_code("2.05");
		packet.header.set_message_id(request_packet.header.get_message_id());
		packet.set_token(request_packet.get_token().clone());
		packet.payload = payload;
		self.send(&packet)
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

	fn coap_scheme_type_mapper(scheme: &str) -> SchemeType {
		match scheme {
			"coap" => SchemeType::Relative(5683),
			_ => SchemeType::NonRelative,
		}
	}
}