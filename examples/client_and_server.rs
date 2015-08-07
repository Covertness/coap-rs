extern crate coap;

use coap::packet::*;
use coap::{CoAPServer, CoAPClient};

fn request_handler(req: Packet, resp: CoAPClient) {
	let uri_path = req.get_option(OptionType::UriPath);
	assert!(uri_path.is_some());
	let uri_path = uri_path.unwrap();
	let mut packet = Packet::new();

	packet.header.set_version(1);
	packet.header.set_type(PacketType::Acknowledgement);
	packet.header.set_code("2.05");
	packet.header.set_message_id(req.header.get_message_id());
	packet.set_token(req.get_token().clone());
	packet.payload = uri_path.front().unwrap().clone();
	resp.send(&packet).unwrap();
}

fn main() {
	let mut server = CoAPServer::new("127.0.0.1:5683").unwrap();
	server.handle(request_handler).unwrap();
		
	let url = "coap://127.0.0.1:5683/Rust";
	println!("Client request: {}", url);

	let response: Packet = CoAPClient::request(url).unwrap();
	println!("Server reply: {}", String::from_utf8(response.payload).unwrap());
}