use std::{
    self,
    pin::Pin,
    net::{self, SocketAddr, ToSocketAddrs},
    task::Context,
    future::Future,
    time::Duration,
};
use log::{debug, error};
use futures::{SinkExt, Stream, StreamExt, select, stream::FusedStream, task::Poll};
use tokio::{
    io,
    sync::mpsc,
    net::UdpSocket
};
use tokio_util::udp::{UdpFramed};
use openssl::{
    error::ErrorStack,
    sha::Sha256,
    ssl::{self, HandshakeError, Ssl, SslContextBuilder, SslMethod, SslStream, SslVerifyMode}
};

use super::message::{
    packet::Packet,
    request::{CoAPRequest},
    response::CoAPResponse,
    Codec,
};
use super::observer::Observer;

pub type MessageSender = mpsc::UnboundedSender<(Packet, SocketAddr)>;
type MessageReceiver = mpsc::UnboundedReceiver<(Packet, SocketAddr)>;

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

pub enum Message {
    NeedSend(Packet, SocketAddr),
    Received(Packet, SocketAddr),
}

pub enum Encryption {
    DTLS,
    TLS,
    SSL,
}

pub struct Server<'a, HandlerRet> where HandlerRet: Future<Output=Option<CoAPResponse>> {
    server: CoAPServer,
    observer: Observer,
    handler: Option<Box<dyn FnMut(CoAPRequest) -> HandlerRet + Send + 'a>>,
}

impl<'a, HandlerRet> Server<'a, HandlerRet> where HandlerRet: Future<Output=Option<CoAPResponse>> {
    /// Creates a CoAP server listening on the given address.
    pub fn new<A: ToSocketAddrs + Copy>(addr: A, method: Option<Encryption>) -> Result<Server<'a, HandlerRet>, io::Error> {
        let (tx, rx) = mpsc::unbounded_channel();
        Ok(Server {
            server: CoAPServer::new(addr, rx, method.unwrap())?,
            observer: Observer::new(tx),
            handler: None,
        })
    }

    /// run the server.
    pub async fn run<F: FnMut(CoAPRequest) -> HandlerRet + Send + 'a>(&mut self, handler: F) -> Result<(), io::Error> {
        self.handler = Some(Box::new(handler));

        loop {
            select! {
                message = self.server.select_next_some() => {
                    match message {
                        Ok(Message::NeedSend(packet, addr)) => {
                            self.server.send((packet, addr)).await?;
                        }
                        Ok(Message::Received(packet, addr)) => {
                            self.dispatch_msg(packet, addr).await?;
                        }
                        Err(e) => {
                            error!("select error: {:?}", e);
                        }
                    }
                }
                _ = self.observer.select_next_some() => {
                    self.observer.timer_handler().await;
                }
                complete => break,
            }
        }
        Ok(())
    }

    /// Return the local address that the server is listening on. This can be useful when starting
    /// a server on a random port as part of unit testing.
    pub fn socket_addr(&self) -> std::io::Result<SocketAddr> {
        self.server.socket_addr()
    }

    async fn dispatch_msg(&mut self, packet: Packet, addr: SocketAddr) -> Result<(), io::Error> {
        let request = CoAPRequest::from_packet(packet, &addr);
        let filtered = !self.observer.request_handler(&request).await;
        if filtered {
            return Ok(());
        }

        if let Some(ref mut handler) = self.handler {
            match handler(request).await {
                Some(response) => {
                    debug!("Response: {:?}", response);
                    self.server.send((response.message, addr)).await?;
                }
                None => {
                    debug!("No response");
                }
            }
        }
        Ok(())
    }
}

#[derive(Debug)]
struct UdpStream {
    socket: std::net::UdpSocket,
    connected: bool,
}

impl std::io::Read for UdpStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.connected {
            self.socket.recv(buf)
        } else {
            match self.socket.recv_from(buf) {
                Ok((bytes, addr)) => {
                    self.connect(addr)?;
                    Ok(bytes)
                }
                Err(e) => Err(e),
            }
        }
    }

}

impl std::io::Write for UdpStream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.socket.send(buf)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl UdpStream {
    /// Binds to the given address and sets timeouts to reasonable values.
    pub fn new<A: ToSocketAddrs>(addr: A) -> std::io::Result<UdpStream> {
        let socket = std::net::UdpSocket::bind(addr).unwrap();
        let duration = Duration::from_millis(300);
        socket.set_read_timeout(Some(duration))?;
        socket.set_write_timeout(Some(duration))?;
        Ok(UdpStream {
            socket: socket,
            connected: false,
        })
    }

    /// Connects the UDP socket to the given address.
    pub fn connect<A: ToSocketAddrs + Copy>(&mut self, addr: A) -> std::io::Result<()> {
        self.socket.connect(addr)?;
        self.connected = true;
        Ok(())
    }
}

pub struct CoAPServer {
    receiver: MessageReceiver,
    is_terminated: bool,
    socket: UdpFramed<Codec>,
    ssl_stream: Option<Result<SslStream<UdpStream>, HandshakeError<UdpStream>>>,
}

