#[cfg(feature = "dtls")]
use crate::dtls::{DtlsConfig, DtlsConnection};
use alloc::string::String;
use alloc::vec::Vec;
use coap_lite::{
    block_handler::{extending_splice, BlockValue, RequestCacheKey},
    error::HandlingError,
    CoapOption, CoapRequest, CoapResponse, ObserveOption, Packet, RequestType as Method,
    ResponseType as Status,
};
use core::mem;
use core::ops::Deref;
use futures::Future;
use log::*;
use lru_time_cache::LruCache;
use regex::Regex;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::time::Duration;
use std::{
    io::{Error, ErrorKind, Result},
    pin::Pin,
};
use tokio::net::{lookup_host, ToSocketAddrs, UdpSocket};
use tokio::sync::oneshot;
use tokio::time::timeout;
use url::Url;
const DEFAULT_RECEIVE_TIMEOUT_SECONDS: u64 = 2; // 2s

#[derive(Debug)]
pub enum ObserveMessage {
    Terminate,
}
use async_trait::async_trait;

#[async_trait]
pub trait Transport: Send {
    async fn recv(&self, buf: &mut [u8]) -> std::io::Result<(usize, SocketAddr)>;
    async fn send(&self, buf: &[u8]) -> std::io::Result<usize>;
}

pub struct UdpTransport {
    pub socket: UdpSocket,
    pub peer_addr: SocketAddr,
}
#[async_trait]
impl Transport for UdpTransport {
    async fn recv(&self, buf: &mut [u8]) -> std::io::Result<(usize, SocketAddr)> {
        self.socket.recv_from(buf).await
    }
    async fn send(&self, buf: &[u8]) -> std::io::Result<usize> {
        self.socket.send_to(buf, self.peer_addr).await
    }
}

/// A CoAP client over UDP. This client can send multicast and broadcasts
pub type UdpCoAPClient = CoAPClient<UdpTransport>;

pub struct CoAPClient<T: Transport> {
    transport: T,
    block2_states: LruCache<RequestCacheKey<SocketAddr>, BlockState>,
    block1_size: usize,
    message_id: u16,
    read_timeout: Option<Duration>,
}

impl UdpCoAPClient {
    pub async fn new_with_specific_source<A: ToSocketAddrs, B: ToSocketAddrs>(
        bind_addr: A,
        peer_addr: B,
    ) -> Result<Self> {
        let peer_addr = lookup_host(peer_addr).await?.next().ok_or(Error::new(
            ErrorKind::InvalidInput,
            "could not get socket address",
        ))?;
        let socket = UdpSocket::bind(bind_addr).await?;
        let transport = UdpTransport { socket, peer_addr };
        return Ok(UdpCoAPClient::from_transport(transport));
    }

    pub async fn new_udp<A: ToSocketAddrs>(addr: A) -> Result<Self> {
        let sock_addr = lookup_host(addr).await?.next().ok_or(Error::new(
            ErrorKind::InvalidInput,
            "could not get socket address",
        ))?;
        Ok(match &sock_addr {
            SocketAddr::V4(_) => Self::new_with_specific_source("0.0.0.0:0", sock_addr).await?,
            SocketAddr::V6(_) => Self::new_with_specific_source(":::0", sock_addr).await?,
        })
    }
    /// Send a request to all CoAP devices.
    /// - IPv4 AllCoAP multicast address is '224.0.1.187'
    /// - IPv6 AllCoAp multicast addresses are 'ff0?::fd'
    /// Parameter segment is used with IPv6 to determine the first octet.
    /// It's value can be between 0x0 and 0xf. To address multiple segments,
    /// you have to call send_all_coap for each of the segments.
    pub async fn send_all_coap(
        &self,
        request: &CoapRequest<SocketAddr>,
        segment: u8,
    ) -> Result<()> {
        assert!(segment <= 0xf);
        let addr = match self.transport.peer_addr {
            SocketAddr::V4(val) => {
                SocketAddr::new(IpAddr::V4(Ipv4Addr::new(224, 0, 1, 187)), val.port())
            }
            SocketAddr::V6(val) => SocketAddr::new(
                IpAddr::V6(Ipv6Addr::new(
                    0xff00 + segment as u16,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0,
                    0xfd,
                )),
                val.port(),
            ),
        };

        match request.message.to_bytes() {
            Ok(bytes) => {
                let size = self.transport.socket.send_to(&bytes[..], addr).await?;
                if size == bytes.len() {
                    Ok(())
                } else {
                    Err(Error::new(ErrorKind::Other, "send length error"))
                }
            }
            Err(_) => Err(Error::new(ErrorKind::InvalidInput, "packet error")),
        }
    }

