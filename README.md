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
- DTLS support via [webrtc-rs](https://github.com/webrtc-rs/webrtc)
- Option to provide custom transports for client and server

[Documentation](https://docs.rs/coap/)

## Installation

First add this to your `Cargo.toml`:

```toml
[dependencies]
coap = "0.26"
coap-lite = "0.13.3"
tokio = {version = "^1.32", features = ["full"]}
```

## Example

### Server:
```rust
use coap::{
    router::{
        extract::{Json, Path, Query, State},
        get, post, Router,
    },
    Server,
};
use serde::Deserialize;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::Mutex;

#[derive(Debug, Clone, Default)]
pub struct RoomState {
    rooms: HashMap<String, f64>,
}

pub type RoomMutex = Arc<Mutex<RoomState>>;

#[derive(Debug, Deserialize)]
pub struct QueryArgs {
    room: String,
}

async fn get_temperature(
    Query(QueryArgs { room }): Query<QueryArgs>,
    state: State<RoomMutex>,
) -> String {
    let state = state.lock().await;
    if let Some(temp) = state.rooms.get(&room) {
        format!("Temperature in {room}: {temp}")
    } else {
        format!("Room {} not found", room)
    }
}

async fn set_temperature(
    Path(room): Path<String>,
    Json(temp): Json<f64>,
    State(state): State<RoomMutex>,
) -> String {
    let mut state = state.lock().await;
    state.rooms.insert(room, temp);
    "OK".to_string()
}

#[tokio::main]
async fn main() {
    let addr = "127.0.0.1:5683";

    let state = Arc::new(Mutex::new(RoomState {
        rooms: HashMap::new(),
    }));

    let router = Router::new()
        .route("/temperature", get(get_temperature))
        .route("/temperature/{room}", post(set_temperature))
        .with_state(state);

    let server = Server::new_udp(addr).unwrap();
    println!("Server up on {addr}");

    server.serve(router).await.unwrap();
}
```

### Client:
```rust
use coap::UdpCoAPClient;

#[tokio::main]
async fn main() {
    let url = "coap://127.0.0.1:5683/temperature?room=kitchen";
    println!("Client request: {}", url);

    let response = UdpCoAPClient::get(url).await.unwrap();
    println!("Server reply: {}", String::from_utf8(response.message.payload).unwrap());
}
```

## Benchmark
```bash
$ cargo bench
```
