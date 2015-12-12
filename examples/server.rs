extern crate coap;

use std::io;
use coap::packet::*;
use coap::{CoAPServer, CoAPClient};

fn request_handler(req: Packet, resp: CoAPClient) {
	println!("Receive request: {:?}", req);
	resp.reply(&req, b"OK".to_vec()).unwrap();
}

fn main() {
	let addr = "127.0.0.1:5683";

	let mut server = CoAPServer::new(addr).unwrap();
	server.handle(request_handler).unwrap();
		
	println!("Server up on {}", addr);
	println!("Press any key to stop...");

	io::stdin().read_line(&mut String::new()).unwrap();

	println!("Server shutdown");
}