use std;
use std::io::{Error, ErrorKind};
use std::thread;
use std::net::{ToSocketAddrs, SocketAddr};
use std::sync::mpsc;
use mio::{EventLoop, PollOpt, EventSet, Handler, Sender, Token};
use mio::udp::UdpSocket;
use message::packet::Packet;
use message::request::CoAPRequest;
use message::response::CoAPResponse;

type TxQueue = mpsc::Sender<QueuedResponse>;
type RxQueue = mpsc::Receiver<QueuedResponse>;

#[derive(Debug)]
pub enum CoAPServerError {
    NetworkError,
    EventLoopError,
    AnotherHandlerIsRunning,
}

#[derive(Debug)]
struct QueuedResponse {
    pub address: SocketAddr,
    pub response: CoAPResponse,
}

pub trait CoAPHandler: Sync + Send + Copy {
    fn handle(&self, CoAPRequest) -> Option<CoAPResponse>;
}

impl<F> CoAPHandler for F
    where F: Fn(CoAPRequest) -> Option<CoAPResponse>,
          F: Sync + Send + Copy
{
    fn handle(&self, request: CoAPRequest) -> Option<CoAPResponse> {
        return self(request);
    }
}

struct UdpHandler<H: CoAPHandler + 'static> {
    socket: UdpSocket,
    tx_sender: TxQueue,
    rx_recv: RxQueue,
    coap_handler: H,
}

impl<H: CoAPHandler + 'static> UdpHandler<H> {
    fn new(socket: UdpSocket,
           tx_sender: TxQueue,
           rx_recv: RxQueue,
           coap_handler: H)
           -> UdpHandler<H> {
        UdpHandler {
            socket: socket,
            tx_sender: tx_sender,
            rx_recv: rx_recv,
            coap_handler: coap_handler,
        }
    }

    fn request_handler(&self) -> Option<QueuedResponse> {
        match self.requset_recv() {
            Some(rqst) => {
                let src = rqst.source.unwrap();
                match self.coap_handler.handle(rqst) {
                    Some(response) => {
                        debug!("Response: {:?}", response);
                        return Some(QueuedResponse {
                            address: src,
                            response: response,
                        });
                    }
                    None => {
                        debug!("No response");
                        return None;
                    }
                }
            }
            None => {
                return None;
            }
        }
    }

    fn response_handler(&self) {
        loop {
            match self.rx_recv.try_recv() {
                Ok(q_res) => {
                    self.response_send(q_res);
                }
                Err(mpsc::TryRecvError::Empty) => {
                    break;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    error!("The RxQueue become disconnected");
                    break;
                }
            }
        }
    }

    fn requset_recv(&self) -> Option<CoAPRequest> {
        let mut buf = [0; 1500];

        match self.socket.recv_from(&mut buf) {
            Ok(Some((nread, src))) => {
                debug!("Handling request from {}", src);

                match Packet::from_bytes(&buf[..nread]) {
                    Ok(packet) => {
                        return Some(CoAPRequest::from_packet(packet, &src));
                    }
                    Err(_) => {
                        error!("Failed to parse request");
                        return None;
                    }
                }
            }
            _ => {
                error!("Failed to read from socket");
                panic!("unexpected error");
            }
        }
    }

    fn response_send(&self, q_res: QueuedResponse) {
        match q_res.response.message.to_bytes() {
            Ok(bytes) => {
                match self.socket.send_to(&bytes[..], &q_res.address) {
                    Ok(None) => {
                        // Look at https://github.com/carllerche/mio/issues/411 in detail
                        error!("Failed to complete the response");
                    }
                    Ok(_) => {}
                    Err(_) => {
                        error!("Failed to send response");
                    }
                }
            }
            Err(_) => {
                error!("Failed to decode response");
            }
        }
    }
}

