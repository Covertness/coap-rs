use std::{
    self,
    pin::Pin,
    net::{self, SocketAddr, ToSocketAddrs, Ipv4Addr, Ipv6Addr},
    task::Context,
    future::Future,
};
use log::{debug, error};
use futures::{SinkExt, Stream, StreamExt, select, stream::FusedStream, task::Poll};
use tokio::{
    io,
    sync::mpsc,
    net::UdpSocket,
};
use tokio_util::udp::{UdpFramed};
use coap_lite::{
    Packet, CoapRequest, CoapResponse,
};

use super::message::Codec;
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

pub struct Server<'a, HandlerRet> where HandlerRet: Future<Output=Option<CoapResponse>> {
    server: CoAPServer,
    observer: Observer,
    handler: Option<Box<dyn FnMut(CoapRequest<SocketAddr>) -> HandlerRet + Send + 'a>>,
}

impl<'a, HandlerRet> Server<'a, HandlerRet> where HandlerRet: Future<Output=Option<CoapResponse>> {
    /// Creates a CoAP server listening on the given address.
    pub fn new<A: ToSocketAddrs>(addr: A) -> Result<Server<'a, HandlerRet>, io::Error> {
        let (tx, rx) = mpsc::unbounded_channel();
        Ok(Server {
            server: CoAPServer::new(addr, rx)?,
            observer: Observer::new(tx),
            handler: None,
        })
    }

    /// run the server.
    pub async fn run<F: FnMut(CoapRequest<SocketAddr>) -> HandlerRet + Send + 'a>(&mut self, handler: F) -> Result<(), io::Error> {
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
        let request = CoapRequest::from_packet(packet, addr);
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

    /// enable discovery - starts a multicast listener in addition to the unicast listener
    /// - IPv4 AllCoAP multicast address is '224.0.1.187'
    /// - IPv6 AllCoAp multicast addresses are 'ff0?::fd'
    /// Parameter segment is used with IPv6 to determine the first octet. 
    /// It's value can be between 0x0 and 0xf. To join multiple segments,
    /// you have to call enable_discovery for each of the segments.
    pub fn enable_discovery(&mut self, segment: u8) {
        assert!(segment <= 0xf);
        let socket = self.server.socket.get_mut();
        // determine wether IPv4 or IPv6 and 
        // join the appropriate AllCoAP multicast address
        match socket.local_addr().unwrap() {
            SocketAddr::V4(val) => {
                let m = Ipv4Addr::new(224, 0, 1, 187);
                let i = val.ip().clone();
                socket.join_multicast_v4(m, i).unwrap();
            },
            SocketAddr::V6(_val) => {
                let m = Ipv6Addr::new(0xff00 + segment as u16,0,0,0,0,0,0,0xfd);
                socket.join_multicast_v6(&m, 0).unwrap();
                //socket.set_only_v6(true)?;
            },
        }
    }
}

pub struct CoAPServer {
    receiver: MessageReceiver,
    is_terminated: bool,
    socket: UdpFramed<Codec>,
}

impl CoAPServer {
    /// Creates a CoAP server listening on the given address.
    pub fn new<A: ToSocketAddrs>(addr: A, receiver: MessageReceiver) -> Result<CoAPServer, io::Error> {
        let socket = UdpSocket::from_std(net::UdpSocket::bind(addr).unwrap())?;

        Ok(CoAPServer {
            receiver,
            is_terminated: false,
            socket: UdpFramed::new(socket, Codec::new()),
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
    use coap_lite::CoapOption;
    use super::super::*;
    use super::*;

    pub fn spawn_server<F: FnMut(CoapRequest<SocketAddr>) -> HandlerRet + Send + 'static, HandlerRet>(request_handler: F) -> mpsc::Receiver<u16>  where HandlerRet: Future<Output=Option<CoapResponse>> {
        let (tx, rx) = mpsc::channel();

        std::thread::Builder::new().name(String::from("server")).spawn(move || {
            tokio::runtime::Runtime::new().unwrap().block_on(async move {
                let mut server = server::Server::new("127.0.0.1:0").unwrap();

                tx.send(server.socket_addr().unwrap().port()).unwrap();
                
                server.run(request_handler).await.unwrap();
            })
        }).unwrap();
        
        rx
    }
    
    async fn request_handler(req: CoapRequest<SocketAddr>) -> Option<CoapResponse> {
        let uri_path_list = req.message.get_option(CoapOption::UriPath).unwrap().clone();
        assert_eq!(uri_path_list.len(), 1);
        
        match req.response {
            Some(mut response) => {
                response.message.payload = uri_path_list.front().unwrap().clone();
                Some(response)
            }
            _ => None,
        }
    }

    pub fn spawn_v4_server_with_discovery<F: FnMut(CoapRequest<SocketAddr>) -> HandlerRet + Send + 'static, HandlerRet>(request_handler: F) -> mpsc::Receiver<u16>  where HandlerRet: Future<Output=Option<CoapResponse>> {
        let (tx, rx) = mpsc::channel();

        std::thread::Builder::new().name(String::from("v4-server")).spawn(move || {
            tokio::runtime::Runtime::new().unwrap().block_on(async move {
                let mut server = server::Server::new(("0.0.0.0", 0)).unwrap();
                server.enable_discovery(0x0);

                tx.send(server.socket_addr().unwrap().port()).unwrap();
                
                server.run(request_handler).await.unwrap();
            })
        }).unwrap();
        
        rx
    }

    pub fn spawn_v6_server_with_discovery<F: FnMut(CoapRequest<SocketAddr>) -> HandlerRet + Send + 'static, HandlerRet>(request_handler: F) -> mpsc::Receiver<u16>  where HandlerRet: Future<Output=Option<CoapResponse>> {
        let (tx, rx) = mpsc::channel();

        std::thread::Builder::new().name(String::from("v6-server")).spawn(move || {
            tokio::runtime::Runtime::new().unwrap().block_on(async move {
                let mut server = server::Server::new(("::0", 0)).unwrap();
                server.enable_discovery(0x0);
                server.enable_discovery(0x1);
                server.enable_discovery(0x2);
                server.enable_discovery(0x3);
                server.enable_discovery(0x4);
                server.enable_discovery(0x5);
                server.enable_discovery(0x6);
                server.enable_discovery(0x7);
                server.enable_discovery(0x8);
                server.enable_discovery(0x9);
                server.enable_discovery(0xa);
                server.enable_discovery(0xb);
                server.enable_discovery(0xc);
                server.enable_discovery(0xd);
                server.enable_discovery(0xe);
                server.enable_discovery(0xf);

                tx.send(server.socket_addr().unwrap().port()).unwrap();
                
                server.run(request_handler).await.unwrap();
            })
        }).unwrap();
        
        rx
    }