    pub fn set_broadcast(&self, value: bool) -> Result<()> {
        self.transport.socket.set_broadcast(value)
    }
}

#[cfg(feature = "dtls")]
impl CoAPClient<DtlsConnection> {
    pub async fn from_dtls_config(config: DtlsConfig) -> Result<Self> {
        Ok(CoAPClient::from_transport(
            DtlsConnection::try_new(config).await?,
        ))
    }
}

impl<T: Transport> CoAPClient<T> {
    const MAX_PAYLOAD_BLOCK: usize = 1024;
    /// Create a CoAP client with a chosen transport type

    pub fn from_transport(transport: T) -> Self {
        CoAPClient {
            transport,
            block2_states: LruCache::with_expiry_duration(Duration::from_secs(120)),
            block1_size: Self::MAX_PAYLOAD_BLOCK,
            message_id: 0,
            read_timeout: Some(Duration::new(DEFAULT_RECEIVE_TIMEOUT_SECONDS, 0)),
        }
    }
    /// Execute a single get request with a coap url
    pub async fn get(url: &str) -> Result<CoapResponse> {
        Self::request(url, Method::Get, None).await
    }

    /// Execute a single get request with a coap url and a specific timeout.
    pub async fn get_with_timeout(url: &str, timeout: Duration) -> Result<CoapResponse> {
        Self::request_with_timeout(url, Method::Get, None, timeout).await
    }

    /// Execute a single post request with a coap url using udp
    pub async fn post(url: &str, data: Vec<u8>) -> Result<CoapResponse> {
        Self::request(url, Method::Post, Some(data)).await
    }

    /// Execute a single post request with a coap url using udp
    pub async fn post_with_timeout(
        url: &str,
        data: Vec<u8>,
        timeout: Duration,
    ) -> Result<CoapResponse> {
        Self::request_with_timeout(url, Method::Post, Some(data), timeout).await
    }

    /// Execute a put request with a coap url using udp
    pub async fn put(url: &str, data: Vec<u8>) -> Result<CoapResponse> {
        Self::request(url, Method::Put, Some(data)).await
    }

    /// Execute a single put request with a coap url using udp
    pub async fn put_with_timeout(
        url: &str,
        data: Vec<u8>,
        timeout: Duration,
    ) -> Result<CoapResponse> {
        Self::request_with_timeout(url, Method::Put, Some(data), timeout).await
    }

    /// Execute a single delete request with a coap url using udp
    pub async fn delete(url: &str) -> Result<CoapResponse> {
        Self::request(url, Method::Delete, None).await
    }

    /// Execute a single delete request with a coap url using udp
    pub async fn delete_with_timeout(url: &str, timeout: Duration) -> Result<CoapResponse> {
        Self::request_with_timeout(url, Method::Delete, None, timeout).await
    }

    /// Execute a single request (GET, POST, PUT, DELETE) with a coap url using udp
    pub async fn request(url: &str, method: Method, data: Option<Vec<u8>>) -> Result<CoapResponse> {
        let (domain, port, path, queries) = Self::parse_coap_url(url)?;
        let mut client = UdpCoAPClient::new_udp((domain.as_str(), port)).await?;
        client
            .request_path(&path, method, data, queries, Some(domain))
            .await
    }

    /// Execute a single request (GET, POST, PUT, DELETE) with a coap url and a specfic timeout
    /// using udp   
    pub async fn request_with_timeout(
        url: &str,
        method: Method,
        data: Option<Vec<u8>>,
        timeout: Duration,
    ) -> Result<CoapResponse> {
        let (domain, port, path, queries) = Self::parse_coap_url(url)?;
        let mut client = UdpCoAPClient::new_udp((domain.as_str(), port)).await?;
        client
            .request_path_with_timeout(&path, method, data, queries, Some(domain), timeout)
            .await
    }

