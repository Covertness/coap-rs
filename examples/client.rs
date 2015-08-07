extern crate coap;

use coap::packet::*;
use coap::CoAPClient;

fn main() {
	let addr = "127.0.0.1:5683";
	let request = "test";

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

	match client.receive() {
		Ok(response) => {
			println!("Server reply: {}", String::from_utf8(response.payload).unwrap());
		},
		Err(e) => {
			println!("Request error: {:?}", e);
		}
	}
}