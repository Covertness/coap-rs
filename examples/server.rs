extern crate coap;

use std::io;
use coap::packet::*;
use coap::CoAPServer;

fn request_handler(req: Packet) -> Option<Packet> {
    return match coap::packet::auto_response(req) {
        Ok(mut response) => {
		response.set_payload(b"OK".to_vec());
		Some(response)
	},
        Err(_) => None
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