    #[test]
    fn test_echo_server() {
        let server_port = spawn_server(request_handler).recv().unwrap();

        let client = CoAPClient::new(format!("127.0.0.1:{}", server_port)).unwrap();
        let mut request = CoapRequest::new();
        request.message.header.set_version(1);
        request.message.header.set_type(coap_lite::MessageType::Confirmable);
        request.message.header.set_code("0.01");
        request.message.header.message_id = 1;
        request.message.set_token(vec![0x51, 0x55, 0x77, 0xE8]);
        request.message.add_option(CoapOption::UriPath, b"test-echo".to_vec());
        client.send(&request).unwrap();

        let recv_packet = client.receive().unwrap();
        assert_eq!(recv_packet.message.payload, b"test-echo".to_vec());
    }

    #[test]
    fn test_echo_server_no_token() {
        let server_port = spawn_server(request_handler).recv().unwrap();

        let client = CoAPClient::new(format!("127.0.0.1:{}", server_port)).unwrap();
        let mut packet = CoapRequest::new();
        packet.message.header.set_version(1);
        packet.message.header.set_type(coap_lite::MessageType::Confirmable);
        packet.message.header.set_code("0.01");
        packet.message.header.message_id = 1;
        packet.message.add_option(CoapOption::UriPath, b"test-echo".to_vec());
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
        let mut request = CoapRequest::new();
        request.set_method(coap_lite::RequestType::Put);
        request.set_path(path);
        request.message.payload = payload1.clone();
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
        request.message.payload = payload2.clone();
        let client2 = CoAPClient::new(format!("127.0.0.1:{}", server_port)).unwrap();
        client2.send(&request).unwrap();
        client2.receive().unwrap();
        assert_eq!(rx2.recv_timeout(Duration::new(5, 0)).unwrap(), ());
    }

    #[test]
    fn test_server_discovery_v4() {
        let server_port = spawn_v4_server_with_discovery(request_handler).recv().unwrap();

        let client = CoAPClient::new(format!("127.0.0.1:{}", server_port)).unwrap();
        let mut request = CoapRequest::new();
        request.message.header.set_version(1);
        request.message.header.set_type(coap_lite::MessageType::Confirmable);
        request.message.header.set_code("0.01");
        request.message.header.message_id = 1;
        request.message.set_token(vec![0x51, 0x55, 0x77, 0xE8]);
        request.message.add_option(CoapOption::UriPath, b"test-echo".to_vec());
        client.send(&request).unwrap();

        let recv_packet = client.receive().unwrap();
        assert_eq!(recv_packet.message.payload, b"test-echo".to_vec());

        let client = CoAPClient::new(format!("224.0.1.187:{}", server_port)).unwrap();
        let mut request = CoapRequest::new();
        request.message.header.set_version(1);
        request.message.header.set_type(coap_lite::MessageType::NonConfirmable);
        request.message.header.set_code("0.01");
        request.message.header.message_id = 2;
        request.message.set_token(vec![0x51, 0x55, 0x77, 0xE8]);
        request.message.add_option(CoapOption::UriPath, b"test-echo".to_vec());
        client.send_all_coap(&request, 0).unwrap();

        let recv_packet = client.receive().unwrap();
        assert_eq!(recv_packet.message.payload, b"test-echo".to_vec());
    }

    #[test]
    fn test_server_discovery_v6() {
        let server_port = spawn_v6_server_with_discovery(request_handler).recv().unwrap();

        let client = CoAPClient::new(format!("::1:{}", server_port)).unwrap();
        let mut request = CoapRequest::new();
        request.message.header.set_version(1);
        request.message.header.set_type(coap_lite::MessageType::Confirmable);
        request.message.header.set_code("0.01");
        request.message.header.message_id = 1;
        request.message.set_token(vec![0x51, 0x55, 0x77, 0xE8]);
        request.message.add_option(CoapOption::UriPath, b"test-echo".to_vec());
        client.send(&request).unwrap();

        let recv_packet = client.receive().unwrap();
        assert_eq!(recv_packet.message.payload, b"test-echo".to_vec());

        let client = CoAPClient::new(format!("::0:{}", server_port)).unwrap();
        let mut request = CoapRequest::new();
        request.message.header.set_version(1);
        request.message.header.set_type(coap_lite::MessageType::NonConfirmable);
        request.message.header.set_code("0.01");
        request.message.header.message_id = 2;
        request.message.set_token(vec![0x51, 0x55, 0x77, 0xE8]);
        request.message.add_option(CoapOption::UriPath, b"test-echo".to_vec());
        client.send_all_coap(&request, 0x0).unwrap();

        let recv_packet = client.receive().unwrap();
        assert_eq!(recv_packet.message.payload, b"test-echo".to_vec());
    }

}
