use crate::observer::Observer;
#[cfg(feature = "router")]
use crate::router::{request::Request, Router};
use async_trait::async_trait;
use coap_lite::{BlockHandler, BlockHandlerConfig, CoapRequest, CoapResponse, Packet};
use log::debug;
use std::{
    future::Future,
    io::ErrorKind,
    net::{self, IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, ToSocketAddrs},
    sync::Arc,
};
use tokio::{
    io,
    net::UdpSocket,
    select,
    sync::{
        mpsc::{self, UnboundedReceiver, UnboundedSender},
        Mutex,
    },
    task::JoinHandle,
};

#[derive(Debug)]
pub enum CoAPServerError {
    NetworkError,
    EventLoopError,
    AnotherHandlerIsRunning,
    EventSendError,
}

use tokio::io::Error;

#[async_trait]
pub trait Dispatcher: Send + Sync {
    async fn dispatch(&self, request: CoapRequest<SocketAddr>) -> Option<CoapResponse>;
}

#[async_trait]
/// This trait represents a generic way to respond to a listener. If you want to implement your own
/// listener, you have to implement this trait to be able to send responses back through the
/// correct transport
pub trait Responder: Sync + Send {
    async fn respond(&self, response: Vec<u8>);
    fn address(&self) -> SocketAddr;
}

/// channel to send new requests from a transport to the CoAP server
pub type TransportRequestSender = UnboundedSender<(Vec<u8>, Arc<dyn Responder>)>;

/// channel used by CoAP server to receive new requests
pub type TransportRequestReceiver = UnboundedReceiver<(Vec<u8>, Arc<dyn Responder>)>;

type UdpResponseReceiver = UnboundedReceiver<(Vec<u8>, SocketAddr)>;
type UdpResponseSender = UnboundedSender<(Vec<u8>, SocketAddr)>;

// listeners receive new connections
#[async_trait]
pub trait Listener: Send {
    async fn listen(
        self: Box<Self>,
        sender: TransportRequestSender,
    ) -> std::io::Result<JoinHandle<std::io::Result<()>>>;
}
/// listener for a UDP socket
pub struct UdpCoapListener {
    socket: UdpSocket,
    multicast_addresses: Vec<IpAddr>,
    response_receiver: UdpResponseReceiver,
    response_sender: UdpResponseSender,
}

#[async_trait]
/// A trait for handling incoming requests. Use this instead of a closure
/// if you want to modify some external state
pub trait RequestHandler: Send + Sync + 'static {
    async fn handle_request(
        &self,
        mut request: Box<CoapRequest<SocketAddr>>,
    ) -> Box<CoapRequest<SocketAddr>>;
}

#[async_trait]
impl<F, HandlerRet> RequestHandler for F
where
    F: Fn(Box<CoapRequest<SocketAddr>>) -> HandlerRet + Send + Sync + 'static,
    HandlerRet: Future<Output = Box<CoapRequest<SocketAddr>>> + Send,
{
    async fn handle_request(
        &self,
        request: Box<CoapRequest<SocketAddr>>,
    ) -> Box<CoapRequest<SocketAddr>> {
        self(request).await
    }
}

/// A listener for UDP packets. This listener can also subscribe to multicast addresses
impl UdpCoapListener {
    pub fn new<A: ToSocketAddrs>(addr: A) -> Result<Self, Error> {
        let std_socket = net::UdpSocket::bind(addr)?;
        std_socket.set_nonblocking(true)?;
        let socket = UdpSocket::from_std(std_socket)?;
        Ok(Self::from_socket(socket))
    }