    /// Execute a request (GET, POST, PUT, DELETE) using the given transport
    pub async fn request_path(
        &mut self,
        path: &str,
        method: Method,
        data: Option<Vec<u8>>,
        queries: Option<Vec<u8>>,
        domain: Option<String>,
    ) -> Result<CoapResponse> {
        self.request_path_with_timeout(
            path,
            method,
            data,
            queries,
            domain,
            Duration::new(DEFAULT_RECEIVE_TIMEOUT_SECONDS, 0),
        )
        .await
    }

    /// Execute a request (GET, POST, PUT, DELETE) with a specfic timeout using the given transport. This method will
    /// try to use block1 requests with a block size of 1024 by default.
    pub async fn request_path_with_timeout(
        &mut self,
        path: &str,
        method: Method,
        data: Option<Vec<u8>>,
        queries: Option<Vec<u8>>,
        domain: Option<String>,
        timeout: Duration,
    ) -> Result<CoapResponse> {
        let mut request = CoapRequest::new();
        request.set_method(method);
        request.set_path(path);
        if let Some(q) = queries {
            request.message.add_option(CoapOption::UriQuery, q);
        }
        if let Some(d) = domain {
            request
                .message
                .add_option(CoapOption::UriHost, d.as_str().as_bytes().to_vec());
        }
        request.message.header.message_id = Self::gen_message_id(&mut self.message_id);

        match data {
            Some(data) => request.message.payload = data,
            None => (),
        }

        self.set_receive_timeout(Some(timeout))?;
        self.send2(&mut request).await?;
        self.receive2(&mut request).await
    }

