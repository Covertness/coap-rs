extern crate coap;

use std::io;
use coap::{CoAPServer, CoAPResponse, CoAPRequest, IsMessage, Method};

fn request_handler(request: CoAPRequest) -> Option<CoAPResponse> {
	match request.get_method() {
		&Method::Get => println!("request by get {}", request.get_path()),
		&Method::Post => println!("request by post {}", String::from_utf8(request.message.payload).unwrap()),
		&Method::Put => println!("request by put {}", String::from_utf8(request.message.payload).unwrap()),
		_ => println!("request by other method"),
	};

	return match request.response {
		Some(mut message) => {
			message.set_payload(b"OK".to_vec());
			Some(message)
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
