//! Implementation of the [CoAP Protocol][spec].
//!
//! This library provides both a client interface (`CoAPClient`)
//!   and a server interface (`CoAPServer`).
//!
//! Features:
//! - CoAP core protocol [RFC 7252](https://tools.ietf.org/rfc/rfc7252.txt)
//! - CoAP Observe option [RFC 7641](https://tools.ietf.org/rfc/rfc7641.txt)
//!
//! # Installation
//!
//! First add this to your `Cargo.toml`:
//!
//! ```toml
//! [dependencies]
//! coap = "0.7"
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

//! use std::io;
//! use coap::{CoAPServer, CoAPClient, CoAPRequest, CoAPResponse, Method};

//! fn request_handler(request: CoAPRequest) -> Option<CoAPResponse> {
//!     match request.get_method() {
//! 		&Method::Get => println!("request by get {}", request.get_path()),
//! 		&Method::Post => println!("request by post {}", String::from_utf8(request.message.payload).unwrap()),
//! 		_ => println!("request by other method"),
//! 	};
//!
//!     // Return the auto-generated response
//!    request.response
//! }

//! fn main() {
//! 	let addr = "127.0.0.1:5683";
//!
//! 	let mut server = CoAPServer::new(addr).unwrap();
//! 	server.handle(request_handler).unwrap();
//!
//! 	println!("Server up on {}", addr);
//!     println!("Press any key to stop...");
//!
//! 	io::stdin().read_line(&mut String::new()).unwrap();
//!
//! 	println!("Server shutdown");
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
//! 	let url = "coap://127.0.0.1:5683/Rust";
//! 	println!("Client request: {}", url);
//!
//! 	let response = CoAPClient::get(url).unwrap();
//! 	println!("Server reply: {}", String::from_utf8(response.message.payload).unwrap());
//! }
//! ```

#[cfg(test)]
extern crate quickcheck;

pub use self::client::CoAPClient;
pub use self::message::header::MessageType;
pub use self::message::IsMessage;
pub use self::message::options::CoAPOption;
pub use self::message::request::CoAPRequest;
pub use self::message::request::Method;
pub use self::message::response::CoAPResponse;
pub use self::message::response::Status;
pub use self::server::CoAPServer;
pub use block_transfer::send;
pub mod message;
pub mod client;
pub mod server;
pub mod block_transfer;
mod observer;



