extern crate coap;

// use coap::client::ObserveMessage;
use coap::UdpCoAPClient;
// use std::io;
use std::io::ErrorKind;

#[tokio::main]
async fn main() {
    println!("GET url:");
    example_get().await;

    println!("POST data to url:");
    example_post().await;

    println!("GET url again:");
    example_get().await;
}

async fn example_get() {
    let url = "coap://127.0.0.1:5683/temperature?room=kitchen";
    println!("Client request: {}", url);

    match UdpCoAPClient::get(url).await {
        Ok(response) => {
            println!(
                "Server reply: {}",
                String::from_utf8(response.message.payload).unwrap()
            );
        }
        Err(e) => {
            match e.kind() {
                ErrorKind::WouldBlock => println!("Request timeout"), // Unix
                ErrorKind::TimedOut => println!("Request timeout"),   // Windows
                _ => println!("Request error: {:?}", e),
            }
        }
    }
}

async fn example_post() {
    let url = "coap://127.0.0.1:5683/temperature/kitchen";
    let data = b"21.0".to_vec();
    println!("Client request: {}", url);

    match UdpCoAPClient::post(url, data).await {
        Ok(response) => {
            println!(
                "Server reply: {}",
                String::from_utf8(response.message.payload).unwrap()
            );
        }
        Err(e) => {
            match e.kind() {
                ErrorKind::WouldBlock => println!("Request timeout"), // Unix
                ErrorKind::TimedOut => println!("Request timeout"),   // Windows
                _ => println!("Request error: {:?}", e),
            }
        }
    }
}
