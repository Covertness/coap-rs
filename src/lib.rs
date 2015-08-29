//! Implementation of the [CoAP Protocol][spec].
//!
//! This library provides both a client interface (`CoAPClient`) and a server interface (`CoAPServer`).
//!
//! [spec]: https://tools.ietf.org/html/rfc7252
//! 
//! # Installation
//! 
//! First add this to your `Cargo.toml`:
//! 
//! ```toml
//! [dependencies]
//! coap = "0.2.1"
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
//! use coap::packet::*;
//! use coap::{CoAPServer, CoAPClient};

//! fn request_handler(req: Packet, _resp: CoAPClient) {
//! 	println!("Receive request: {:?}", req);
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
//! use coap::packet::*;
//! use coap::CoAPClient;
//!
//! fn main() {
//! 	let url = "coap://127.0.0.1:5683/Rust";
//! 	println!("Client request: {}", url);
//!
//! 	let response: Packet = CoAPClient::request(url).unwrap();
//! 	println!("Server reply: {}", String::from_utf8(response.payload).unwrap());
//! }
//! ```

extern crate bincode;
extern crate rustc_serialize;
extern crate bytes;
extern crate mio;
extern crate threadpool;
extern crate url;
extern crate num;
extern crate rand;
#[cfg(test)] extern crate quickcheck;

pub use server::CoAPServer;
pub use client::CoAPClient;

pub mod packet;
pub mod client;
pub mod server;