impl<H: CoAPHandler + 'static> Handler for UdpHandler<H> {
    type Timeout = usize;
    type Message = ();

    fn ready(&mut self, event_loop: &mut EventLoop<UdpHandler<H>>, _: Token, events: EventSet) {
        if events.is_writable() {
            // handle the response
            self.response_handler();
            event_loop.reregister(&self.socket, Token(0), EventSet::readable(), PollOpt::edge()).unwrap();
            return;
        } else if events.is_readable() {
            // handle the request
            match self.request_handler() {
                Some(response) => {
                    self.tx_sender.send(response).unwrap();
                    event_loop.reregister(&self.socket, Token(0), EventSet::writable(), PollOpt::edge()).unwrap();
                }
                None => {}
            }
        } else {
            warn!("Unreadable Event, {:?}", events);
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
}

impl CoAPServer {
    /// Creates a CoAP server listening on the given address.
    pub fn new<A: ToSocketAddrs>(addr: A) -> std::io::Result<CoAPServer> {
        addr.to_socket_addrs().and_then(|mut iter| {
            match iter.next() {
                Some(ad) => {
                    UdpSocket::bound(&ad).and_then(|s| {
                        Ok(CoAPServer {
                            socket: s,
                            event_sender: None,
                            event_thread: None,
                        })
                    })
                }
                None => Err(Error::new(ErrorKind::Other, "no address")),
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
            Ok(good_socket) => socket = good_socket,
            Err(_) => {
                error!("Network Error!");
                return Err(CoAPServerError::NetworkError);
            }
        }

        // Create resources
        let (tx, rx) = mpsc::channel();
        let (tx_send, tx_recv): (TxQueue, RxQueue) = mpsc::channel();

        // Setup and spawn event loop thread, which will spawn
        //   children threads which handle incomining requests
        let thread = thread::spawn(move || {
            let mut event_loop = EventLoop::new().unwrap();
            event_loop.register(&socket, Token(0), EventSet::readable(), PollOpt::edge()).unwrap();

            tx.send(event_loop.channel()).unwrap();

            event_loop.run(&mut UdpHandler::new(socket, tx_send, tx_recv, handler)).unwrap();
        });

        // Ensure threads started successfully
        match rx.recv() {
            Ok(event_sender) => {
                self.event_sender = Some(event_sender);
                self.event_thread = Some(thread);
                Ok(())
            }
            Err(_) => Err(CoAPServerError::EventLoopError),
        }
    }

    /// Stop the server.
    pub fn stop(&mut self) {
        let event_sender = self.event_sender.take();
        match event_sender {
            Some(ref sender) => {
                sender.send(()).unwrap();
                self.event_thread.take().map(|g| g.join());
            }
            _ => {}
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
    use client::CoAPClient;
    use message::header;
    use message::IsMessage;
    use message::packet::CoAPOption;
    use message::request::CoAPRequest;
    use message::response::CoAPResponse;
    use super::*;

    fn request_handler(req: CoAPRequest) -> Option<CoAPResponse> {
        let uri_path_list = req.get_option(CoAPOption::UriPath).unwrap();
        assert!(uri_path_list.len() == 1);

        match req.response {
            Some(mut response) => {
                response.set_payload(uri_path_list.front().unwrap().clone());
                Some(response)
            }
            _ => None,
        }
    }

    #[test]
    fn test_echo_server() {
        let mut server = CoAPServer::new("127.0.0.1:5683").unwrap();
        server.handle(request_handler).unwrap();

        let client = CoAPClient::new("127.0.0.1:5683").unwrap();
        let mut request = CoAPRequest::new();
        request.set_version(1);
        request.set_type(header::MessageType::Confirmable);
        request.set_code("0.01");
        request.set_message_id(1);
        request.set_token(vec![0x51, 0x55, 0x77, 0xE8]);
        request.add_option(CoAPOption::UriPath, b"test-echo".to_vec());
        client.send(&request).unwrap();

        let recv_packet = client.receive().unwrap();
        assert_eq!(recv_packet.message.payload, b"test-echo".to_vec());
    }

    #[test]
    fn test_echo_server_no_token() {
        let mut server = CoAPServer::new("127.0.0.1:5685").unwrap();
        server.handle(request_handler).unwrap();

        let client = CoAPClient::new("127.0.0.1:5685").unwrap();
        let mut packet = CoAPRequest::new();
        packet.set_version(1);
        packet.set_type(header::MessageType::Confirmable);
        packet.set_code("0.01");
        packet.set_message_id(1);
        packet.add_option(CoAPOption::UriPath, b"test-echo".to_vec());
        client.send(&packet).unwrap();

        let recv_packet = client.receive().unwrap();
        assert_eq!(recv_packet.message.payload, b"test-echo".to_vec());
    }
}