impl CoAPServer {
    /// Creates a CoAP server listening on the given address.
    pub fn new<A: ToSocketAddrs + Copy>(addr: A, receiver: MessageReceiver, method: Encryption) -> Result<CoAPServer, io::Error> {
        let socket = UdpSocket::from_std(net::UdpSocket::bind(addr).unwrap())?;
        Ok(CoAPServer {
            receiver,
            is_terminated: false,
            socket: UdpFramed::new(socket, Codec::new()),
            ssl_stream: match method {
                Encryption::DTLS => {
                    let mut context_builder = SslContextBuilder::new(SslMethod::dtls()).unwrap();
                    context_builder.set_verify(SslVerifyMode::PEER | SslVerifyMode::FAIL_IF_NO_PEER_CERT);
                    let ssl_context = context_builder.build();
                    let ssl = Ssl::new(&ssl_context).unwrap();
                    let socket = UdpStream::new(addr).unwrap();
                    match ssl.accept(socket) {
                        Ok(stream) => Some(Ok(stream)),
                        Err(e) => Some(Err(e))
                    }
                }
                Encryption::TLS => None,
                Encryption::SSL => None
            },
        })
    }

    /// Stop the server.
    pub fn stop(&mut self) {
        self.is_terminated = true;
    }

    /// send the packet to the specific address.
    pub async fn send(&mut self, frame: (Packet, SocketAddr)) -> Result<(), io::Error> {
        self.socket.send(frame).await
    }

    /// Return the local address that the server is listening on. This can be useful when starting
    /// a server on a random port as part of unit testing.
    pub fn socket_addr(&self) -> std::io::Result<SocketAddr> {
        self.socket.get_ref().local_addr()
    }
}


impl Drop for CoAPServer {
    fn drop(&mut self) {
        self.stop();
    }
}

impl Stream for CoAPServer {
    type Item = Result<Message, io::Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if let Poll::Ready(Some((p, a))) = self.receiver.poll_next_unpin(cx) {
            return Poll::Ready(Some(Ok(Message::NeedSend(p, a))));
        }

        let result: Option<_> = futures::ready!(self.socket.poll_next_unpin(cx));

        Poll::Ready(match result {
            Some(Ok(message)) => {
                let (my_packet, addr) = message;
                Some(Ok(Message::Received(my_packet, addr)))
            }
            Some(Err(e)) => Some(Err(e)),
            None => None,
        })
    }
}

impl FusedStream for CoAPServer {
    fn is_terminated(&self) -> bool {
        self.is_terminated
    }
}

#[cfg(test)]
pub mod test {
    use std::{
        time::Duration,
        sync::mpsc,
    };
    use super::super::*;
    use super::*;

    fn spawn_server<F: FnMut(CoAPRequest) -> HandlerRet + Send + 'static, HandlerRet>(request_handler: F, method: Encryption) -> mpsc::Receiver<u16>  where HandlerRet: Future<Output=Option<CoAPResponse>> {
        let (tx, rx) = mpsc::channel();

        std::thread::Builder::new().name(String::from("server")).spawn(move || {
            tokio::runtime::Runtime::new().unwrap().block_on(async move {
                let mut server = server::Server::new("127.0.0.1:0", Some(method)).unwrap();

                tx.send(server.socket_addr().unwrap().port()).unwrap();
                
                server.run(request_handler).await.unwrap();
            })
        }).unwrap();
        
        rx
    }

    pub fn spawn_udp(request_handler: F) {
        spawn_server(request_handler, Encryption::DTLS);
    }

    pub fn spawn_dtls_over_udp(request_handler: F) {
        spawn_server(request_handler, None);
    }
    
    async fn request_handler(req: CoAPRequest) -> Option<CoAPResponse> {
        let uri_path_list = req.get_option(CoAPOption::UriPath).unwrap().clone();
        assert_eq!(uri_path_list.len(), 1);

        match req.response {
            Some(mut response) => {
                response.set_payload(uri_path_list.front().unwrap().clone());
                Some(response)
            }
            _ => None,
        }
    }
}

    #[test]
    fn test_echo_server() {
        let server_port = spawn_server(request_handler).recv().unwrap();

        let client = CoAPClient::new(format!("127.0.0.1:{}", server_port)).unwrap();
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
        let server_port = spawn_server(request_handler).recv().unwrap();

        let client = CoAPClient::new(format!("127.0.0.1:{}", server_port)).unwrap();
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

        let server_port = spawn_server(request_handler).recv().unwrap();

        let mut client = CoAPClient::new(format!("127.0.0.1:{}", server_port)).unwrap();

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
        request.set_payload(payload2.clone());
        let client2 = CoAPClient::new(format!("127.0.0.1:{}", server_port)).unwrap();
        client2.send(&request).unwrap();
        client2.receive().unwrap();
        assert_eq!(rx2.recv_timeout(Duration::new(5, 0)).unwrap(), ());
    }
