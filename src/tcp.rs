//! CoAP over TCP transport as defined in [RFC 8323](https://datatracker.ietf.org/doc/rfc8323/).
//!
//! This module provides:
//! - [`TcpCoapListener`]: a server-side listener that accepts incoming TCP connections
//! - [`TcpTransport`]: a client-side transport that sends and receives CoAP messages over TCP
//! - [`TcpCoAPClient`]: a ready-to-use CoAP client type alias backed by [`TcpTransport`]
//!
//! # Wire format
//!
//! RFC 8323 defines a compact framing format for CoAP over reliable transports.
//! The first byte encodes a 4-bit length indicator (`Len`) and the 4-bit token
//! length (`TKL`).  The `Len` field selects whether additional "extended length"
//! bytes follow, after which comes the `Code` byte, the token, the CoAP options,
//! and the optional payload.
//!
//! Internally this implementation converts between the RFC 8323 wire format and
//! the coap-lite UDP wire format so that the rest of the library can continue to
//! work with [`coap_lite::Packet`] unchanged.
//!
//! # Example (server + client)
//!
//! ```no_run
//! use coap::Server;
//! use coap::tcp::TcpCoAPClient;
//! use coap_lite::{CoapRequest, RequestType as Method};
//! use std::net::SocketAddr;
//!
//! #[tokio::main]
//! async fn main() {
//!     // --- server ---
//!     tokio::spawn(async {
//!         let mut server = Server::new_tcp("127.0.0.1:5683").await.unwrap();
//!         server.run(|mut req: Box<CoapRequest<SocketAddr>>| async {
//!             if let Some(ref mut resp) = req.response {
//!                 resp.message.payload = b"hello".to_vec();
//!             }
//!             req
//!         }).await.unwrap();
//!     });
//!
//!     // --- client ---
//!     let client = TcpCoAPClient::new("127.0.0.1:5683").await.unwrap();
//!     let request = coap::request::RequestBuilder::new("/test", Method::Get).build();
//!     let response = client.send(request).await.unwrap();
//!     println!("{}", String::from_utf8_lossy(&response.message.payload));
//! }
//! ```

use crate::client::{ClientTransport, CoAPClient};
use crate::server::{Listener, Responder, TransportRequestSender};
use async_trait::async_trait;
use log::debug;
use std::io::{Error, ErrorKind, Result as IoResult};
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, ToSocketAddrs};
use tokio::sync::{mpsc::unbounded_channel, Mutex};
use tokio::task::JoinHandle;

// ─── RFC 8323 framing ──────────────────────────────────────────────────────

/// Encode a packet in coap-lite's UDP wire format into the RFC 8323 TCP wire
/// format.
///
/// UDP format: `[Ver|T|TKL][Code][MsgID_Hi][MsgID_Lo][Token…][Options+Payload]`
/// TCP format: `[Len|TKL][ExtLen…][Code][Token…][Options+Payload]`
///
/// The `Type`, `Version`, and `Message ID` fields present in the UDP format
/// are absent in the TCP format.  Token, options, and payload encoding is
/// identical between the two formats.
pub(crate) fn encode_tcp(udp_bytes: &[u8]) -> IoResult<Vec<u8>> {
    if udp_bytes.len() < 4 {
        return Err(Error::new(ErrorKind::InvalidData, "packet too short"));
    }
    let tkl = udp_bytes[0] & 0x0F;
    let code = udp_bytes[1];
    // Bytes 2-3 are the UDP message ID – skip them.  Bytes 4+ are Token +
    // Options + Payload, which are identical in both formats.
    let rest = &udp_bytes[4..];

    // data_len = Code (1 byte) + Token + Options + Payload
    let data_len = 1_usize + rest.len();

    let mut tcp_bytes: Vec<u8> = Vec::with_capacity(data_len + 5);

    if data_len <= 12 {
        tcp_bytes.push((data_len as u8) << 4 | tkl);
    } else if data_len <= 268 {
        // Len = 13, extended length = data_len - 13
        tcp_bytes.push(13u8 << 4 | tkl);
        tcp_bytes.push((data_len - 13) as u8);
    } else if data_len <= 65804 {
        // Len = 14, extended length (2 bytes BE) = data_len - 269
        tcp_bytes.push(14u8 << 4 | tkl);
        tcp_bytes.extend_from_slice(&((data_len - 269) as u16).to_be_bytes());
    } else {
        // Len = 15, extended length (4 bytes BE) = data_len - 65805
        tcp_bytes.push(15u8 << 4 | tkl);
        tcp_bytes.extend_from_slice(&((data_len - 65805) as u32).to_be_bytes());
    }

    tcp_bytes.push(code);
    tcp_bytes.extend_from_slice(rest);
    Ok(tcp_bytes)
}