    pub fn from_socket(socket: tokio::net::UdpSocket) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self {
            socket,
            multicast_addresses: Vec::new(),
            response_receiver: rx,
            response_sender: tx,
        }
    }

    /// join multicast - adds the multicast addresses to the unicast listener
    /// - IPv4 multicast address range is '224.0.0.0/4'
    /// - IPv6 AllCoAp multicast addresses are 'ff00::/8'
    ///
    /// Parameter segment is used with IPv6 to determine the first octet.
    /// - It's value can be between 0x0 and 0xf.
    /// - To join multiple segments, you have to call enable_discovery for each of the segments.
    ///
    /// Some Multicast address scope
    /// IPv6        IPv4 equivalent[16]	        Scope	            Purpose
    /// ffx1::/16	127.0.0.0/8	                Interface-local	    Packets with this destination address may not be sent over any network link, but must remain within the current node; this is the multicast equivalent of the unicast loopback address.
    /// ffx2::/16	224.0.0.0/24	            Link-local	        Packets with this destination address may not be routed anywhere.
    /// ffx3::/16	239.255.0.0/16	            IPv4 local scope
    /// ffx4::/16	            	            Admin-local	        The smallest scope that must be administratively configured.
    /// ffx5::/16		                        Site-local	        Restricted to the local physical network.
    /// ffx8::/16	239.192.0.0/14	            Organization-local	Restricted to networks used by the organization administering the local network. (For example, these addresses might be used over VPNs; when packets for this group are routed over the public internet (where these addresses are not valid), they would have to be encapsulated in some other protocol.)
    /// ffxe::/16	224.0.1.0-238.255.255.255	Global scope	    Eligible to be routed over the public internet.
    ///
    /// Notable addresses:
    /// ff02::1	    All nodes on the local network segment
    /// ff0x::c	    Simple Service Discovery Protocol
    /// ff0x::fb	Multicast DNS
    /// ff0x::fb	Multicast CoAP
    /// ff0x::114	Used for experiments
    //    pub fn join_multicast(&mut self, addr: IpAddr) {
    //        self.udp_server.join_multicast(addr);
    //    }
    pub fn join_multicast(&mut self, addr: IpAddr) {
        assert!(addr.is_multicast());
        // determine wether IPv4 or IPv6 and
        // join the appropriate multicast address
        match self.socket.local_addr().unwrap() {
            SocketAddr::V4(val) => {
                match addr {
                    IpAddr::V4(ipv4) => {
                        let i = val.ip().clone();
                        self.socket.join_multicast_v4(ipv4, i).unwrap();
                        self.multicast_addresses.push(addr);
                    }
                    IpAddr::V6(_ipv6) => { /* handle IPv6 */ }
                }
            }
            SocketAddr::V6(_val) => {
                match addr {
                    IpAddr::V4(_ipv4) => { /* handle IPv4 */ }
                    IpAddr::V6(ipv6) => {
                        self.socket.join_multicast_v6(&ipv6, 0).unwrap();
                        self.multicast_addresses.push(addr);
                        //self.socket.set_only_v6(true)?;
                    }
                }
            }
        }
    }

    /// leave multicast - remove the multicast address from the listener
    pub fn leave_multicast(&mut self, addr: IpAddr) {
        assert!(addr.is_multicast());
        // determine wether IPv4 or IPv6 and
        // leave the appropriate multicast address
        match self.socket.local_addr().unwrap() {
            SocketAddr::V4(val) => {
                match addr {
                    IpAddr::V4(ipv4) => {
                        let i = val.ip().clone();
                        self.socket.leave_multicast_v4(ipv4, i).unwrap();
                        let index = self
                            .multicast_addresses
                            .iter()
                            .position(|&item| item == addr)
                            .unwrap();
                        self.multicast_addresses.remove(index);
                    }
                    IpAddr::V6(_ipv6) => { /* handle IPv6 */ }
                }
            }
            SocketAddr::V6(_val) => {
                match addr {
                    IpAddr::V4(_ipv4) => { /* handle IPv4 */ }
                    IpAddr::V6(ipv6) => {
                        self.socket.leave_multicast_v6(&ipv6, 0).unwrap();
                        let index = self
                            .multicast_addresses
                            .iter()
                            .position(|&item| item == addr)
                            .unwrap();
                        self.multicast_addresses.remove(index);
                    }
                }
            }
        }
    }
    /// enable AllCoAP multicasts - adds the AllCoap addresses to the listener
    /// - IPv4 AllCoAP multicast address is '224.0.1.187'
    /// - IPv6 AllCoAp multicast addresses are 'ff0?::fd'
    ///
    /// Parameter segment is used with IPv6 to determine the first octet.
    /// - It's value can be between 0x0 and 0xf.
    /// - To join multiple segments, you have to call enable_discovery for each of the segments.
    ///
    /// For further details see method join_multicast
    pub fn enable_all_coap(&mut self, segment: u8) {
        assert!(segment <= 0xf);
        let m = match self.socket.local_addr().unwrap() {
            SocketAddr::V4(_val) => IpAddr::V4(Ipv4Addr::new(224, 0, 1, 187)),
            SocketAddr::V6(_val) => IpAddr::V6(Ipv6Addr::new(
                0xff00 + segment as u16,
                0,
                0,
                0,
                0,
                0,
                0,
                0xfd,
            )),
        };
        self.join_multicast(m);
    }
}
#[async_trait]
impl Listener for UdpCoapListener {
    async fn listen(
        mut self: Box<Self>,
        sender: TransportRequestSender,
    ) -> std::io::Result<JoinHandle<std::io::Result<()>>> {
        return Ok(tokio::spawn(self.receive_loop(sender)));
    }
}

