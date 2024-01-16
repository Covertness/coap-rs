#[cfg(feature = "dtls")]
use crate::dtls::{DtlsConfig, DtlsConnection};
use alloc::string::String;
use alloc::vec::Vec;
use coap_lite::{
    block_handler::{extending_splice, BlockValue, RequestCacheKey},
    error::HandlingError,
    CoapOption, CoapRequest, CoapResponse, MessageClass, MessageType, ObserveOption, Packet,
    RequestType as Method, ResponseType as Status,
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
    io::{Error, ErrorKind, Result as IoResult},
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
/// A basic interface for a transport on both the client and transport
/// representing a one-to-one connection between a client and server
/// timeouts and retries do not need to be implemented by the transport
/// if confirmable messages are sent
pub trait Transport: Send {
    async fn recv(&self, buf: &mut [u8]) -> std::io::Result<(usize, SocketAddr)>;
    async fn send(&self, buf: &[u8]) -> std::io::Result<usize>;
}

/// a wrapper for transports responsible for retries and timeouts
struct ClientTransport<T> {
    pub(crate) inner: T,
    cache: Option<(Packet, SocketAddr)>,
    pub(crate) retries: usize,
    pub(crate) timeout: Duration,
}

impl<T: Transport> ClientTransport<T> {
    pub const DEFAULT_NUM_RETRIES: usize = 5;
    async fn send_confirmable_message(&mut self, msg: &[u8]) -> IoResult<()> {
        self.cache = None;
        self.inner.send(&msg).await?;
        let (packet, src) = self.try_receive_packet().await?;
        let msg_type = packet.header.get_type();
        let msg_code = packet.header.code;
        // case 1: we received an ACK and it is empty
        match (msg_type, msg_code) {
            (MessageType::Acknowledgement, MessageClass::Empty) => Ok(()),
            (_, MessageClass::Response(_)) => {
                self.cache = Some((packet, src));
                Ok(())
            }
            (_, _) => Err(Error::new(ErrorKind::InvalidInput, "protocol error")),
        }
    }

    pub async fn try_receive_packet(&mut self) -> IoResult<(Packet, SocketAddr)> {
        let mut buf = [0; 1500];
        if let Some(p) = self.cache.take() {
            return Ok(p);
        }
        let (nread, src) = timeout(self.timeout, self.inner.recv(&mut buf)).await??;
        let packet = Packet::from_bytes(&buf[..nread])
            .map_err(|_| Error::new(ErrorKind::InvalidInput, "packet error"))?;
        return Ok((packet, src));
    }
    /// tries to send a confirmable message with retries and timeouts
    async fn try_send_confirmable_message(&mut self, msg: &[u8]) -> IoResult<()> {
        for _ in 0..self.retries {
            let try_send = timeout(self.timeout, self.send_confirmable_message(msg)).await;
            if let Ok(Ok(res)) = try_send {
                return Ok(res);
            }
        }
        Err(Error::new(ErrorKind::TimedOut, "send timeout"))
    }

    async fn send_non_confirmable_message(&mut self, msg: &[u8]) -> IoResult<()> {
        self.cache = None;
        self.inner.send(&msg).await?;
        Ok(())
    }

    pub async fn send_request(&mut self, request: &CoapRequest<SocketAddr>) -> IoResult<()> {
        self.send_packet(&request.message).await
    }

    pub async fn send_packet(&mut self, packet: &Packet) -> IoResult<()> {
        if packet.header.get_type() == MessageType::Confirmable {
            self.try_send_confirmable_message(&packet.to_bytes().unwrap())
                .await?;
        } else {
            self.send_non_confirmable_message(&packet.to_bytes().unwrap())
                .await?;
        }
        Ok(())
    }

    pub async fn receive_packet_no_timeout(&mut self) -> IoResult<(Packet, SocketAddr)> {
        let old_timeout = self.timeout;
        self.timeout = Duration::from_secs(u64::MAX);
        let res = self.try_receive_packet().await;
        self.timeout = old_timeout;
        return res;
    }
    pub fn from_transport(transport: T) -> Self {
        return Self {
            inner: transport,
            cache: None,
            retries: Self::DEFAULT_NUM_RETRIES,
            timeout: Duration::from_secs(3),
        };
    }
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
    transport: ClientTransport<T>,
    block2_states: LruCache<RequestCacheKey<SocketAddr>, BlockState>,
    block1_size: usize,
    message_id: u16,
}

impl UdpCoAPClient {
    pub async fn new_with_specific_source<A: ToSocketAddrs, B: ToSocketAddrs>(
        bind_addr: A,
        peer_addr: B,
    ) -> IoResult<Self> {
        let peer_addr = lookup_host(peer_addr).await?.next().ok_or(Error::new(
            ErrorKind::InvalidInput,
            "could not get socket address",
        ))?;
        let socket = UdpSocket::bind(bind_addr).await?;
        let transport = UdpTransport { socket, peer_addr };
        return Ok(UdpCoAPClient::from_transport(transport));
    }

    pub async fn new_udp<A: ToSocketAddrs>(addr: A) -> IoResult<Self> {
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
    ) -> IoResult<()> {
        assert!(segment <= 0xf);
        let addr = match self.transport.inner.peer_addr {
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
                let size = self
                    .transport
                    .inner
                    .socket
                    .send_to(&bytes[..], addr)
                    .await?;
                if size == bytes.len() {
                    Ok(())
                } else {
                    Err(Error::new(ErrorKind::Other, "send length error"))
                }
            }
            Err(_) => Err(Error::new(ErrorKind::InvalidInput, "packet error")),
        }
    }

    pub fn set_broadcast(&self, value: bool) -> IoResult<()> {
        self.transport.inner.socket.set_broadcast(value)
    }
}

