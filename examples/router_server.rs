extern crate coap;

use coap::{
    router::{
        extract::{Body, Path, Query, State},
        Router,
    },
    Server,
};
use serde::Deserialize;
use std::{collections::HashMap, sync::Arc};
use tokio::sync::Mutex;

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
    println!("get_temperature: {room}");
    let state = state.lock().await;

    if let Some(temp) = state.rooms.get(&room) {
        format!("Temperature in {room}: {temp}")
    } else {
        format!("Room {} not found", room)
    }
}

async fn set_temperature(
    Path(room): Path<String>,
    Body(temp): Body<f64>,
    State(state): State<RoomMutex>,
) -> String {
    println!("set_temperature: {:?}", room);
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
        .get("/temperature", get_temperature)
        .post("/temperature/{room}", set_temperature);

    let server = Server::new_udp(addr).unwrap();
    println!("Server up on {addr}");

    server.serve(router, state).await;
}
