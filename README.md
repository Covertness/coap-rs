# coap-rs

[![Travis Build Status](https://travis-ci.org/Covertness/coap-rs.svg?branch=master)](https://travis-ci.org/Covertness/coap-rs)
[![Windows Build Status](https://ci.appveyor.com/api/projects/status/ic36jdu4xy6doc59?svg=true)](https://ci.appveyor.com/project/Covertness/coap-rs)
![Downloads](https://img.shields.io/crates/d/coap.svg?style=flat)
[![](https://meritbadge.herokuapp.com/coap)](https://crates.io/crates/coap)
[![Coverage Status](https://coveralls.io/repos/github/Covertness/coap-rs/badge.svg?branch=master)](https://coveralls.io/github/Covertness/coap-rs?branch=master)
[![MIT licensed](https://img.shields.io/badge/license-MIT-blue.svg)](./LICENSE)

A fast and stable [Constrained Application Protocol(CoAP)](https://tools.ietf.org/html/rfc7252) library implemented in Rust.

Features:
- CoAP core protocol [RFC 7252](https://tools.ietf.org/rfc/rfc7252.txt)
- CoAP Observe option [RFC 7641](https://tools.ietf.org/rfc/rfc7641.txt)
- *Too Many Requests* Response Code [RFC 8516](https://tools.ietf.org/html/rfc8516)

[Documentation](http://covertness.github.io/coap-rs/master/coap/index.html)

## Installation

First add this to your `Cargo.toml`:

```toml
[dependencies]
coap = "0.7"
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
use coap::{CoAPServer, CoAPResponse, CoAPRequest, Method};

fn request_handler(request: CoAPRequest) -> Option<CoAPResponse> {
    match request.get_method() {
		&Method::Get => println!("request by get {}", request.get_path()),
		&Method::Post => println!("request by post {}", String::from_utf8(request.message.payload).unwrap()),
		_ => println!("request by other method"),
	};

    // Return the auto-generated response
    request.response
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

use coap::{CoAPClient, CoAPResponse};

fn main() {
    let url = "coap://127.0.0.1:5683/Rust";
    println!("Client request: {}", url);

    let response = CoAPClient::get(url).unwrap();
    println!("Server reply: {}", String::from_utf8(response.message.payload).unwrap());
}
```

## Benchmark
```bash
$ cargo run --example server
$ cargo bench
```