#[derive(Clone)]
struct UdpResponder {
    address: SocketAddr, // this is the address we are sending to
    tx: UdpResponseSender,
}

#[async_trait]
impl Responder for UdpResponder {
    async fn respond(&self, response: Vec<u8>) {
        let _ = self.tx.send((response, self.address));
    }
    fn address(&self) -> SocketAddr {
        self.address
    }
}

impl UdpCoapListener {
    pub async fn receive_loop(mut self, sender: TransportRequestSender) -> std::io::Result<()> {
        loop {
            let mut recv_vec = Vec::with_capacity(u16::MAX as usize);
            select! {
                message =self.socket.recv_buf_from(&mut recv_vec)=> {
                    match message {
                        Ok((_size, from)) => {
                            sender.send((recv_vec, Arc::new(UdpResponder{address: from, tx: self.response_sender.clone()}))).map_err( |_| std::io::Error::new(ErrorKind::Other, "server channel error"))?;
                        }
                        Err(e) => {
                            return Err(e);
                        }
                    }
                },
                response = self.response_receiver.recv() => {
                    if let Some((bytes, to)) = response{
                        debug!("sending {:?} to {:?}", &bytes,  &to);
                        self.socket.send_to(&bytes, to).await?;
                    }
                    else {
                        // in case nobody is listening to us, we can just terminate, though this
                        // should never happen for UDP
                        return Ok(());
                    }

                }
            }
        }
    }
}

#[derive(Debug)]
pub struct QueuedMessage {
    pub address: SocketAddr,
    pub message: Packet,
}

struct ServerCoapState {
    observer: Observer,
    block_handler: BlockHandler<SocketAddr>,
    disable_observe: bool,
}

pub enum ShouldForwardToHandler {
    True,
    False,
}

impl ServerCoapState {
    pub async fn intercept_request(
        &mut self,
        request: &mut CoapRequest<SocketAddr>,
        responder: Arc<dyn Responder>,
    ) -> ShouldForwardToHandler {
        match self.block_handler.intercept_request(request) {
            Ok(true) => return ShouldForwardToHandler::False,
            Err(_err) => return ShouldForwardToHandler::False,
            Ok(false) => {}
        };

        if self.disable_observe {
            return ShouldForwardToHandler::True;
        }

        let should_be_forwarded = self.observer.request_handler(request, responder).await;
        if should_be_forwarded {
            return ShouldForwardToHandler::True;
        } else {
            return ShouldForwardToHandler::False;
        }
    }

    pub async fn intercept_response(&mut self, request: &mut CoapRequest<SocketAddr>) {
        match self.block_handler.intercept_response(request) {
            Err(err) => {
                let _ = request.apply_from_error(err);
            }
            _ => {}
        }
    }
    pub fn new() -> Self {
        Self {
            observer: Observer::new(),
            block_handler: BlockHandler::new(BlockHandlerConfig::default()),
            disable_observe: false,
        }
    }
    pub fn disable_observe_handling(&mut self, value: bool) {
        self.disable_observe = value
    }
}

pub struct Server {
    listeners: Vec<Box<dyn Listener>>,
    coap_state: Arc<Mutex<ServerCoapState>>,
    new_packet_receiver: TransportRequestReceiver,
    new_packet_sender: TransportRequestSender,
}

impl Server {
    /// Creates a CoAP server listening on the given address.
    pub fn new_udp<A: ToSocketAddrs>(addr: A) -> Result<Self, io::Error> {
        let listener: Vec<Box<dyn Listener>> = vec![Box::new(UdpCoapListener::new(addr)?)];
        Ok(Self::from_listeners(listener))
    }

