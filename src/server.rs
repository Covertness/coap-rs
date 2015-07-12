use std;
use std::thread;
use std::net::ToSocketAddrs;
use std::sync::mpsc;
use mio::*;
use mio::udp::UdpSocket;
use packet::Packet;
use client::CoAPClient;

#[derive(Debug)]
pub enum CoAPServerError {
	NetworkError,
	EventLoopError,
	AnotherHandlerIsRunning,
}

pub trait CoAPHandler: Sync + Send + Copy {
	fn handle(&self, Packet, CoAPClient);
}

impl<F> CoAPHandler for F where F: Fn(Packet, CoAPClient), F: Sync + Send + Copy {
	fn handle(&self, request: Packet, response: CoAPClient) {
		self(request, response);
	}
}

struct UdpHandler<H: CoAPHandler + 'static>(UdpSocket, H);

impl<H: CoAPHandler + 'static> Handler for UdpHandler<H> {
	type Timeout = ();
	type Message = ();

	fn readable(&mut self, _: &mut EventLoop<UdpHandler<H>>, _: Token, _: ReadHint) {
		let UdpHandler(ref mut socket, coap_handler) = *self;
		let mut buf = [0; 1500];

		match socket.recv_from(&mut buf) {
			Ok((nread, src)) => {
				thread::spawn(move || {
					match Packet::from_bytes(&buf[..nread]) {
						Ok(packet) => {
							let client = CoAPClient::new(src).unwrap();
							coap_handler.handle(packet, client);
						},
						Err(_) => return
					};
				});
			},
			Err(_) => panic!("unexpected error"),
		}
	}

	fn notify(&mut self, event_loop: &mut EventLoop<UdpHandler<H>>, _: ()) {
        event_loop.shutdown();
    }
}

pub struct CoAPServer {
    socket: UdpSocket,
    event_sender: Option<Sender<()>>,
    event_thread: Option<thread::JoinHandle<()>>,
}

impl CoAPServer {
	/// Creates a CoAP server listening on the given address.
	pub fn new<A: ToSocketAddrs>(addr: A) -> std::io::Result<CoAPServer> {
		UdpSocket::bind(addr).and_then(|s| Ok(CoAPServer {
			socket: s,
			event_sender: None,
			event_thread: None,
		}))
	}

	/// Starts handling requests with the handler.
	pub fn handle<H: CoAPHandler + 'static>(&mut self, handler: H) -> Result<(), CoAPServerError> {
		match self.event_sender {
			None => {
				let (tx, rx) = mpsc::channel();
				let socket = self.socket.try_clone();
				match socket {
					Ok(socket) => {
						let thread = thread::spawn(move || {
							let mut event_loop = EventLoop::new().unwrap();
							event_loop.register(&socket, Token(0)).unwrap();

							tx.send(event_loop.channel()).unwrap();

							event_loop.run(&mut UdpHandler(socket, handler)).unwrap();
						});

						match rx.recv() {
							Ok(event_sender) => {
								self.event_sender = Some(event_sender);
								self.event_thread = Some(thread);
								Ok(())
							},
							Err(_) => Err(CoAPServerError::EventLoopError)
						}
					},
					Err(_) => Err(CoAPServerError::NetworkError),
				}
			},
			Some(_) => Err(CoAPServerError::AnotherHandlerIsRunning),
		}
	}

	/// Stop the server.
	pub fn stop(&mut self) {
		let event_sender = self.event_sender.take();
		match event_sender {
			Some(ref sender) => {
				sender.send(()).unwrap();
				self.event_thread.take().map(|g| g.join());
			},
			_ => {},
		}
	}
}

impl Drop for CoAPServer {
    fn drop(&mut self) {
        self.stop();
    }
}


#[cfg(test)]
mod test {
	use super::*;
	use packet::{Packet, PacketType, OptionType};
	use client::CoAPClient;

	fn request_handler(req: Packet, resp: CoAPClient) {
		let uri_path = req.get_option(OptionType::UriPath);
		assert!(uri_path.is_some());
		let uri_path = uri_path.unwrap();
		let mut packet = Packet::new();

		packet.header.set_version(1);
		packet.header.set_type(PacketType::Acknowledgement);
		packet.header.set_code("2.05");
		packet.header.set_message_id(req.header.get_message_id());
		packet.set_token(req.get_token().clone());
		packet.payload = uri_path.front().unwrap().clone();
		resp.send(&packet).unwrap();
	}

	#[test]
	fn test_echo_server() {
		let mut server = CoAPServer::new("127.0.0.1:5683").unwrap();
		server.handle(request_handler).unwrap();
		
		let client = CoAPClient::new("127.0.0.1:5683").unwrap();
		let mut packet = Packet::new();
		packet.header.set_version(1);
		packet.header.set_type(PacketType::Confirmable);
		packet.header.set_code("0.01");
		packet.header.set_message_id(1);
		packet.set_token(vec!(0x51, 0x55, 0x77, 0xE8));
		packet.add_option(OptionType::UriPath, b"test-echo".to_vec());
		client.send(&packet).unwrap();

		let recv_packet = client.receive().unwrap();
		assert_eq!(recv_packet.payload, b"test-echo".to_vec());
	}
}