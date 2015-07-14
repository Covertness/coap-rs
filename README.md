# coap-rs

[![crates.io](http://meritbadge.herokuapp.com/coap)](https://crates.io/crates/coap)
[![Travis Build Status](https://travis-ci.org/Covertness/coap-rs.svg?branch=master)](https://travis-ci.org/Covertness/coap-rs)
[![Windows Build Status](https://ci.appveyor.com/api/projects/status/ic36jdu4xy6doc59?svg=true)](https://ci.appveyor.com/project/Covertness/coap-rs)
[![MIT licensed](https://img.shields.io/badge/license-MIT-blue.svg)](./LICENSE)

A [Constrained Application Protocol(CoAP)](https://tools.ietf.org/html/rfc7252) library implemented in Rust.

[Documentation](http://covertness.github.io/coap-rs/coap/index.html)

## Installation

First add this to your `Cargo.toml`:

```toml
[dependencies]
coap = "0.1.0"
```

Then, add this to your crate root:

```rust
extern crate coap;
```

## Example

### Server:
```rust
extern crate coap;

use std::io;
use coap::packet::*;
use coap::{CoAPServer, CoAPClient};

fn request_handler(req: Packet, _resp: CoAPClient) {
	println!("Receive request: {:?}", req);
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
```

### Client:
```rust
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

	let response = client.receive().unwrap();
	println!("Server reply: {}", String::from_utf8(response.payload).unwrap());
}
```