/// Decode a complete RFC 8323 TCP-framed message into coap-lite's UDP wire
/// format.
///
/// The caller is responsible for providing a buffer that contains exactly one
/// complete TCP frame (as produced by [`read_tcp_frame`]).
pub(crate) fn decode_tcp(tcp_bytes: &[u8]) -> IoResult<Vec<u8>> {
    if tcp_bytes.is_empty() {
        return Err(Error::new(ErrorKind::InvalidData, "empty frame"));
    }

    let first_byte = tcp_bytes[0];
    let len_nibble = (first_byte >> 4) & 0x0F;
    let tkl = first_byte & 0x0F;

    let (data_len, header_end) = decode_length(len_nibble, tcp_bytes)?;

    if tcp_bytes.len() < header_end + data_len {
        return Err(Error::new(ErrorKind::InvalidData, "frame too short"));
    }

    let code = tcp_bytes[header_end];
    let rest = &tcp_bytes[header_end + 1..header_end + data_len];

    // Reconstruct as UDP format.
    // Ver=1 (01), T=Non-confirmable (01), TKL = tkl → first nibble = 0b01_01_TKL = 0x50 | tkl
    let mut udp_bytes: Vec<u8> = Vec::with_capacity(4 + rest.len());
    udp_bytes.push(0x50 | tkl); // Ver=1, Type=Non-confirmable
    udp_bytes.push(code);
    udp_bytes.push(0x00); // Message ID (not meaningful over TCP)
    udp_bytes.push(0x00);
    udp_bytes.extend_from_slice(rest);
    Ok(udp_bytes)
}

/// Decode the length fields from a TCP frame header.
/// Returns `(data_len, header_end_offset)`.
fn decode_length(len_nibble: u8, buf: &[u8]) -> IoResult<(usize, usize)> {
    match len_nibble {
        0..=12 => Ok((len_nibble as usize, 1)),
        13 => {
            if buf.len() < 2 {
                return Err(Error::new(ErrorKind::InvalidData, "truncated extended length"));
            }
            Ok((buf[1] as usize + 13, 2))
        }
        14 => {
            if buf.len() < 3 {
                return Err(Error::new(ErrorKind::InvalidData, "truncated extended length"));
            }
            Ok((
                u16::from_be_bytes([buf[1], buf[2]]) as usize + 269,
                3,
            ))
        }
        15 => {
            if buf.len() < 5 {
                return Err(Error::new(ErrorKind::InvalidData, "truncated extended length"));
            }
            Ok((
                u32::from_be_bytes([buf[1], buf[2], buf[3], buf[4]]) as usize + 65805,
                5,
            ))
        }
        _ => Err(Error::new(ErrorKind::InvalidData, "invalid Len nibble")),
    }
}

/// Read exactly one complete TCP-framed CoAP message from `stream`.
///
/// This function issues multiple small reads to avoid holding partial state
/// across cancel points in the way a single `read_exact` call would.
async fn read_tcp_frame(stream: &mut (impl AsyncReadExt + Unpin)) -> IoResult<Vec<u8>> {
    // Read the first byte (contains Len nibble and TKL nibble).
    let mut first = [0u8; 1];
    stream.read_exact(&mut first).await?;
    let len_nibble = (first[0] >> 4) & 0x0F;

    // Read optional extended-length bytes based on the Len nibble.
    let mut ext_buf = Vec::new();
    let (data_len, _) = match len_nibble {
        0..=12 => (len_nibble as usize, 1usize),
        13 => {
            let mut ext = [0u8; 1];
            stream.read_exact(&mut ext).await?;
            ext_buf.extend_from_slice(&ext);
            (ext[0] as usize + 13, 2)
        }
        14 => {
            let mut ext = [0u8; 2];
            stream.read_exact(&mut ext).await?;
            ext_buf.extend_from_slice(&ext);
            (u16::from_be_bytes(ext) as usize + 269, 3)
        }
        15 => {
            let mut ext = [0u8; 4];
            stream.read_exact(&mut ext).await?;
            ext_buf.extend_from_slice(&ext);
            (u32::from_be_bytes(ext) as usize + 65805, 5)
        }
        _ => {
            return Err(Error::new(ErrorKind::InvalidData, "invalid Len nibble"));
        }
    };

    // Read the message data (Code + Token + Options + Payload).
    let mut data = vec![0u8; data_len];
    stream.read_exact(&mut data).await?;

    // Reassemble the full TCP frame bytes.
    let mut frame = Vec::with_capacity(1 + ext_buf.len() + data_len);
    frame.push(first[0]);
    frame.extend_from_slice(&ext_buf);
    frame.extend_from_slice(&data);
    Ok(frame)
}