    pub fn from_listeners(listeners: Vec<Box<dyn Listener>>) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Server {
            listeners,
            coap_state: Arc::new(Mutex::new(ServerCoapState::new())),
            new_packet_receiver: rx,
            new_packet_sender: tx,
        }
    }

    async fn spawn_handles(
        listeners: Vec<Box<dyn Listener>>,
        sender: TransportRequestSender,
    ) -> std::io::Result<Vec<JoinHandle<std::io::Result<()>>>> {
        let mut handles = vec![];
        for listener in listeners.into_iter() {
            let handle = listener.listen(sender.clone()).await?;
            handles.push(handle);
        }
        return Ok(handles);
    }

    /// run the server.
    pub async fn run<Handler: RequestHandler>(mut self, handler: Handler) -> Result<(), io::Error> {
        let _handles = Self::spawn_handles(self.listeners, self.new_packet_sender.clone()).await?;

        let handler_arc = Arc::new(handler);
        // receive an input, sync our cache / states, then call custom handler
        loop {
            let (bytes, respond) =
                self.new_packet_receiver.recv().await.ok_or_else(|| {
                    std::io::Error::new(ErrorKind::Other, "listen channel closed")
                })?;
            if let Ok(packet) = Packet::from_bytes(&bytes) {
                let mut request = Box::new(CoapRequest::<SocketAddr>::from_packet(
                    packet,
                    respond.address(),
                ));
                let mut coap_state = self.coap_state.lock().await;
                let should_forward = coap_state
                    .intercept_request(&mut request, respond.clone())
                    .await;

                match should_forward {
                    ShouldForwardToHandler::True => {
                        let handler_clone = handler_arc.clone();
                        let coap_state_clone = self.coap_state.clone();
                        tokio::spawn(async move {
                            request = handler_clone.handle_request(request).await;
                            coap_state_clone
                                .lock()
                                .await
                                .intercept_response(request.as_mut())
                                .await;

                            Self::respond_to_request(request, respond).await;
                        });
                    }
                    ShouldForwardToHandler::False => {
                        Self::respond_to_request(request, respond).await;
                    }
                }
            }
        }
    }

    #[cfg(feature = "router")]
    pub async fn serve<S>(self, router: Router<S>, state: S)
    where
        S: Clone + Send + Sync + 'static,
    {
        let router = Arc::new(router);
        let handler = {
            move |req| {
                let r = router.clone();
                let s = state.clone();
                let req = Request::new(req);
                async move { r.handle(req, s).await.req }
            }
        };
        self.run(handler).await.unwrap();
    }

    async fn respond_to_request(req: Box<CoapRequest<SocketAddr>>, responder: Arc<dyn Responder>) {
        // if we have some reponse to send, send it
        if let Some(Ok(b)) = req.response.map(|resp| resp.message.to_bytes()) {
            responder.respond(b).await;
        }
    }
    #[deprecated(since = "0.21.0", note = "Use 'coap::Server::automatic_observe_handling' instead.")]
    /// disable auto-observe handling in server
    pub async fn disable_observe_handling(&mut self, value: bool) {
        self.automatic_observe_handling(value).await
    }
    /// set auto-observe handling in server, defaults to enabled
    pub async fn automatic_observe_handling(&mut self, value: bool) {
        let mut coap_state = self.coap_state.lock().await;
        coap_state.disable_observe_handling(value)
    }
}

#[cfg(test)]
pub mod test {
    use crate::request::RequestBuilder;

    use super::super::*;
    use super::*;
    use coap_lite::{block_handler::BlockValue, CoapOption, RequestType};
    use std::str;
    use std::time::Duration;

    pub fn spawn_server<
        F: Fn(Box<CoapRequest<SocketAddr>>) -> HandlerRet + Send + Sync + 'static,
        HandlerRet,
    >(
        ip: &'static str,
        request_handler: F,
    ) -> mpsc::UnboundedReceiver<u16>
    where
        HandlerRet: Future<Output = Box<CoapRequest<SocketAddr>>> + Send,
    {
        let (tx, rx) = mpsc::unbounded_channel();
        let _task = tokio::spawn(async move {
            let sock = UdpSocket::bind(ip).await.unwrap();
            let addr = sock.local_addr().unwrap();
            let listener = Box::new(UdpCoapListener::from_socket(sock));
            let server = Server::from_listeners(vec![listener]);
            tx.send(addr.port()).unwrap();
            server.run(request_handler).await.unwrap();
        });

        rx
    }

