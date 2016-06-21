extern crate coap;

use std::io;
use coap::packet::*;
use coap::CoAPServer;

fn request_handler(_: Packet, response: Option<Packet>) -> Option<Packet> {
	return match response {
		Some(mut packet) => {
			packet.set_payload(b"OK".to_vec());
			Some(packet)
		},
		_ => None
	};
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