#[cfg(feature = "dtls")]
impl CoAPClient<DtlsConnection> {
    pub async fn from_dtls_config(config: DtlsConfig) -> IoResult<Self> {
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
            transport: ClientTransport::from_transport(transport),
            block2_states: LruCache::with_expiry_duration(Duration::from_secs(120)),
            block1_size: Self::MAX_PAYLOAD_BLOCK,
            message_id: 0,
        }
    }
    /// Execute a single get request with a coap url
    pub async fn get(url: &str) -> IoResult<CoapResponse> {
        Self::request(url, Method::Get, None).await
    }

    /// Execute a single get request with a coap url and a specific timeout.
    pub async fn get_with_timeout(url: &str, timeout: Duration) -> IoResult<CoapResponse> {
        Self::request_with_timeout(url, Method::Get, None, timeout).await
    }

    /// Execute a single post request with a coap url using udp
    pub async fn post(url: &str, data: Vec<u8>) -> IoResult<CoapResponse> {
        Self::request(url, Method::Post, Some(data)).await
    }

    /// Execute a single post request with a coap url using udp
    pub async fn post_with_timeout(
        url: &str,
        data: Vec<u8>,
        timeout: Duration,
    ) -> IoResult<CoapResponse> {
        Self::request_with_timeout(url, Method::Post, Some(data), timeout).await
    }

    /// Execute a put request with a coap url using udp
    pub async fn put(url: &str, data: Vec<u8>) -> IoResult<CoapResponse> {
        Self::request(url, Method::Put, Some(data)).await
    }

    /// Execute a single put request with a coap url using udp
    pub async fn put_with_timeout(
        url: &str,
        data: Vec<u8>,
        timeout: Duration,
    ) -> IoResult<CoapResponse> {
        Self::request_with_timeout(url, Method::Put, Some(data), timeout).await
    }

    /// Execute a single delete request with a coap url using udp
    pub async fn delete(url: &str) -> IoResult<CoapResponse> {
        Self::request(url, Method::Delete, None).await
    }

    /// Execute a single delete request with a coap url using udp
    pub async fn delete_with_timeout(url: &str, timeout: Duration) -> IoResult<CoapResponse> {
        Self::request_with_timeout(url, Method::Delete, None, timeout).await
    }

    /// Execute a single request (GET, POST, PUT, DELETE) with a coap url using udp
    pub async fn request(
        url: &str,
        method: Method,
        data: Option<Vec<u8>>,
    ) -> IoResult<CoapResponse> {
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
    ) -> IoResult<CoapResponse> {
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
    ) -> IoResult<CoapResponse> {
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
    ) -> IoResult<CoapResponse> {
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
        self.set_receive_timeout(timeout);
        self.perform_request(request).await
    }

    /// Send a Request via the given transport, and receive a response.
    pub async fn perform_request(
        &mut self,
        mut request: CoapRequest<SocketAddr>,
    ) -> IoResult<CoapResponse> {
        self.send_request(&mut request).await?;
        self.receive2(&mut request).await
    }

    pub async fn observe<H: FnMut(Packet) + Send + 'static>(
        self,
        resource_path: &str,
        handler: H,
    ) -> IoResult<oneshot::Sender<ObserveMessage>>
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
    ) -> IoResult<oneshot::Sender<ObserveMessage>>
    where
        T: 'static + Send + Sync,
    {
        // TODO: support observe multi resources at the same time
        let mut message_id = self.message_id;
        let mut register_packet = CoapRequest::new();
        register_packet.set_observe_flag(ObserveOption::Register);
        register_packet.message.header.message_id = Self::gen_message_id(&mut message_id);
        register_packet.set_path(&resource_path);

        self.send_raw_request(&register_packet).await?;

        self.set_receive_timeout(timeout);
        let response = self.receive().await?;
        if *response.get_status() != Status::Content {
            return Err(Error::new(ErrorKind::NotFound, "te resource not found"));
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
                    sock_rx = self.transport.receive_packet_no_timeout() => {
                        self.receive_and_handle_message_observe(sock_rx, &mut handler).await;
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

        let _ = self.transport.send_packet(&deregister_packet.message).await;
        let _ = self.transport.try_receive_packet().await.unwrap();
    }

    async fn receive_and_handle_message_observe<H: FnMut(Packet) + Send + 'static>(
        &mut self,
        socket_result: IoResult<(Packet, SocketAddr)>,
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

                    match self.transport.send_packet(&packet).await {
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

    /// sends a request through the transport. If a request is confirmable, it will attempt
    /// retries until receiving a response. requests sent using a multicast-address should be non-confirmable
    /// the user is responsible for setting meaningful fields in the request
    /// Do not use this method unless you need low-level control over the protocol (e.g.,
    /// multicast), instead use perform_request for client applications.
    pub async fn send_raw_request(&mut self, request: &CoapRequest<SocketAddr>) -> IoResult<()> {
        self.transport.send_request(request).await
    }

    /// low-level method to send a a request supporting block1 option based on
    /// the block size set in the client, prefer using
    async fn send_request(&mut self, request: &mut CoapRequest<SocketAddr>) -> IoResult<()> {
        let request_length = request.message.payload.len();
        if request_length <= self.block1_size {
            return self.send_raw_request(request).await;
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
            self.send_raw_request(request).await?;
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
    pub async fn receive(&mut self) -> IoResult<CoapResponse> {
        let (packet, _) = self.transport.try_receive_packet().await?;
        Ok(CoapResponse { message: packet })
    }

    /// Receive a response support block-wise.
    pub async fn receive2(
        &mut self,
        request: &mut CoapRequest<SocketAddr>,
    ) -> IoResult<CoapResponse> {
        loop {
            let packet = self.transport.try_receive_packet().await?.0;
            request.response = CoapResponse::new(&request.message);
            let response = request
                .response
                .as_mut()
                .ok_or_else(|| Error::new(ErrorKind::Interrupted, "packet error"))?;
            response.message = packet;
            match self.intercept_response(request) {
                Ok(true) => {
                    self.send_raw_request(request).await?;
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

    /// Set the receive timeout.
    pub fn set_receive_timeout(&mut self, dur: Duration) {
        self.transport.timeout = dur;
    }

    pub fn set_transport_retries(&mut self, num_retries: usize) {
        self.transport.retries = num_retries;
    }

    /// Set the maximum size for a block1 request. Default is 1024 bytes
    pub fn set_block1_size(&mut self, block1_max_bytes: usize) {
        self.block1_size = block1_max_bytes;
    }

    fn parse_coap_url(url: &str) -> IoResult<(String, u16, String, Option<Vec<u8>>)> {
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
    use std::sync::atomic::{AtomicU32, Ordering};
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

    struct FaultyUdp {
        pub udp: UdpTransport,
        pub num_fails: u32,
        pub current_fails: AtomicU32,
    }

    #[async_trait]
    impl Transport for FaultyUdp {
        async fn recv(&self, buf: &mut [u8]) -> std::io::Result<(usize, SocketAddr)> {
            self.udp.recv(buf).await
        }

        async fn send(&self, buf: &[u8]) -> std::io::Result<usize> {
            self.current_fails.fetch_add(1, Ordering::Relaxed);
            self.current_fails
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |n| {
                    Some(n % self.num_fails)
                })
                .unwrap();
            if self.current_fails.load(Ordering::Relaxed) == 0 {
                return self.udp.send(buf).await;
            }
            Err(Error::new(ErrorKind::Other, "fails this time"))
        }
    }

    #[tokio::test]
    async fn test_retries() {
        let server_port = server::test::spawn_server("127.0.0.1:0", |mut req| async {
            req.response.as_mut().unwrap().message.payload = b"Rust".to_vec();
            return req;
        })
        .recv()
        .await
        .unwrap();

        let server_addr = format!("127.0.0.1:{}", server_port);
        let peer_addr = lookup_host(server_addr.clone())
            .await
            .unwrap()
            .next()
            .ok_or(Error::new(
                ErrorKind::InvalidInput,
                "could not get socket address",
            ))
            .unwrap();
        let socket = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let transport = UdpTransport { socket, peer_addr };
        let transport = FaultyUdp {
            udp: transport,
            num_fails: ClientTransport::<UdpTransport>::DEFAULT_NUM_RETRIES as u32 + 1,
            current_fails: 0.into(),
        };

        let mut client = CoAPClient::from_transport(transport);
        let error = client
            .request_path("/Rust", Method::Get, None, None, Some(server_addr.clone()))
            .await
            .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::TimedOut);
        //this request will work, we do this to reset the state of the faulty udp
        client
            .request_path("/Rust", Method::Get, None, None, Some(server_addr.clone()))
            .await
            .unwrap();

        client.set_transport_retries(ClientTransport::<UdpTransport>::DEFAULT_NUM_RETRIES + 2);
        let resp = client
            .request_path("/Rust", Method::Get, None, None, Some(server_addr))
            .await
            .unwrap();

        assert_eq!(resp.message.payload, b"Rust".to_vec());
    }
}
