use std;
use std::io::{Error, ErrorKind};
use std::thread;
use std::net::{ToSocketAddrs, SocketAddr};
use std::sync::mpsc;
use mio::{EventLoop, PollOpt, EventSet, Handler, Sender, Token};
use mio::udp::UdpSocket;
use packet::Packet;
use threadpool::ThreadPool;

const DEFAULT_WORKER_NUM: usize = 4;
pub type TxQueue = mpsc::Sender<CoAPResponse>;
pub type RxQueue = mpsc::Receiver<CoAPResponse>;

#[derive(Debug)]
pub enum CoAPServerError {
	NetworkError,
	EventLoopError,
	AnotherHandlerIsRunning,
}

#[derive(Debug)]
pub struct CoAPResponse {
	pub address: SocketAddr,
	pub response: Packet
}

pub trait CoAPHandler: Sync + Send + Copy {
	fn handle(&self, Packet) -> Option<Packet>;
}

impl<F> CoAPHandler for F where F: Fn(Packet) -> Option<Packet>, F: Sync + Send + Copy {
	fn handle(&self, request: Packet) -> Option<Packet> {
		return self(request);
	}
}

struct UdpHandler<H: CoAPHandler + 'static> {
	socket: UdpSocket,
	thread_pool: ThreadPool,
	tx_sender: TxQueue,
	coap_handler: H
}

impl<H: CoAPHandler + 'static> UdpHandler<H> {
	fn new(socket: UdpSocket, thread_pool: ThreadPool, tx_sender: TxQueue, coap_handler: H) -> UdpHandler<H> {
		UdpHandler {
			socket: socket,
			thread_pool: thread_pool,
			tx_sender: tx_sender,
			coap_handler: coap_handler
		}
	}
}

impl<H: CoAPHandler + 'static> Handler for UdpHandler<H> {
	type Timeout = usize;
	type Message = ();

	fn ready(&mut self, _: &mut EventLoop<UdpHandler<H>>, _: Token, events: EventSet) {
		if !events.is_readable() {
			warn!("Unreadable Event");
			return;
		}

		let coap_handler = self.coap_handler;
		let mut buf = [0; 1500];

		match self.socket.recv_from(&mut buf) {
			Ok(Some((nread, src))) => {
				debug!("Handling request from {}", src);
				let response_q = self.tx_sender.clone();
				self.thread_pool.execute(move || {

					match Packet::from_bytes(&buf[..nread]) {
						Ok(packet) => {
							// Dispatch user handler, if there is a response packet
							//   send the reply via the TX thread
							match coap_handler.handle(packet) {
								Some(response) => {
									debug!("Response: {:?}", response);
									response_q.send(CoAPResponse{
										address: src,
										response: response
									}).unwrap();
								},
								None => {
									debug!("No response");
								}
							};
						},
						Err(_) => {
							error!("Failed to parse request");
							return;
						}
					};


				});
			},
			_ => {
				error!("Failed to read from socket");
				panic!("unexpected error");
			},
		}

	}

	fn notify(&mut self, event_loop: &mut EventLoop<UdpHandler<H>>, _: ()) {
		info!("Shutting down request handler");
        event_loop.shutdown();
    }
}

pub struct CoAPServer {
    socket: UdpSocket,
    event_sender: Option<Sender<()>>,
    event_thread: Option<thread::JoinHandle<()>>,
    tx_thread: Option<thread::JoinHandle<()>>,
    worker_num: usize,
}

impl CoAPServer {
	/// Creates a CoAP server listening on the given address.
	pub fn new<A: ToSocketAddrs>(addr: A) -> std::io::Result<CoAPServer> {
		addr.to_socket_addrs().and_then(|mut iter| {
			match iter.next() {
				Some(ad) => {
					UdpSocket::bound(&ad).and_then(|s| Ok(CoAPServer {
						socket: s,
						event_sender: None,
						event_thread: None,
						tx_thread: None,
						worker_num: DEFAULT_WORKER_NUM,
					}))
				},
				None => Err(Error::new(ErrorKind::Other, "no address"))
			}
		})
	}