    pub async fn observe<H: FnMut(Packet) + Send + 'static>(
        self,
        resource_path: &str,
        handler: H,
    ) -> Result<oneshot::Sender<ObserveMessage>>
    where
        T: 'static + Send + Sync,
    {
        self.observe_with_timeout(
            resource_path,
            handler,
            Duration::new(DEFAULT_RECEIVE_TIMEOUT_SECONDS, 0),
        )
        .await
    }

    /// Observe a resource with the handler and specified timeout using the given transport.
    /// Use the oneshot sender to cancel observation. If this sender is dropped without explicitly
    /// cancelling it, the observation will continue forever.
    pub async fn observe_with_timeout<H: FnMut(Packet) + Send + 'static>(
        mut self,
        resource_path: &str,
        mut handler: H,
        timeout: Duration,
    ) -> Result<oneshot::Sender<ObserveMessage>>
    where
        T: 'static + Send + Sync,
    {
        // TODO: support observe multi resources at the same time
        let mut message_id = self.message_id;
        let mut register_packet = CoapRequest::new();
        register_packet.set_observe_flag(ObserveOption::Register);
        register_packet.message.header.message_id = Self::gen_message_id(&mut message_id);
        register_packet.set_path(&resource_path);

        self.send(&register_packet).await?;

        self.set_receive_timeout(Some(timeout))?;
        let response = self.receive().await?;
        if *response.get_status() != Status::Content {
            return Err(Error::new(ErrorKind::NotFound, "the resource not found"));
        }

        handler(response.message);

        let (tx, rx) = oneshot::channel();
        let observe_path = String::from(resource_path);

        tokio::spawn(async move {
            let mut rx_pinned: Pin<
                Box<
                    dyn Future<
                            Output = std::result::Result<ObserveMessage, oneshot::error::RecvError>,
                        > + Send,
                >,
            > = Box::pin(rx);
            loop {
                tokio::select! {
                    sock_rx = self.receive_from_socket() => {
                        self.receive_and_handle_message(sock_rx, &mut handler).await;
                    }
                    observe = &mut rx_pinned => {
                        match observe {
                            Ok(ObserveMessage::Terminate) => {
                                self.terminate_observe(&observe_path, &mut message_id).await;
                                break;
                            }
                            // if the receiver is dropped, we change the future to wait forever
                            Err(_) => {
                                debug!("observe continuing forever");
                                rx_pinned  = Box::pin(futures::future::pending())
                            },
                        }
                    }

                }
            }
        });
        return Ok(tx);
    }

    async fn terminate_observe(&mut self, observe_path: &str, message_id: &mut u16) {
        let mut deregister_packet = CoapRequest::<SocketAddr>::new();
        deregister_packet.message.header.message_id = Self::gen_message_id(message_id);
        deregister_packet.set_observe_flag(ObserveOption::Deregister);
        deregister_packet.set_path(observe_path);
        let _ = Self::send_with_socket(&self.transport, &deregister_packet.message).await;
        let _ = self.receive_from_socket().await;
    }

    async fn receive_and_handle_message<H: FnMut(Packet) + Send + 'static>(
        &self,
        socket_result: Result<(Packet, SocketAddr)>,
        handler: &mut H,
    ) {
        match socket_result {
            Ok((packet, src)) => {
                let receive_packet = CoapRequest::from_packet(packet, src);
                debug!("received a packet: {:?}", &receive_packet);
                handler(receive_packet.message);

                if let Some(response) = receive_packet.response {
                    let mut packet = Packet::new();
                    packet.header.set_type(response.message.header.get_type());
                    packet.header.message_id = response.message.header.message_id;
                    packet.set_token(response.message.get_token().into());

                    match Self::send_with_socket(&self.transport, &packet).await {
                        Ok(_) => (),
                        Err(e) => {
                            warn!("reply ack failed {}", e)
                        }
                    }
                }
            }
            Err(e) => match e.kind() {
                ErrorKind::WouldBlock => {
                    info!("Observe timeout");
                }
                _ => warn!("observe failed {:?}", e),
            },
        };
    }

    /// Execute a request.
    pub async fn send(&self, request: &CoapRequest<SocketAddr>) -> Result<()> {
        Self::send_with_socket(&self.transport, &request.message).await
    }

    /// send a request supporting block1 option based on the block size set in the client
    pub async fn send2(&mut self, request: &mut CoapRequest<SocketAddr>) -> Result<()> {
        let request_length = request.message.payload.len();
        if request_length <= self.block1_size {
            return self.send(request).await;
        }
        let payload = std::mem::take(&mut request.message.payload);
        let mut it = payload.chunks(self.block1_size).enumerate().peekable();
        while let Some((idx, elem)) = it.next() {
            let more_blocks = it.peek().is_some();
            let block = BlockValue::new(idx, more_blocks, self.block1_size)
                .map_err(|_| Error::new(ErrorKind::Other, "could not set block size"))?;

            request.message.clear_option(CoapOption::Block1);
            request
                .message
                .add_option_as::<BlockValue>(CoapOption::Block1, block.clone());
            request.message.payload = elem.to_vec();

            request.message.header.message_id = Self::gen_message_id(&mut self.message_id);
            self.send(request).await?;
            // continue receiving responses until last element
            if it.peek().is_some() {
                let resp = self.receive().await?;
                let maybe_block1 = resp
                    .message
                    .get_first_option_as::<BlockValue>(CoapOption::Block1)
                    .ok_or(Error::new(
                        ErrorKind::Unsupported,
                        "endpoint does not support blockwise transfers. Try setting block1_size to a larger value",
                    ))?;
                let block1_resp = maybe_block1.map_err(|_| {
                    Error::new(
                        ErrorKind::InvalidData,
                        "endpoint responded with invalid block",
                    )
                })?;
                //TODO: negotiate smaller block size
                if block1_resp.size_exponent != block.size_exponent {
                    return Err(Error::new(
                        ErrorKind::Unsupported,
                        "negotiating block size is currently unsupported",
                    ));
                }
            }
        }
        Ok(())
    }
    /// Receive a response.
    pub async fn receive(&self) -> Result<CoapResponse> {
        let (packet, _src) = self.receive_from_socket().await?;
        Ok(CoapResponse { message: packet })
    }

    /// Receive a response support block-wise.
    pub async fn receive2(
        &mut self,
        request: &mut CoapRequest<SocketAddr>,
    ) -> Result<CoapResponse> {
        loop {
            let (packet, _src) = self.receive_from_socket().await?;
            request.response = CoapResponse::new(&request.message);
            let response = request
                .response
                .as_mut()
                .ok_or_else(|| Error::new(ErrorKind::Interrupted, "packet error"))?;
            response.message = packet;
            match self.intercept_response(request) {
                Ok(true) => {
                    self.send(request).await?;
                }
                Err(err) => {
                    error!("intercept response error: {:?}", err);
                    return Err(Error::new(ErrorKind::Interrupted, "packet error"));
                }
                Ok(false) => {
                    break;
                }
            }
        }
        Ok(CoapResponse {
            message: request.response.as_ref().unwrap().message.clone(),
        })
    }

    /// Receive a response.
    pub async fn receive_from(&self) -> Result<(CoapResponse, SocketAddr)> {
        let (packet, src) = self.receive_from_socket().await?;
        Ok((CoapResponse { message: packet }, src))
    }

    /// Set the receive timeout.
    pub fn set_receive_timeout(&mut self, dur: Option<Duration>) -> Result<()> {
        self.read_timeout = dur;
        Ok(())
    }

    /// Set the maximum size for a block1 request. Default is 1024 bytes
    pub fn set_block1_size(&mut self, block1_max_bytes: usize) {
        self.block1_size = block1_max_bytes;
    }

    async fn send_with_socket(socket: &T, message: &Packet) -> Result<()> {
        let message_bytes = message
            .to_bytes()
            .map_err(|_| Error::new(ErrorKind::InvalidInput, "packet error"))?;
        // TODO: figure out what to do with peer address
        let size = socket.send(&message_bytes[..]).await?;
        if size == message_bytes.len() {
            Ok(())
        } else {
            Err(Error::new(ErrorKind::Other, "send length error"))
        }
    }

    async fn receive_from_socket(&self) -> Result<(Packet, SocketAddr)> {
        let mut buf = [0; 1500];
        let (nread, src) = match self.read_timeout {
            Some(dur) => timeout(dur, self.transport.recv(&mut buf)).await??,
            None => self.transport.recv(&mut buf).await?,
        };
        match Packet::from_bytes(&buf[..nread]) {
            Ok(packet) => Ok((packet, src)),
            Err(_) => Err(Error::new(ErrorKind::InvalidInput, "packet error")),
        }
    }

    fn parse_coap_url(url: &str) -> Result<(String, u16, String, Option<Vec<u8>>)> {
        let url_params = match Url::parse(url) {
            Ok(url_params) => url_params,
            Err(_) => return Err(Error::new(ErrorKind::InvalidInput, "url error")),
        };

        let host = match url_params.host_str() {
            Some("") => return Err(Error::new(ErrorKind::InvalidInput, "host error")),
            Some(h) => h,
            None => return Err(Error::new(ErrorKind::InvalidInput, "host error")),
        };
        let host = Regex::new(r"^\[(.*?)]$")
            .unwrap()
            .replace(&host, "$1")
            .to_string();

        let port = match url_params.port() {
            Some(p) => p,
            None => 5683,
        };

        let path = url_params.path().to_string();

        let queries = url_params.query().map(|q| q.as_bytes().to_vec());

        return Ok((host.to_string(), port, path, queries));
    }

    fn gen_message_id(message_id: &mut u16) -> u16 {
        (*message_id) += 1;
        return *message_id;
    }

    fn intercept_response(
        &mut self,
        request: &mut CoapRequest<SocketAddr>,
    ) -> std::result::Result<bool, HandlingError> {
        let state = self
            .block2_states
            .entry(request.deref().into())
            .or_insert(BlockState::default());

        let block2_handled = Self::maybe_handle_response_block2(request, state)?;
        if block2_handled {
            return Ok(true);
        }

        Ok(false)
    }

    fn maybe_handle_response_block2(
        request: &mut CoapRequest<SocketAddr>,
        state: &mut BlockState,
    ) -> std::result::Result<bool, HandlingError> {
        let response = request.response.as_ref().unwrap();
        let maybe_block2 = response
            .message
            .get_first_option_as::<BlockValue>(CoapOption::Block2)
            .and_then(|x| x.ok());

        if let Some(block2) = maybe_block2 {
            if state.cached_payload.is_none() {
                state.cached_payload = Some(Vec::new());
            }
            let cached_payload = state.cached_payload.as_mut().unwrap();

            let payload_offset = usize::from(block2.num) * block2.size();
            extending_splice(
                cached_payload,
                payload_offset..payload_offset + block2.size(),
                response.message.payload.iter().copied(),
                16 * 1024,
            )
            .map_err(HandlingError::internal)?;

            if block2.more {
                request.message.clear_option(CoapOption::Block2);
                let mut next_block2 = block2.clone();
                next_block2.num += 1;
                request
                    .message
                    .add_option_as::<BlockValue>(CoapOption::Block2, next_block2);
                return Ok(true);
            } else {
                let cached_payload = mem::take(&mut state.cached_payload).unwrap();
                request.response.as_mut().unwrap().message.payload = cached_payload;
            }
        }

        Ok(false)
    }
}