    async fn request_handler(
        mut req: Box<CoapRequest<SocketAddr>>,
    ) -> Box<CoapRequest<SocketAddr>> {
        let uri_path_list = req.message.get_option(CoapOption::UriPath).unwrap().clone();
        assert_eq!(uri_path_list.len(), 1);

        match req.response {
            Some(ref mut response) => {
                response.message.payload = uri_path_list.front().unwrap().clone();
            }
            _ => {}
        }
        return req;
    }

    pub fn spawn_server_with_all_coap<
        F: Fn(Box<CoapRequest<SocketAddr>>) -> HandlerRet + Send + Sync + 'static,
        HandlerRet,
    >(
        ip: &'static str,
        request_handler: F,
        segment: u8,
    ) -> mpsc::UnboundedReceiver<u16>
    where
        HandlerRet: Future<Output = Box<CoapRequest<SocketAddr>>> + Send,
    {
        let (tx, rx) = mpsc::unbounded_channel();

        std::thread::Builder::new()
            .name(String::from("v4-server"))
            .spawn(move || {
                tokio::runtime::Runtime::new()
                    .unwrap()
                    .block_on(async move {
                        // multicast needs a server on a real interface
                        let sock = UdpSocket::bind((ip, 0)).await.unwrap();
                        let addr = sock.local_addr().unwrap();
                        let mut listener = Box::new(UdpCoapListener::from_socket(sock));
                        listener.enable_all_coap(segment);
                        let server = Server::from_listeners(vec![listener]);
                        tx.send(addr.port()).unwrap();
                        server.run(request_handler).await.unwrap();
                    })
            })
            .unwrap();

        rx
    }

    pub fn spawn_server_disable_observe<
        F: Fn(Box<CoapRequest<SocketAddr>>) -> HandlerRet + Send + Sync + 'static,
        HandlerRet,
    >(
        ip: &'static str,
        request_handler: F,
    ) -> mpsc::UnboundedReceiver<u16>
    where
        HandlerRet: Future<Output = Box<CoapRequest<SocketAddr>>> + Send,
    {
        let (tx, rx) = mpsc::unbounded_channel();
        let _task = tokio::spawn(async move {
            let sock = UdpSocket::bind(ip).await.unwrap();
            let addr = sock.local_addr().unwrap();
            let listener = Box::new(UdpCoapListener::from_socket(sock));
            let mut server = Server::from_listeners(vec![listener]);
            server.disable_observe_handling(true).await;
            tx.send(addr.port()).unwrap();
            server.run(request_handler).await.unwrap();
        });

        rx
    }

