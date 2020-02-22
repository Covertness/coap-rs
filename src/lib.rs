//! Implementation of the [CoAP Protocol][spec].
//!
//! This library provides both a client interface (`CoAPClient`)
//!   and a server interface (`CoAPServer`).
//!
//! Features:
//! - CoAP core protocol [RFC 7252](https://tools.ietf.org/rfc/rfc7252.txt)
//! - CoAP Observe option [RFC 7641](https://tools.ietf.org/rfc/rfc7641.txt)
//! - *Too Many Requests* Response Code [RFC 8516](https://tools.ietf.org/html/rfc8516)
//!
//! # Installation
//!
//! First add this to your `Cargo.toml`:
//!
//! ```toml
//! [dependencies]
//! coap = "0.8"
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
//! #![feature(async_closure)]
//!
//! extern crate coap;
//!
//! use coap::{Server, IsMessage, Method};
//! use tokio::runtime::Runtime;

//! fn main() {
//!     let addr = "127.0.0.1:5683";

//! 	Runtime::new().unwrap().block_on(async move {
//!         let mut server = Server::new(addr).unwrap();
//!         println!("Server up on {}", addr);
//! 
//!         server.run(async move |request| {
//!             match request.get_method() {
//!                 &Method::Get => println!("request by get {}", request.get_path()),
//!                 &Method::Post => println!("request by post {}", String::from_utf8(request.message.payload).unwrap()),
//!                 &Method::Put => println!("request by put {}", String::from_utf8(request.message.payload).unwrap()),
//!                 _ => println!("request by other method"),
//!             };
            
//!             return match request.response {
//!                 Some(mut message) => {
//!                     message.set_payload(b"OK".to_vec());
//!                     Some(message)
//!                 },
//!                 _ => None
//!             };
//!         }).await.unwrap();
//!     });
//! }
//! ```
//!
//! ## Client:
//! ```no_run
//! extern crate coap;
//!
//! use coap::{CoAPClient, CoAPResponse};
//!
//! fn main() {
//!     let url = "coap://127.0.0.1:5683/Rust";
//!     println!("Client request: {}", url);
//!
//!     let response = CoAPClient::get(url).unwrap();
//!     println!("Server reply: {}", String::from_utf8(response.message.payload).unwrap());
//! }
//! ```

#[cfg(test)]
extern crate quickcheck;

pub use self::client::CoAPClient;
pub use self::message::header::MessageType;
pub use self::message::IsMessage;
pub use self::message::packet::CoAPOption;
pub use self::message::request::CoAPRequest;
pub use self::message::request::Method;
pub use self::message::response::CoAPResponse;
pub use self::message::response::Status;
pub use self::observer::Observer;
pub use self::server::{Server, CoAPServer};
pub mod message;
pub mod client;
pub mod server;
mod observer;



