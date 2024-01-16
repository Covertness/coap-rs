//! Implementation of the [CoAP Protocol][spec].
//!
//! This library provides both a client interface (`CoAPClient`)
//!   and a server interface (`CoAPServer`).
//!
//! Features:
//! - CoAP core protocol [RFC 7252](https://tools.ietf.org/rfc/rfc7252.txt)
//! - CoAP Observe option [RFC 7641](https://tools.ietf.org/rfc/rfc7641.txt)
//! - *Too Many Requests* Response Code [RFC 8516](https://tools.ietf.org/html/rfc8516)
//! - Block-Wise Transfers [RFC 7959](https://tools.ietf.org/html/rfc7959)
//! - DTLS support via [webrtc-rs](https://github.com/webrtc-rs/webrtc)
//! - Option to provide custom transports for client and server
//!
//! Missing features:
//! - Client receiving piggybacked requests
//!
//! # Installation
//!
//! First add this to your `Cargo.toml`:
//!
//! ```toml
//! [dependencies]
//! coap = "0.14"
//! coap-lite = "0.11.3"
//! tokio = {version = "^1.32", features = ["full"]}
//! ```
//!
//! Then, add this to your crate root:
//!
//! ```
//! extern crate coap;
//! ```
//!
//! # Example
//!
//! ## Server:
//! ```no_run
//! extern crate coap;
//!
//! use coap_lite::{RequestType as Method, CoapRequest};
//! use coap::Server;
//! use tokio::runtime::Runtime;
//! use std::net::SocketAddr;
//! fn main() {
//!     let addr = "127.0.0.1:5683";
//! 	Runtime::new().unwrap().block_on(async move {
//!         let mut server = Server::new_udp(addr).unwrap();
//!         println!("Server up on {}", addr);
//!
//!         server.run(|mut request: Box<CoapRequest<SocketAddr>>| async {
//!             match request.get_method() {
//!                 &Method::Get => println!("request by get {}", request.get_path()),
//!                 &Method::Post => println!("request by post {}", String::from_utf8(request.message.payload.clone()).unwrap()),
//!                 &Method::Put => println!("request by put {}", String::from_utf8(request.message.payload.clone()).unwrap()),
//!                 _ => println!("request by other method"),
//!             };

//!             match request.response {
//!                 Some(ref mut message) => {
//!                     message.message.payload = b"OK".to_vec();

//!                 },
//!                 _ => {}
//!             };
//!             return request
//!         }).await.unwrap();
//!     });
//! }
//! ```
//!
//! ## Client:
//! ```no_run
//! extern crate coap;
//!
//! use coap_lite::{RequestType as Method, CoapRequest};
//! use coap::{UdpCoAPClient};
//! use tokio::main;
//! #[tokio::main]
//! async fn main() {
//!     let url = "coap://127.0.0.1:5683/Rust";
//!     println!("Client request: {}", url);
//!
//!     let response = UdpCoAPClient::get(url).await.unwrap();
//!     println!("Server reply: {}", String::from_utf8(response.message.payload).unwrap());
//! }

#[macro_use]
extern crate alloc;

#[cfg(test)]
extern crate quickcheck;

pub use self::client::UdpCoAPClient;
pub use self::observer::Observer;
pub use self::server::Server;
pub mod client;
#[cfg(feature = "dtls")]
pub mod dtls;
mod observer;
pub mod server;
