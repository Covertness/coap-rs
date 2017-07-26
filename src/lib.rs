//! Implementation of the [CoAP Protocol][spec].
//!
//! This library provides both a client interface (`CoAPClient`)
//!   and a server interface (`CoAPServer`).
//!
//! [spec]: https://tools.ietf.org/html/rfc7252
//!
//! # Installation
//!
//! First add this to your `Cargo.toml`:
//!
//! ```toml
//! [dependencies]
//! coap = "0.5"
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
//! use coap::{CoAPServer, CoAPClient, CoAPRequest, CoAPResponse};

//! fn request_handler(request: CoAPRequest) -> Option<CoAPResponse> {
//! 	println!("Receive request: {:?}", request);
//!     
//!     // Return the auto-generated response
//!     request.response
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
//! use coap::message::response::CoAPResponse;
//! use coap::CoAPClient;
//!
//! fn main() {
//! 	let url = "coap://127.0.0.1:5683/Rust";
//! 	println!("Client request: {}", url);
//!
//! 	let response: CoAPResponse = CoAPClient::request(url).unwrap();
//! 	println!("Server reply: {}", String::from_utf8(response.message.payload).unwrap());
//! }
//! ```

extern crate bincode;
extern crate rustc_serialize;
extern crate mio;
extern crate url;
extern crate num;
extern crate rand;
extern crate threadpool;
#[macro_use] extern crate enum_primitive;

#[cfg(test)]
extern crate quickcheck;

#[macro_use]
extern crate log;

pub use client::CoAPClient;
pub use message::header::MessageType;
pub use message::IsMessage;
pub use message::packet::CoAPOption;
pub use message::request::CoAPRequest;
pub use message::response::CoAPResponse;
pub use server::CoAPServer;

pub mod message;
pub mod client;
pub mod server;