    #[tokio::test]
    async fn test_listener_instantiation() {
        let listener = UdpCoapListener::new("127.0.0.1:0").unwrap();
        assert!(
            listener.socket.local_addr().unwrap().ip() == IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))
        );
        // assert!(listener.socket.blocking() == false);

        let explicit_socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let another_listener = UdpCoapListener::from_socket(explicit_socket);
        assert!(
            another_listener.socket.local_addr().unwrap().ip()
                == IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1))
        );
    }

    #[tokio::test]
    async fn test_echo_server() {
        let server_port = spawn_server("127.0.0.1:0", request_handler)
            .recv()
            .await
            .unwrap();

        let client = UdpCoAPClient::new_udp(format!("127.0.0.1:{}", server_port))
            .await
            .unwrap();
        let mut request = CoapRequest::new();
        request.message.header.set_version(1);
        request
            .message
            .header
            .set_type(coap_lite::MessageType::Confirmable);
        request.message.header.set_code("0.01");
        request.message.header.message_id = 1;
        request.message.set_token(vec![0x51, 0x55, 0x77, 0xE8]);
        request
            .message
            .add_option(CoapOption::UriPath, b"test-echo".to_vec());
        client.send_single_request(&request).await.unwrap();

        let recv_packet = client.send(request).await.unwrap();
        assert_eq!(recv_packet.message.payload, b"test-echo".to_vec());
    }

    #[tokio::test]
    async fn test_put_block() {
        let server_port = spawn_server("127.0.0.1:0", request_handler)
            .recv()
            .await
            .unwrap();
        let data = "hello this is a payload";
        let mut v = Vec::new();
        for _ in 0..1024 {
            v.extend_from_slice(data.as_bytes());
        }
        let payload_size = v.len();
        let server_string = format!("127.0.0.1:{}", server_port);
        let client = UdpCoAPClient::new_udp(server_string.clone()).await.unwrap();

        let request = RequestBuilder::new("/large", RequestType::Put)
            .data(Some(v))
            .domain(server_string.clone())
            .build();

        let resp = client.send(request).await.unwrap();
        let block_opt = resp
            .message
            .get_first_option_as::<BlockValue>(CoapOption::Block1)
            .expect("expected block opt in response")
            .expect("could not decode block1 option");
        let expected_number = (payload_size as f32 / 1024.0).ceil() as u16 - 1;
        assert_eq!(
            block_opt.num, expected_number,
            "block not completely received!"
        );

        assert_eq!(resp.message.payload, b"large".to_vec());
    }

    #[tokio::test]
    #[ignore]
    async fn test_echo_server_v6() {
        let server_port = spawn_server("::1:0", request_handler).recv().await.unwrap();

        let client = UdpCoAPClient::new_udp(format!("::1:{}", server_port))
            .await
            .unwrap();
        let mut request = CoapRequest::new();
        request.message.header.set_version(1);
        request
            .message
            .header
            .set_type(coap_lite::MessageType::Confirmable);
        request.message.header.set_code("0.01");
        request.message.header.message_id = 1;
        request.message.set_token(vec![0x51, 0x55, 0x77, 0xE8]);
        request
            .message
            .add_option(CoapOption::UriPath, b"test-echo".to_vec());

        let recv_packet = client.send(request).await.unwrap();
        assert_eq!(recv_packet.message.payload, b"test-echo".to_vec());
    }

    #[tokio::test]
    async fn test_echo_server_no_token() {
        let server_port = spawn_server("127.0.0.1:0", request_handler)
            .recv()
            .await
            .unwrap();

        let client = UdpCoAPClient::new_udp(format!("127.0.0.1:{}", server_port))
            .await
            .unwrap();
        let mut packet = CoapRequest::new();
        packet.message.header.set_version(1);
        packet
            .message
            .header
            .set_type(coap_lite::MessageType::Confirmable);
        packet.message.header.set_code("0.01");
        packet.message.header.message_id = 1;
        packet
            .message
            .add_option(CoapOption::UriPath, b"test-echo".to_vec());
        let recv_packet = client.send(packet).await.unwrap();
        assert_eq!(recv_packet.message.payload, b"test-echo".to_vec());
    }

    #[tokio::test]
    #[ignore]
    async fn test_echo_server_no_token_v6() {
        let server_port = spawn_server("::1:0", request_handler).recv().await.unwrap();

        let client = UdpCoAPClient::new_udp(format!("::1:{}", server_port))
            .await
            .unwrap();
        let mut packet = CoapRequest::new();
        packet.message.header.set_version(1);
        packet
            .message
            .header
            .set_type(coap_lite::MessageType::Confirmable);
        packet.message.header.set_code("0.01");
        packet.message.header.message_id = 1;
        packet
            .message
            .add_option(CoapOption::UriPath, b"test-echo".to_vec());

        let recv_packet = client.send(packet).await.unwrap();
        assert_eq!(recv_packet.message.payload, b"test-echo".to_vec());
    }

    #[tokio::test]
    async fn test_update_resource() {
        let path = "/test";
        let payload1 = b"data1".to_vec();
        let payload2 = b"data2".to_vec();
        let (tx, mut rx) = mpsc::unbounded_channel();
        let (tx2, mut rx2) = mpsc::unbounded_channel();
        let mut step = 1;

        let server_port = spawn_server("127.0.0.1:0", request_handler)
            .recv()
            .await
            .unwrap();

        let client = UdpCoAPClient::new_udp(format!("127.0.0.1:{}", server_port))
            .await
            .unwrap();

        tx.send(step).unwrap();
        let mut request = CoapRequest::new();
        request.set_method(RequestType::Put);
        request.set_path(path);
        request.message.payload = payload1.clone();
        client.send(request.clone()).await.unwrap();

        let mut receive_step = 1;
        let payload1_clone = payload1.clone();
        let payload2_clone = payload2.clone();
        client
            .observe(path, move |msg| {
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
            })
            .await
            .unwrap();

        step = 2;
        tx.send(step).unwrap();
        request.message.payload = payload2.clone();
        let client2 = UdpCoAPClient::new_udp(format!("127.0.0.1:{}", server_port))
            .await
            .unwrap();
        let _ = client2.send(request).await.unwrap();
        assert_eq!(
            tokio::time::timeout(Duration::new(5, 0), rx2.recv())
                .await
                .unwrap(),
            Some(())
        );
    }

    #[tokio::test]
    async fn test_observe_transparent_transmission() {
        let path = "/test";
        let (tx, mut rx) = mpsc::unbounded_channel();

        let server_port = spawn_server_disable_observe("127.0.0.1:0", request_handler)
            .recv()
            .await
            .unwrap();

        let client = UdpCoAPClient::new_udp(format!("127.0.0.1:{}", server_port))
            .await
            .unwrap();

        client
            .observe(path, move |msg| {
                assert_eq!(msg.payload, b"test".to_vec());
                tx.send(()).unwrap();
            })
            .await
            .unwrap();

        assert_eq!(
            tokio::time::timeout(Duration::new(5, 0), rx.recv())
                .await
                .unwrap(),
            Some(())
        );
    }

    #[tokio::test]
    async fn multicast_server_all_coap() {
        // segment not relevant with IPv4
        let segment = 0x0;
        let server_port = spawn_server_with_all_coap("0.0.0.0", request_handler, segment)
            .recv()
            .await
            .unwrap();

        let client = UdpCoAPClient::new_udp(format!("127.0.0.1:{}", server_port))
            .await
            .unwrap();
        let mut request = CoapRequest::new();
        request.message.header.set_version(1);
        request
            .message
            .header
            .set_type(coap_lite::MessageType::Confirmable);
        request.message.header.set_code("0.01");
        request.message.header.message_id = 1;
        request.message.set_token(vec![0x51, 0x55, 0x77, 0xE8]);
        request
            .message
            .add_option(CoapOption::UriPath, b"test-echo".to_vec());
        let recv_packet = client.send(request).await.unwrap();

        assert_eq!(recv_packet.message.payload, b"test-echo".to_vec());

        let client = UdpCoAPClient::new_udp(format!("224.0.1.187:{}", server_port))
            .await
            .unwrap();
        let mut request = RequestBuilder::new("test-echo", RequestType::Get)
            .data(Some(vec![0x51, 0x55, 0x77, 0xE8]))
            .confirmable(true)
            .build();

        let mut receiver = client.create_receiver_for(&request).await;
        client.send_all_coap(&mut request, segment).await.unwrap();
        let recv_packet = receiver.receive().await.unwrap();
        assert_eq!(recv_packet.message.payload, b"test-echo".to_vec());
    }

    //This test right now does not work on windows
    #[cfg(unix)]
    #[tokio::test]
    #[ignore]
    async fn multicast_server_all_coap_v6() {
        // use segment 0x04 which should be the smallest administered scope

        let segment = 0x04;
        let server_port = spawn_server_with_all_coap("::0", request_handler, segment)
            .recv()
            .await
            .unwrap();

        let client = UdpCoAPClient::new_udp(format!("::1:{}", server_port))
            .await
            .unwrap();
        let mut request = CoapRequest::new();
        request.message.header.set_version(1);
        request
            .message
            .header
            .set_type(coap_lite::MessageType::Confirmable);
        request.message.header.set_code("0.01");
        request.message.header.message_id = 1;
        request.message.set_token(vec![0x51, 0x55, 0x77, 0xE8]);
        request
            .message
            .add_option(CoapOption::UriPath, b"test-echo".to_vec());
        client.send_single_request(&request).await.unwrap();

        let recv_packet = client.send(request).await.unwrap();
        assert_eq!(recv_packet.message.payload, b"test-echo".to_vec());

        // use 0xff02 to keep it within this network
        let client = UdpCoAPClient::new_udp(format!("ff0{}::fd:{}", segment, server_port))
            .await
            .unwrap();
        let mut request = CoapRequest::new();
        request.message.header.set_version(1);
        request
            .message
            .header
            .set_type(coap_lite::MessageType::NonConfirmable);
        request.message.header.set_code("0.01");
        request.message.header.message_id = 2;
        request.message.set_token(vec![0x51, 0x55, 0x77, 0xE8]);
        request
            .message
            .add_option(CoapOption::UriPath, b"test-echo".to_vec());
        let mut receiver = client.create_receiver_for(&request).await;
        client.send_all_coap(&mut request, segment).await.unwrap();
        let recv_packet = receiver.receive().await.unwrap();
        assert_eq!(recv_packet.message.payload, b"test-echo".to_vec());
    }

    #[test]
    fn multicast_join_leave() {
        std::thread::Builder::new()
            .name(String::from("v4-server"))
            .spawn(move || {
                tokio::runtime::Runtime::new()
                    .unwrap()
                    .block_on(async move {
                        // multicast needs a server on a real interface
                        let sock = UdpSocket::bind(("0.0.0.0", 0)).await.unwrap();
                        let mut listener = Box::new(UdpCoapListener::from_socket(sock));
                        listener.join_multicast(IpAddr::V4(Ipv4Addr::new(224, 0, 1, 1)));
                        listener.join_multicast(IpAddr::V4(Ipv4Addr::new(224, 1, 1, 1)));
                        listener.leave_multicast(IpAddr::V4(Ipv4Addr::new(224, 0, 1, 1)));
                        listener.leave_multicast(IpAddr::V4(Ipv4Addr::new(224, 1, 1, 1)));
                        let server = Server::from_listeners(vec![listener]);
                        server.run(request_handler).await.unwrap();
                    })
            })
            .unwrap();

        std::thread::sleep(std::time::Duration::from_secs(1));
    }
    #[test]
    #[ignore]
    fn multicast_join_leave_v6() {
        std::thread::Builder::new()
            .name(String::from("v6-server"))
            .spawn(move || {
                tokio::runtime::Runtime::new()
                    .unwrap()
                    .block_on(async move {
                        // multicast needs a server on a real interface
                        let sock = UdpSocket::bind(("0.0.0.0", 0)).await.unwrap();
                        let mut listener = Box::new(UdpCoapListener::from_socket(sock));
                        listener.join_multicast(IpAddr::V6(Ipv6Addr::new(
                            0xff02, 0, 0, 0, 0, 0, 1, 0x1,
                        )));
                        listener.join_multicast(IpAddr::V6(Ipv6Addr::new(
                            0xff02, 0, 0, 0, 0, 1, 0, 0x2,
                        )));
                        listener.leave_multicast(IpAddr::V6(Ipv6Addr::new(
                            0xff02, 0, 0, 0, 0, 0, 1, 0x1,
                        )));
                        listener.join_multicast(IpAddr::V6(Ipv6Addr::new(
                            0xff02, 0, 0, 0, 0, 1, 0, 0x2,
                        )));
                        let server = Server::from_listeners(vec![listener]);
                        server.run(request_handler).await.unwrap();
                    })
            })
            .unwrap();

        std::thread::sleep(std::time::Duration::from_secs(1));
    }

    fn get_expected_response() -> Vec<u8> {
        let mut resp = vec![];
        for c in b'a'..=b'z' {
            resp.extend(std::iter::repeat(c).take(1024));
        }
        resp
    }
    async fn block2_responder(
        mut req: Box<CoapRequest<SocketAddr>>,
    ) -> Box<CoapRequest<SocketAddr>> {
        // vec should contain 'a' 1024 times, then 'b' 1024, up to ascii 'z'

        match req.response {
            Some(ref mut response) => {
                response.message.payload = get_expected_response();
            }
            _ => {}
        }
        return req;
    }
    #[tokio::test]
    async fn test_block2_server_response() {
        let server_port = spawn_server("127.0.0.1:0", block2_responder)
            .recv()
            .await
            .unwrap();

        let client = UdpCoAPClient::new_udp(format!("127.0.0.1:{}", server_port))
            .await
            .unwrap();
        let resp = client
            .send(RequestBuilder::new("/", RequestType::Get).build())
            .await
            .unwrap();
        assert_eq!(
            resp.message.payload,
            get_expected_response(),
            "responses do not match"
        );
    }
}
