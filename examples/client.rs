extern crate coap;

use coap::client::ObserveMessage;
use coap::UdpCoAPClient;
use std::io;
use std::io::ErrorKind;

#[tokio::main]
async fn main() {
    println!("GET url:");
    example_get().await;

    println!("POST data to url:");
    example_post().await;

    println!("PUT data to url:");
    example_put().await;

    println!("DELETE url:");
    example_delete().await;

    println!("Observing:");
    example_observe().await;
}

async fn example_get() {
    let url = "coap://127.0.0.1:5683/hello/get";
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
    let url = "coap://127.0.0.1:5683/hello/post";
    let data = b"data".to_vec();
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

async fn example_put() {
    let url = "coap://127.0.0.1:5683/hello/put";
    let data = b"data".to_vec();
    println!("Client request: {}", url);

    match UdpCoAPClient::put(url, data).await {
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

async fn example_delete() {
    let url = "coap://127.0.0.1:5683/hello/delete";
    println!("Client request: {}", url);

    match UdpCoAPClient::delete(url).await {
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

async fn example_observe() {
    let client = UdpCoAPClient::new_udp("127.0.0.1:5683").await.unwrap();
    let observe_channel = client
        .observe("/hello/put", |msg| {
            println!(
                "resource changed {}",
                String::from_utf8(msg.payload).unwrap()
            );
        })
        .await
        .unwrap();

    println!("Enter any key to stop...");

    io::stdin().read_line(&mut String::new()).unwrap();
    observe_channel.send(ObserveMessage::Terminate).unwrap();
}
