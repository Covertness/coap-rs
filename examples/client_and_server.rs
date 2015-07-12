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
	let addr = "127.0.0.1:5683";
	let request = "test";

	let mut server = CoAPServer::new(addr).unwrap();
	server.handle(request_handler).unwrap();
		
	let client = CoAPClient::new(addr).unwrap();
	let mut packet = Packet::new();
	packet.header.set_version(1);
	packet.header.set_type(PacketType::Confirmable);
	packet.header.set_code("0.01");
	packet.header.set_message_id(1);
	packet.set_token(vec!(0x51, 0x55, 0x77, 0xE8));
	packet.add_option(OptionType::UriPath, request.to_string().into_bytes());
	client.send(&packet).unwrap();
	println!("Client request: coap://{}/{}", addr, request);

	let response = client.receive().unwrap();
	println!("Server reply: {}", String::from_utf8(response.payload).unwrap());
}