#[derive(Debug, Clone, Default)]
pub struct BlockState {
    cached_payload: Option<Vec<u8>>,
}

#[cfg(test)]
mod test {

    use super::super::*;
    use super::*;
    use std::io::ErrorKind;
    use std::time::Duration;
    #[test]
    fn test_parse_coap_url_good_url() {
        assert!(UdpCoAPClient::parse_coap_url("coap://127.0.0.1").is_ok());
        assert!(UdpCoAPClient::parse_coap_url("coap://127.0.0.1:5683").is_ok());
        assert!(UdpCoAPClient::parse_coap_url("coap://[::1]").is_ok());
        assert!(UdpCoAPClient::parse_coap_url("coap://[::1]:5683").is_ok());
        assert!(UdpCoAPClient::parse_coap_url("coap://[bbbb::9329:f033:f558:7418]").is_ok());
        assert!(UdpCoAPClient::parse_coap_url("coap://[bbbb::9329:f033:f558:7418]:5683").is_ok());
        assert!(UdpCoAPClient::parse_coap_url("coap://127.0.0.1/?hello=world").is_ok());
    }

    #[test]
    fn test_parse_coap_url_bad_url() {
        assert!(UdpCoAPClient::parse_coap_url("coap://127.0.0.1:65536").is_err());
        assert!(UdpCoAPClient::parse_coap_url("coap://").is_err());
        assert!(UdpCoAPClient::parse_coap_url("coap://:5683").is_err());
        assert!(UdpCoAPClient::parse_coap_url("127.0.0.1").is_err());
    }