// ─── Server-side: TcpCoapListener ──────────────────────────────────────────

/// A CoAP listener that accepts connections on a TCP socket.
///
/// Create an instance with [`TcpCoapListener::new`] and pass it to
/// [`crate::Server::from_listeners`], or use the convenience constructor
/// [`crate::Server::new_tcp`].
pub struct TcpCoapListener {
    listener: TcpListener,
}

impl TcpCoapListener {
    /// Bind a TCP listener to `addr`.
    pub async fn new<A: ToSocketAddrs>(addr: A) -> IoResult<Self> {
        let listener = TcpListener::bind(addr).await?;
        Ok(Self { listener })
    }

    /// Return the local address the listener is bound to.
    pub fn local_addr(&self) -> IoResult<SocketAddr> {
        self.listener.local_addr()
    }
}

#[async_trait]
impl Listener for TcpCoapListener {
    async fn listen(
        self: Box<Self>,
        sender: TransportRequestSender,
    ) -> IoResult<JoinHandle<IoResult<()>>> {
        Ok(tokio::spawn(async move {
            loop {
                match self.listener.accept().await {
                    Ok((stream, remote_addr)) => {
                        tokio::spawn(handle_tcp_connection(stream, remote_addr, sender.clone()));
                    }
                    Err(e) => return Err(e),
                }
            }
        }))
    }
}

/// Handle a single TCP connection: read frames in a loop and dispatch them to
/// the server channel together with a [`TcpResponder`] that writes replies back
/// on the same connection.
async fn handle_tcp_connection(
    stream: TcpStream,
    remote_addr: SocketAddr,
    sender: TransportRequestSender,
) {
    let (mut reader, writer) = stream.into_split();
    let writer = Arc::new(Mutex::new(writer));

    loop {
        match read_tcp_frame(&mut reader).await {
            Ok(tcp_bytes) => match decode_tcp(&tcp_bytes) {
                Ok(udp_bytes) => {
                    let responder = Arc::new(TcpResponder {
                        writer: writer.clone(),
                        remote_addr,
                    });
                    if sender.send((udp_bytes, responder)).is_err() {
                        break;
                    }
                }
                Err(e) => {
                    debug!("failed to decode TCP CoAP frame from {}: {}", remote_addr, e);
                }
            },
            Err(e) => {
                debug!("TCP connection from {} closed: {}", remote_addr, e);
                break;
            }
        }
    }
}

/// Sends a CoAP response back over the TCP connection it was received on.
struct TcpResponder {
    writer: Arc<Mutex<tokio::net::tcp::OwnedWriteHalf>>,
    remote_addr: SocketAddr,
}

#[async_trait]
impl Responder for TcpResponder {
    async fn respond(&self, response: Vec<u8>) {
        match encode_tcp(&response) {
            Ok(tcp_bytes) => {
                let writer = self.writer.clone();
                tokio::spawn(async move {
                    let _ = writer.lock().await.write_all(&tcp_bytes).await;
                });
            }
            Err(e) => {
                debug!("failed to encode TCP CoAP response: {}", e);
            }
        }
    }

    fn address(&self) -> SocketAddr {
        self.remote_addr
    }
}

// ─── Client-side: TcpTransport ─────────────────────────────────────────────

/// A CoAP transport over TCP.
///
/// Internally a background task continuously reads frames from the socket and
/// converts them to coap-lite UDP format before placing them in a channel.
/// This avoids partial-read cancellation issues that would otherwise arise from
/// the 300 ms polling loop in the generic receive loop.
///
/// Use [`TcpCoAPClient`] for a ready-to-use client.
pub struct TcpTransport {
    // Receives decoded UDP-format packet bytes from the background reader task.
    receiver: Arc<Mutex<tokio::sync::mpsc::UnboundedReceiver<IoResult<Vec<u8>>>>>,
    writer: Arc<Mutex<tokio::net::tcp::OwnedWriteHalf>>,
    peer_addr: SocketAddr,
}

