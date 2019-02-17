use std;
use std::io::{Error, ErrorKind};
use std::thread;
use std::net::{ToSocketAddrs, SocketAddr};
use std::sync::mpsc;
use mio::{EventLoop, PollOpt, EventSet, Handler, Sender, Token};
use mio::udp::UdpSocket;
use log::{warn, debug, error, info};
use super::message::packet::Packet;
use super::message::request::{CoAPRequest};
use super::message::IsMessage;
use super::message::response::CoAPResponse;
use threadpool::ThreadPool;
use super::observer::Observer;

const DEFAULT_WORKER_NUM: usize = 4;

pub type TxQueue = mpsc::Sender<QueuedMessage>;
type RxQueue = mpsc::Receiver<QueuedMessage>;

#[derive(Debug)]
pub enum CoAPServerError {
    NetworkError,
    EventLoopError,
    AnotherHandlerIsRunning,
    EventSendError,
}

#[derive(Debug)]
pub struct QueuedMessage {
    pub address: SocketAddr,
    pub message: Packet,
}

#[derive(Debug)]
enum EventLoopNotifyType {
    NewResponse,
    Shutdown,
    UpdateResource,
}

#[derive(Debug)]
struct EventLoopNotify {
    notify_type: EventLoopNotifyType,
    request: Option<CoAPRequest>,
}

#[derive(Debug)]
enum EventLoopTimer {
    ObserveTimer,
}

#[derive(Debug)]
enum ResponseError {
    SocketUnwritable,
    SocketError,
    PacketInvalid,
}

pub trait CoAPHandler: Sync + Send + Copy {
    fn handle(&self, request: CoAPRequest) -> Option<CoAPResponse>;
}

impl<F> CoAPHandler for F
    where F: Fn(CoAPRequest) -> Option<CoAPResponse>,
          F: Sync + Send + Copy
{
    fn handle(&self, request: CoAPRequest) -> Option<CoAPResponse> {
        return self(request);
    }
}

struct UdpHandler<H: CoAPHandler + 'static, N: Fn() + Send + 'static> {
    socket: UdpSocket,
    tx_sender: TxQueue,
    rx_recv: RxQueue,
    worker_pool: ThreadPool,
    coap_handler: H,
    observer: Observer<N>,
}

impl<H: CoAPHandler + 'static, N: Fn() + Send + 'static> UdpHandler<H, N> {
    fn new(socket: UdpSocket,
           tx_sender: TxQueue,
           rx_recv: RxQueue,
           worker_num: usize,
           coap_handler: H,
           response_notify: N)
           -> UdpHandler<H, N> {
        let response_q = tx_sender.clone();

        UdpHandler {
            socket: socket,
            tx_sender: tx_sender,
            rx_recv: rx_recv,
            worker_pool: ThreadPool::new(worker_num),
            coap_handler: coap_handler,
            observer: Observer::new(response_q, response_notify),
        }
    }

    fn request_handler(&mut self, event_loop: &mut EventLoop<UdpHandler<H, N>>) {
        match self.requset_recv() {
            Some(rqst) => {
                let filtered = !self.observer.request_handler(&rqst);
                if filtered {
                    return;
                }

                let src = rqst.source.unwrap();
                let coap_handler = self.coap_handler;
                let response_q = self.tx_sender.clone();
                let event_sender = event_loop.channel();

                self.worker_pool.execute(move || {
                    match coap_handler.handle(rqst) {
                        Some(response) => {
                            debug!("Response: {:?}", response);

                            response_q.send(QueuedMessage {
                                address: src,
                                message: response.message,
                            }).unwrap();
                            match event_sender.send(EventLoopNotify {
                                notify_type: EventLoopNotifyType::NewResponse,
                                request: None
                            }) {
                                Ok(()) => {}
                                Err(error) => {
                                    warn!("Notify NewResponse failed, {:?}", error);
                                }
                            }
                        }
                        None => {
                            debug!("No response");
                        }
                    }
                });
            }
            None => {}
        }
    }