    async fn request_handler(req: Box<CoapRequest<SocketAddr>>) -> Box<CoapRequest<SocketAddr>> {
        tokio::time::sleep(Duration::from_secs(1)).await;
        req
    }

    #[test]
    fn test_parse_queries() {
        if let Ok((_, _, _, Some(queries))) =
            UdpCoAPClient::parse_coap_url("coap://127.0.0.1/?hello=world&test1=test2")
        {
            assert_eq!("hello=world&test1=test2".as_bytes().to_vec(), queries);
        } else {
            error!("Parse Queries failed");
        }
    }

    #[tokio::test]
    async fn test_get_url() {
        let resp = UdpCoAPClient::get("coap://coap.me:5683/hello")
            .await
            .unwrap();
        assert_eq!(resp.message.payload, b"world".to_vec());
    }

    #[tokio::test]
    async fn test_get_url_timeout() {
        let server_port = server::test::spawn_server("127.0.0.1:0", request_handler)
            .recv()
            .await
            .unwrap();

        let error = UdpCoAPClient::get_with_timeout(
            &format!("coap://127.0.0.1:{}/Rust", server_port),
            Duration::new(0, 0),
        )
        .await
        .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::TimedOut);
    }

    #[tokio::test]
    async fn test_get() {
        let domain = "coap.me";
        let mut client = UdpCoAPClient::new_udp((domain, 5683)).await.unwrap();
        let resp = client
            .request_path("/hello", Method::Get, None, None, Some(domain.to_string()))
            .await
            .unwrap();
        assert_eq!(resp.message.payload, b"world".to_vec());
    }
    #[tokio::test]
    async fn test_post_url() {
        let resp = UdpCoAPClient::post("coap://coap.me:5683/validate", b"world".to_vec())
            .await
            .unwrap();
        assert_eq!(resp.message.payload, b"POST OK".to_vec());
        let resp = UdpCoAPClient::post("coap://coap.me:5683/validate", b"test".to_vec())
            .await
            .unwrap();
        assert_eq!(resp.message.payload, b"POST OK".to_vec());
    }

    #[tokio::test]
    async fn test_post() {
        let domain = "coap.me";
        let mut client = UdpCoAPClient::new_udp((domain, 5683)).await.unwrap();
        let resp = client
            .request_path(
                "/validate",
                Method::Post,
                Some(b"world".to_vec()),
                None,
                Some(domain.to_string()),
            )
            .await
            .unwrap();
        assert_eq!(resp.message.payload, b"POST OK".to_vec());
    }

    #[tokio::test]
    async fn test_put_url() {
        let resp = UdpCoAPClient::put("coap://coap.me:5683/create1", b"world".to_vec())
            .await
            .unwrap();
        assert_eq!(resp.message.payload, b"Created".to_vec());
        let resp = UdpCoAPClient::put("coap://coap.me:5683/create1", b"test".to_vec())
            .await
            .unwrap();
        assert_eq!(resp.message.payload, b"Created".to_vec());
    }

    #[tokio::test]
    async fn test_put() {
        let domain = "coap.me";
        let mut client = UdpCoAPClient::new_udp((domain, 5683)).await.unwrap();
        let resp = client
            .request_path(
                "/create1",
                Method::Put,
                Some(b"world".to_vec()),
                None,
                Some(domain.to_string()),
            )
            .await
            .unwrap();
        assert_eq!(resp.message.payload, b"Created".to_vec());
    }

    #[tokio::test]
    async fn test_delete_url() {
        let resp = UdpCoAPClient::delete("coap://coap.me:5683/validate")
            .await
            .unwrap();
        assert_eq!(resp.message.payload, b"DELETE OK".to_vec());
        let resp = UdpCoAPClient::delete("coap://coap.me:5683/validate")
            .await
            .unwrap();
        assert_eq!(resp.message.payload, b"DELETE OK".to_vec());
    }

    #[tokio::test]
    async fn test_delete() {
        let domain = "coap.me";
        let mut client = UdpCoAPClient::new_udp((domain, 5683)).await.unwrap();
        let resp = client
            .request_path(
                "/validate",
                Method::Delete,
                None,
                None,
                Some(domain.to_string()),
            )
            .await
            .unwrap();
        assert_eq!(resp.message.payload, b"DELETE OK".to_vec());
    }

    #[tokio::test]
    async fn test_set_broadcast() {
        let client = UdpCoAPClient::new_udp(("127.0.0.1", 5683)).await.unwrap();
        assert!(client.set_broadcast(true).is_ok());
        assert!(client.set_broadcast(false).is_ok());
    }

    #[tokio::test]
    #[ignore]
    async fn test_set_broadcast_v6() {
        let client = UdpCoAPClient::new_udp(("::1", 5683)).await.unwrap();
        assert!(client.set_broadcast(true).is_ok());
        assert!(client.set_broadcast(false).is_ok());
    }

    #[tokio::test]
    async fn test_send_all_coap() {
        // prepare the Non-confirmable request with the broadcast message
        let mut request: CoapRequest<SocketAddr> = CoapRequest::new();
        request.set_method(Method::Get);
        request.set_path("/");
        request
            .message
            .header
            .set_type(coap_lite::MessageType::NonConfirmable);
        request.message.payload = b"Discovery".to_vec();

        let client = UdpCoAPClient::new_udp(("127.0.0.1", 5683)).await.unwrap();
        client.send_all_coap(&request, 0).await.unwrap();
    }
    #[tokio::test]
    async fn test_change_block_option() {
        // this test is a little finnicky because it relies on the configuration
        // of the reception endpoint. It tries to send a payload larger than the
        // default using a block option, this request is expected to fail because
        // the endpoint does not support block requests. Afterwards, we change the
        // maximum block size and thus expect the request to work.
        const PAYLOAD_STR: &str = "this is a payload";
        let mut large_payload = vec![];
        while large_payload.len() < 1024 {
            large_payload.extend_from_slice(PAYLOAD_STR.as_bytes());
        }
        let domain = "coap.me";
        let mut client = UdpCoAPClient::new_udp((domain, 5683)).await.unwrap();
        let resp = client
            .request_path(
                "/large-create",
                Method::Put,
                Some(large_payload.clone()),
                None,
                Some(domain.to_string()),
            )
            .await;
        let err = resp.unwrap_err();
        assert_eq!(err.kind(), ErrorKind::Unsupported);
        //we now set the block size to make sure it is sent in a single request
        client.set_block1_size(10_000_000);

        let resp = client
            .request_path(
                "/large-create",
                Method::Post,
                Some(large_payload.clone()),
                None,
                Some(domain.to_string()),
            )
            .await
            .unwrap();
        assert_eq!(*resp.get_status(), Status::Created);
    }
    #[tokio::test]
    #[ignore]
    async fn test_send_all_coap_v6() {
        // prepare the Non-confirmable request with the broadcast message
        let mut request: CoapRequest<SocketAddr> = CoapRequest::new();
        request.set_method(Method::Get);
        request.set_path("/");
        request
            .message
            .header
            .set_type(coap_lite::MessageType::NonConfirmable);
        request.message.payload = b"Discovery".to_vec();

        let client = UdpCoAPClient::new_udp(("::1", 5683)).await.unwrap();
        client.send_all_coap(&request, 0x4).await.unwrap();
    }
}
