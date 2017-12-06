extern crate coap;

use std::io;
use coap::{CoAPServer, CoAPResponse, CoAPRequest, IsMessage};

fn request_handler(request: CoAPRequest) -> Option<CoAPResponse> {
	let request_code = request.get_code();

	match request_code.as_ref() {
		"0.01" => {
			println!("request by get");
		},
		"0.02" => {
			println!("request by post");
			println!("request body: {}", String::from_utf8(request.message.payload).unwrap());
		},
		_ => {
			println!("request by other method {}", request_code);
		}
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
