extern crate coap;

use coap::packet::*;
use coap::{CoAPServer, CoAPClient};

fn request_handler(req: Packet) -> Option<Packet> {
	let uri_path = req.get_option(OptionType::UriPath).unwrap();

	return match coap::packet::auto_response(req) {
	Ok(mut response) => {
		response.set_payload(uri_path.front().unwrap().clone());
		Some(response)
	},
	Err(_) => None
    };
}

fn main() {
	let mut server = CoAPServer::new("127.0.0.1:5683").unwrap();
	server.handle(request_handler).unwrap();

	let url = "coap://127.0.0.1:5683/Rust";
	println!("Client request: {}", url);

	let response: Packet = CoAPClient::request(url).unwrap();
	println!("Server reply: {}", String::from_utf8(response.payload).unwrap());
}
