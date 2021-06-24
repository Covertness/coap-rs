extern crate coap;

use coap::CoAPClient;
use std::io;
use std::io::ErrorKind;

fn main() {
    println!("GET url:");
    example_get();

    println!("POST data to url:");
    example_post();

    println!("PUT data to url:");
    example_put();

    println!("DELETE url:");
    example_delete();

    println!("Observing:");
    example_observe();
}

fn example_get() {
    let url = "coap://127.0.0.1:5683/hello/get";
    println!("Client request: {}", url);

    match CoAPClient::get(url) {
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

fn example_post() {
    let url = "coap://127.0.0.1:5683/hello/post";
    let data = b"data".to_vec();
    println!("Client request: {}", url);

    match CoAPClient::post(url, data) {
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

fn example_put() {
    let url = "coap://127.0.0.1:5683/hello/put";
    let data = b"data".to_vec();
    println!("Client request: {}", url);

    match CoAPClient::put(url, data) {
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

fn example_delete() {
    let url = "coap://127.0.0.1:5683/hello/delete";
    println!("Client request: {}", url);

    match CoAPClient::delete(url) {
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

fn example_observe() {
    let mut client = CoAPClient::new("127.0.0.1:5683").unwrap();
    client
        .observe("/hello/put", |msg| {
            println!(
                "resource changed {}",
                String::from_utf8(msg.payload).unwrap()
            );
        })
        .unwrap();

    println!("Press any key to stop...");

    io::stdin().read_line(&mut String::new()).unwrap();
}
