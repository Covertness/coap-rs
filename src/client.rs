use std::io::{Result, Error, ErrorKind};
use std::net::{ToSocketAddrs, SocketAddr, UdpSocket};
use packet::Packet;

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
}