impl TcpTransport {
    /// Connect to `addr` and return a new [`TcpTransport`].
    pub async fn new<A: ToSocketAddrs>(addr: A) -> IoResult<Self> {
        let stream = TcpStream::connect(addr).await?;
        let peer_addr = stream.peer_addr()?;
        let (reader, writer) = stream.into_split();
        let (tx, rx) = unbounded_channel();

        // Spawn a background task that reads TCP frames and decodes them.
        // `read_exact` is not cancellation-safe, so we must not cancel it from
        // the outside; keeping it in a dedicated task is the correct pattern.
        tokio::spawn(async move {
            let mut reader = reader;
            loop {
                match read_tcp_frame(&mut reader).await {
                    Ok(tcp_bytes) => {
                        let result = decode_tcp(&tcp_bytes);
                        if tx.send(result).is_err() {
                            break; // client was dropped
                        }
                    }
                    Err(e) => {
                        let _ = tx.send(Err(e));
                        break;
                    }
                }
            }
        });

        Ok(Self {
            receiver: Arc::new(Mutex::new(rx)),
            writer: Arc::new(Mutex::new(writer)),
            peer_addr,
        })
    }
}

#[async_trait]
impl ClientTransport for TcpTransport {
    /// Receive one decoded CoAP packet (UDP wire format) from the background
    /// reader.  The provided buffer must be large enough to hold the packet;
    /// data beyond `buf.len()` is silently discarded.
    async fn recv(&self, buf: &mut [u8]) -> IoResult<(usize, Option<SocketAddr>)> {
        let udp_bytes = {
            let mut rx = self.receiver.lock().await;
            rx.recv()
                .await
                .ok_or_else(|| Error::new(ErrorKind::ConnectionAborted, "TCP connection closed"))??
        };
        let n = udp_bytes.len().min(buf.len());
        buf[..n].copy_from_slice(&udp_bytes[..n]);
        Ok((n, Some(self.peer_addr)))
    }

    /// Encode `buf` (UDP wire format) as a TCP frame and write it to the
    /// socket.  Returns the original `buf.len()` on success (not the number of
    /// TCP bytes written).
    async fn send(&self, buf: &[u8]) -> IoResult<usize> {
        let tcp_bytes = encode_tcp(buf)?;
        self.writer.lock().await.write_all(&tcp_bytes).await?;
        Ok(buf.len())
    }
}

// ─── TcpCoAPClient ─────────────────────────────────────────────────────────

/// A CoAP client that communicates over a TCP connection (RFC 8323).
///
/// # Example
///
/// ```no_run
/// use coap::tcp::TcpCoAPClient;
/// use coap::request::RequestBuilder;
/// use coap_lite::RequestType as Method;
///
/// #[tokio::main]
/// async fn main() {
///     let client = TcpCoAPClient::new("127.0.0.1:5683").await.unwrap();
///     let request = RequestBuilder::new("/hello", Method::Get).build();
///     let response = client.send(request).await.unwrap();
///     println!("{}", String::from_utf8_lossy(&response.message.payload));
/// }
/// ```
pub type TcpCoAPClient = CoAPClient<TcpTransport>;

impl TcpCoAPClient {
    /// Connect to the given address and return a new CoAP-over-TCP client.
    pub async fn new<A: ToSocketAddrs>(addr: A) -> IoResult<Self> {
        let transport = TcpTransport::new(addr).await?;
        Ok(CoAPClient::from_transport(transport))
    }
}

