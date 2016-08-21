#![feature(test)]

extern crate test;
extern crate coap;

use test::Bencher;
use coap::{CoAPClient, CoAPRequest, IsMessage, MessageType, CoAPOption};

#[bench]
fn bench_client_request(b: &mut Bencher) {
	let addr = "127.0.0.1:5683";
	let endpoint = "test";

	let mut request = CoAPRequest::new();
	request.set_version(1);
	request.set_type(MessageType::Confirmable);
	request.set_code("0.01");
	request.set_message_id(1);
	request.set_token(vec!(0x51, 0x55, 0x77, 0xE8));
	request.add_option(CoAPOption::UriPath, endpoint.to_string().into_bytes());

	b.iter(|| {
		let client = CoAPClient::new(addr).unwrap();
		client.send(&request).unwrap();
		client.receive().unwrap();
	});
}