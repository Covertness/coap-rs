# coap-rs

[![CircleCI](https://circleci.com/gh/Covertness/coap-rs.svg?style=svg)](https://circleci.com/gh/Covertness/coap-rs)
[![Windows Build Status](https://ci.appveyor.com/api/projects/status/ic36jdu4xy6doc59?svg=true)](https://ci.appveyor.com/project/Covertness/coap-rs)
![Downloads](https://img.shields.io/crates/d/coap.svg?style=flat)
[![Coverage Status](https://coveralls.io/repos/github/Covertness/coap-rs/badge.svg?branch=master)](https://coveralls.io/github/Covertness/coap-rs?branch=master)
[![MIT licensed](https://img.shields.io/badge/license-MIT-blue.svg)](./LICENSE)

A fast and stable [Constrained Application Protocol(CoAP)](https://tools.ietf.org/html/rfc7252) library implemented in Rust.

Features:
- CoAP core protocol [RFC 7252](https://tools.ietf.org/rfc/rfc7252.txt)
- CoAP Observe option [RFC 7641](https://tools.ietf.org/rfc/rfc7641.txt)
- *Too Many Requests* Response Code [RFC 8516](https://tools.ietf.org/html/rfc8516)
- Block-Wise Transfers [RFC 7959](https://tools.ietf.org/html/rfc7959)

[Documentation](https://docs.rs/coap/)

## Installation

First add this to your `Cargo.toml`:

```toml
[dependencies]
coap = "0.12"
```

## Example

### Server:
```rust
use coap_lite::{RequestType as Method};
use coap::Server;
use tokio::runtime::Runtime;

fn main() {
    let addr = "127.0.0.1:5683";

    Runtime::new().unwrap().block_on(async move {
        let mut server = Server::new(addr).unwrap();
        println!("Server up on {}", addr);
        
        server.run(|request| async {
            match request.get_method() {
                &Method::Get => println!("request by get {}", request.get_path()),
                &Method::Post => println!("request by post {}", String::from_utf8(request.message.payload).unwrap()),
                &Method::Put => println!("request by put {}", String::from_utf8(request.message.payload).unwrap()),
                _ => println!("request by other method"),
            };
            
            return match request.response {
                Some(mut message) => {
                    message.message.payload = b"OK".to_vec();
                    Some(message)
                },
                _ => None
            };
        }).await.unwrap();
    });
}
```

### Client:
```rust
use coap::CoAPClient;

fn main() {
    let url = "coap://127.0.0.1:5683/Rust";
    println!("Client request: {}", url);

    let response = CoAPClient::get(url).unwrap();
    println!("Server reply: {}", String::from_utf8(response.message.payload).unwrap());
}
```

## Benchmark
```bash
$ cargo bench
```