// ─── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::request::RequestBuilder;
    use crate::server::Server;
    use coap_lite::{CoapOption, CoapRequest, RequestType as Method};
    use std::future::Future;
    use tokio::sync::mpsc;

    // ── framing round-trip ─────────────────────────────────────────────────

    /// Build a minimal UDP-format packet byte string.
    fn make_udp_bytes(tkl: u8, code: u8, token: &[u8], payload: &[u8]) -> Vec<u8> {
        assert!(tkl as usize == token.len());
        let mut v = vec![
            0x50 | tkl, // Ver=1, T=Non-confirmable, TKL
            code,
            0x00,
            0x00, // Message ID
        ];
        v.extend_from_slice(token);
        if !payload.is_empty() {
            v.push(0xFF); // payload marker
            v.extend_from_slice(payload);
        }
        v
    }

    #[test]
    fn test_framing_small_len() {
        // data_len = 1 (code) + 0 (no token/payload) = 1
        let udp = make_udp_bytes(0, 0x01, &[], &[]);
        let tcp = encode_tcp(&udp).unwrap();
        // First byte: Len=1 (0x10), TKL=0 → 0x10
        assert_eq!(tcp[0] >> 4, 1);
        assert_eq!(tcp[0] & 0x0F, 0);
        let decoded = decode_tcp(&tcp).unwrap();
        assert_eq!(decoded, udp);
    }

    #[test]
    fn test_framing_with_token_and_payload() {
        let token = [0xDE, 0xAD, 0xBE, 0xEF];
        let payload = b"hello world";
        let udp = make_udp_bytes(4, 0x45, &token, payload);
        let tcp = encode_tcp(&udp).unwrap();
        let decoded = decode_tcp(&tcp).unwrap();
        assert_eq!(decoded, udp);
    }

    #[test]
    fn test_framing_extended_len_13() {
        // Craft a UDP packet where data_len > 12.
        // data_len = 1 (code) + 4 (token) + 13 (payload) = 18 > 12 → uses Len=13 encoding
        let token = [0xAA; 4];
        let payload = vec![0xBB; 13];
        let udp = make_udp_bytes(4, 0x45, &token, &payload);
        let tcp = encode_tcp(&udp).unwrap();
        // Len nibble should be 13
        assert_eq!(tcp[0] >> 4, 13);
        let decoded = decode_tcp(&tcp).unwrap();
        assert_eq!(decoded, udp);
    }

    #[test]
    fn test_framing_extended_len_14() {
        // data_len > 268 → Len=14
        let token = [0x00; 8];
        let payload = vec![0xCC; 262]; // data_len = Code(1) + Token(8) + marker(1) + payload(262) = 272 > 268 → Len=14
        let udp = make_udp_bytes(8, 0x45, &token, &payload);
        let tcp = encode_tcp(&udp).unwrap();
        assert_eq!(tcp[0] >> 4, 14);
        let decoded = decode_tcp(&tcp).unwrap();
        assert_eq!(decoded, udp);
    }

    #[test]
    fn test_decode_tcp_rejects_short_frame() {
        assert!(decode_tcp(&[]).is_err());
        // A frame claiming Len=13 but providing no extended length byte
        assert!(decode_tcp(&[0xD0]).is_err());
    }

    // ── read_tcp_frame round-trip ──────────────────────────────────────────

    #[tokio::test]
    async fn test_read_tcp_frame() {
        let token = [0x01, 0x02];
        let payload = b"world";
        let udp = make_udp_bytes(2, 0x45, &token, payload);
        let tcp = encode_tcp(&udp).unwrap();

        // Feed the TCP bytes into an in-memory pipe.
        let (mut writer, mut reader) = tokio::io::duplex(1024);
        writer.write_all(&tcp).await.unwrap();
        drop(writer);

        let frame = read_tcp_frame(&mut reader).await.unwrap();
        assert_eq!(frame, tcp);
        let recovered = decode_tcp(&frame).unwrap();
        assert_eq!(recovered, udp);
    }

    // ── integration: TCP server + client ──────────────────────────────────

    async fn echo_request_handler(
        mut req: Box<CoapRequest<SocketAddr>>,
    ) -> Box<CoapRequest<SocketAddr>> {
        if let Some(ref mut resp) = req.response {
            let payload = req
                .message
                .get_option(CoapOption::UriPath)
                .and_then(|l| l.front().cloned())
                .unwrap_or_default();
            resp.message.payload = payload;
        }
        req
    }

    fn spawn_tcp_server<F, Fut>(addr: &'static str, handler: F) -> mpsc::UnboundedReceiver<u16>
    where
        F: Fn(Box<CoapRequest<SocketAddr>>) -> Fut + Send + Sync + 'static,
        Fut: Future<Output = Box<CoapRequest<SocketAddr>>> + Send,
    {
        let (tx, rx) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            let listener = TcpCoapListener::new(addr).await.unwrap();
            let port = listener.local_addr().unwrap().port();
            let server = Server::from_listeners(vec![Box::new(listener)]);
            tx.send(port).unwrap();
            server.run(handler).await.unwrap();
        });
        rx
    }

    #[tokio::test]
    async fn test_tcp_echo() {
        let mut port_rx = spawn_tcp_server("127.0.0.1:0", echo_request_handler);
        let port = port_rx.recv().await.unwrap();

        let client = TcpCoAPClient::new(format!("127.0.0.1:{}", port))
            .await
            .unwrap();
        let request = RequestBuilder::new("/tcp-echo", Method::Get)
            .confirmable(false)
            .build();
        let response = client.send(request).await.unwrap();
        assert_eq!(response.message.payload, b"tcp-echo".to_vec());
    }

    #[tokio::test]
    async fn test_tcp_multiple_requests() {
        let mut port_rx = spawn_tcp_server("127.0.0.1:0", echo_request_handler);
        let port = port_rx.recv().await.unwrap();

        let client = TcpCoAPClient::new(format!("127.0.0.1:{}", port))
            .await
            .unwrap();

        for path in &["first", "second", "third"] {
            let request = RequestBuilder::new(path, Method::Get)
                .confirmable(false)
                .build();
            let response = client.send(request).await.unwrap();
            assert_eq!(response.message.payload, path.as_bytes().to_vec());
        }
    }
}