	/// Starts handling requests with the handler
	pub fn handle<H: CoAPHandler + 'static>(&mut self, handler: H) -> Result<(), CoAPServerError> {
		let socket;

		// Early return error checking
		if let Some(_) = self.event_sender {
			error!("Handler already running!");
			return Err(CoAPServerError::AnotherHandlerIsRunning);
		}
		match self.socket.try_clone() {
			Ok(good_socket) => {
				socket = good_socket
			},
			Err(_) => {
				error!("Network Error!");
				return Err(CoAPServerError::NetworkError);
			},
		}

		// Create resources
		let worker_num = self.worker_num;
		let (tx, rx) = mpsc::channel();
		let (tx_send, tx_recv) : (TxQueue, RxQueue) = mpsc::channel();
		let tx_only = self.socket.try_clone().unwrap();

		// Setup and spawn single TX thread
		let tx_thread = thread::spawn(move || {
			transmit_handler(tx_recv, tx_only);
		});

		// Setup and spawn event loop thread, which will spawn
		//   children threads which handle incomining requests
		let thread = thread::spawn(move || {
			let thread_pool = ThreadPool::new(worker_num);
			let mut event_loop = EventLoop::new().unwrap();
			event_loop.register(&socket, Token(0), EventSet::readable(), PollOpt::edge()).unwrap();

			tx.send(event_loop.channel()).unwrap();

			event_loop.run(&mut UdpHandler::new(socket, thread_pool, tx_send, handler)).unwrap();
		});

		// Ensure threads started successfully
		match rx.recv() {
			Ok(event_sender) => {
				self.event_sender = Some(event_sender);
				self.event_thread = Some(thread);
				self.tx_thread = Some(tx_thread);
				Ok(())
			},
			Err(_) => Err(CoAPServerError::EventLoopError)
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

	/// Set the number of threads for handling requests
	pub fn set_worker_num(&mut self, worker_num: usize) {
		self.worker_num = worker_num;
	}
}

fn transmit_handler(tx_recv: RxQueue, tx_only: UdpSocket) {
	// Note! We should only transmit with this UDP Socket
	// TODO: Add better support for failure detection or logging
	loop {
		match tx_recv.recv() {
			Ok(q_res) => {
				match q_res.response.to_bytes() {
					Ok(bytes) => {
						let _ = tx_only.send_to(&bytes[..], &q_res.address);
					},
					Err(_) => {
						error!("Failed to decode response");
					}
				}
			},
			// recv error occurs when all transmitters are terminited
			//   (when all UDP Handlers are closed)
			Err(_) => {
				info!("Shutting down Transmit Handler");
				break;
			}
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
	use packet::{Packet, PacketType, OptionType, auto_response};
	use client::CoAPClient;

	fn request_handler(req: Packet) -> Option<Packet> {
		let uri_path_list = req.get_option(OptionType::UriPath).unwrap();
		assert!(uri_path_list.len() == 1);

		return match auto_response(req) {
			Ok(mut response) => {
				response.set_payload(uri_path_list.front().unwrap().clone());
				Some(response)
			},
			Err(_) => None
		};
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

	#[test]
	fn test_echo_server_no_token() {
		let mut server = CoAPServer::new("127.0.0.1:5683").unwrap();
		server.handle(request_handler).unwrap();

		let client = CoAPClient::new("127.0.0.1:5683").unwrap();
		let mut packet = Packet::new();
		packet.header.set_version(1);
		packet.header.set_type(PacketType::Confirmable);
		packet.header.set_code("0.01");
		packet.header.set_message_id(1);
		packet.add_option(OptionType::UriPath, b"test-echo".to_vec());
		client.send(&packet).unwrap();

		let recv_packet = client.receive().unwrap();
		assert_eq!(recv_packet.payload, b"test-echo".to_vec());
	}
}
