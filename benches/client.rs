#![feature(test)]

extern crate test;
extern crate coap;

use test::Bencher;
use coap::packet::*;
use coap::CoAPClient;

#[bench]
fn bench_client_request(b: &mut Bencher) {
	let addr = "127.0.0.1:5683";
	let request = "test";
	let mut packet = Packet::new();
	packet.header.set_version(1);
	packet.header.set_type(MessageType::Confirmable);
	packet.header.set_code("0.01");
	packet.header.set_message_id(1);
	packet.set_token(vec!(0x51, 0x55, 0x77, 0xE8));
	packet.add_option(CoAPOption::UriPath, request.to_string().into_bytes());

	b.iter(|| {
		let client = CoAPClient::new(addr).unwrap();
		client.send(&packet).unwrap();
	});
}