    fn response_handler(&self) -> bool {
        loop {
            match self.rx_recv.try_recv() {
                Ok(q_res) => {
                    match self.response_send(&q_res) {
                        Ok(()) => {}
                        Err(ResponseError::SocketUnwritable) => {
                            self.tx_sender.send(q_res).unwrap();
                            return false;
                        }
                        Err(_) => {}
                    }
                }
                Err(mpsc::TryRecvError::Empty) => {
                    return true;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    panic!("The RxQueue become disconnected");
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

    fn response_send(&self, q_res: & QueuedMessage) -> Result<(), ResponseError> {
        match q_res.message.to_bytes() {
            Ok(bytes) => {
                match self.socket.send_to(&bytes[..], &q_res.address) {
                    // Look at https://github.com/carllerche/mio/issues/411 in detail
                    Ok(None) => return Err(ResponseError::SocketUnwritable),
                    Ok(_) => Ok(()),
                    Err(error) => {
                        error!("Failed to send response, {:?}", error);
                        return Err(ResponseError::SocketError);
                    }
                }
            }
            Err(error) => {
                error!("Failed to decode response, {:?}", error);
                return Err(ResponseError::PacketInvalid);
            }
        }
    }
}

impl<H: CoAPHandler + 'static, N: Fn() + Send + 'static> Handler for UdpHandler<H, N> {
    type Timeout = EventLoopTimer;
    type Message = EventLoopNotify;

    fn ready(&mut self, event_loop: &mut EventLoop<UdpHandler<H, N>>, _: Token, events: EventSet) {
        if events.is_writable() {
            // handle the response
            match self.response_handler() {
                true => {
                    event_loop.reregister(&self.socket, Token(0), EventSet::readable(), PollOpt::level()).unwrap();
                }
                false => {
                    event_loop.reregister(&self.socket, Token(0), EventSet::writable(), PollOpt::edge()).unwrap();
                }
            }
        } else if events.is_readable() {
            // handle the request
            self.request_handler(event_loop);
        } else {
            warn!("Unknown Event, {:?}", events);
        }
    }

    fn notify(&mut self, event_loop: &mut EventLoop<UdpHandler<H, N>>, msg: EventLoopNotify) {
        match msg.notify_type {
            EventLoopNotifyType::NewResponse => {
                event_loop.reregister(&self.socket, Token(0), EventSet::writable(), PollOpt::edge()).unwrap();
            }
            EventLoopNotifyType::Shutdown => {
                info!("Shutting down request handler");
                event_loop.shutdown();
            }
            EventLoopNotifyType::UpdateResource => {
                self.observer.change_resource(&msg.request.unwrap());
            }
        }
    }

    fn timeout(&mut self, event_loop: &mut EventLoop<UdpHandler<H, N>>, token: EventLoopTimer) {
        match token {
            EventLoopTimer::ObserveTimer => {
                self.observer.timer_handler();
                event_loop.timeout_ms(EventLoopTimer::ObserveTimer, 1000).unwrap();
            }
        }
    }
}

pub struct CoAPServer {
    socket: UdpSocket,
    event_sender: Option<Sender<EventLoopNotify>>,
    event_thread: Option<thread::JoinHandle<()>>,
    worker_num: usize,
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
                            worker_num: DEFAULT_WORKER_NUM,
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
        let worker_num = self.worker_num;

        // Setup and spawn event loop thread, which will spawn
        //   children threads which handle incomining requests
        let thread = thread::spawn(move || {
            let mut event_loop = EventLoop::new().unwrap();
            event_loop.register(&socket, Token(0), EventSet::readable(), PollOpt::level()).unwrap();

            tx.send(event_loop.channel()).unwrap();

            // setup observe timer
            event_loop.timeout_ms(EventLoopTimer::ObserveTimer, 1000).unwrap();

            let event_sender = event_loop.channel();
            event_loop.run(&mut UdpHandler::new(socket, tx_send, tx_recv, worker_num, handler, move || {
                match event_sender.send(EventLoopNotify {
                                notify_type: EventLoopNotifyType::NewResponse,
                                request: None
                            }) {
                    Ok(()) => {}
                    Err(error) => {
                        warn!("Notify NewResponse failed, {:?}", error);
                    }
                }
            })).unwrap();
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
                sender.send(EventLoopNotify {
                                notify_type: EventLoopNotifyType::Shutdown,
                                request: None
                            }).unwrap();
                self.event_thread.take().map(|g| g.join().unwrap());
            }
            _ => {}
        }
    }

    /// Set the number of threads for handling requests
    pub fn set_worker_num(&mut self, worker_num: usize) {
        self.worker_num = worker_num;
    }

    /// Update the resource asynchronously, like PUT method in client
    pub fn update_resource(&mut self, path: &str, payload: Vec<u8>) -> Result<(), CoAPServerError> {
        let mut request = CoAPRequest::new();
        request.set_path(path);
        request.set_payload(payload);

        match self.event_sender {
            Some(ref event_sender) => {
                match event_sender.send(EventLoopNotify {
                    notify_type: EventLoopNotifyType::UpdateResource,
                    request: Some(request)
                }) {
                    Ok(_) => Ok(()),
                    _ => Err(CoAPServerError::EventSendError),
                }
            },
            _ => Err(CoAPServerError::EventSendError),
        }
    }

    /// Return the local address that the server is listening on. This can be useful when starting
    /// a server on a random port as part of unit testing.
    pub fn socket_addr(&self) -> std::io::Result<SocketAddr> {
        self.socket.local_addr()
    }
}


impl Drop for CoAPServer {
    fn drop(&mut self) {
        self.stop();
    }
}


#[cfg(test)]
mod test {
    use std::time::Duration;
    use super::super::*;
    use super::*;

    fn request_handler(req: CoAPRequest) -> Option<CoAPResponse> {
        let uri_path_list = req.get_option(CoAPOption::UriPath).unwrap().clone();
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
        request.set_type(MessageType::Confirmable);
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
        packet.set_type(MessageType::Confirmable);
        packet.set_code("0.01");
        packet.set_message_id(1);
        packet.add_option(CoAPOption::UriPath, b"test-echo".to_vec());
        client.send(&packet).unwrap();

        let recv_packet = client.receive().unwrap();
        assert_eq!(recv_packet.message.payload, b"test-echo".to_vec());
    }

    #[test]
    fn test_update_resource() {
        let path = "/test";
        let payload1 = b"data1".to_vec();
        let payload2 = b"data2".to_vec();
        let (tx, rx) = mpsc::channel();
        let (tx2, rx2) = mpsc::channel();
        let mut step = 1;

        let mut server = CoAPServer::new("127.0.0.1:5690").unwrap();
        server.handle(request_handler).unwrap();

        let mut client = CoAPClient::new("127.0.0.1:5690").unwrap();

        tx.send(step).unwrap();
        let mut request = CoAPRequest::new();
        request.set_method(Method::Put);
        request.set_path(path);
        request.set_payload(payload1.clone());
        client.send(&request).unwrap();
        client.receive().unwrap();

        let mut receive_step = 1;
        let payload1_clone = payload1.clone();
        let payload2_clone = payload2.clone();
        client.observe(path, move |msg| {
            match rx.try_recv() {
                Ok(n) => receive_step = n,
                _ => (),
            }

            match receive_step {
                1 => assert_eq!(msg.payload, payload1_clone),
                2 => {
                    assert_eq!(msg.payload, payload2_clone);
                    tx2.send(()).unwrap();
                }
                _ => panic!("unexpected step"),
            }
        }).unwrap();

        step = 2;
        tx.send(step).unwrap();
        server.update_resource(path, payload2.clone()).unwrap();
        assert_eq!(rx2.recv_timeout(Duration::new(5, 0)).unwrap(), ());
    }

    #[test]
    fn test_server_socket_addr() {
        let socket_addr = "127.0.0.1:5690".to_socket_addrs().unwrap().next().unwrap();
        let server = CoAPServer::new(socket_addr).unwrap();
        assert_eq!(server.socket_addr().unwrap(), socket_addr);
    }

    #[test]
    fn test_random_server_port() {
        // A port of ZERO should assign a random available port.
        let mut server = CoAPServer::new("127.0.0.1:0").unwrap();
        server.handle(request_handler).unwrap();

        let server_port = server.socket_addr().unwrap().port();
        assert_ne!(server_port, 0); // Ensure a reasonable port number was assigned.

        let client_addr = SocketAddr::from(([127, 0, 0, 1], server_port));
        let client = CoAPClient::new(client_addr).unwrap();
        let mut request = CoAPRequest::new();
        request.set_version(1);
        request.set_type(MessageType::Confirmable);
        request.set_code("0.01");
        request.set_message_id(1);
        request.set_token(vec![0x51, 0x55, 0x77, 0xE8]);
        request.add_option(CoAPOption::UriPath, b"test-echo".to_vec());
        client.send(&request).unwrap();

        let recv_packet = client.receive().unwrap();
        assert_eq!(recv_packet.message.payload, b"test-echo".to_vec());
    }
}
