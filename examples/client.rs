extern crate coap;

use std::io::ErrorKind;
use coap::{CoAPClient, CoAPRequest, IsMessage, Method};

fn main() {
    println!("Request by GET:");
    example_get();

    println!("Request by POST:");
    example_post();
}


fn example_get() {
    let url = "coap://127.0.0.1:5683/hello/get";
    println!("Client request: {}", url);

    match CoAPClient::get(url) {
        Ok(response) => {
            println!("Server reply: {}",
                     String::from_utf8(response.message.payload).unwrap());
        }
        Err(e) => {
            match e.kind() {
                ErrorKind::WouldBlock => println!("Request timeout"),   // Unix
                ErrorKind::TimedOut => println!("Request timeout"),     // Windows
                _ => println!("Request error: {:?}", e),
            }
        }
    }
}

fn example_post() {
    let addr = "127.0.0.1:5683";
    let path = "/hello/post";

    let mut request = CoAPRequest::new();
    request.set_method(Method::Post);
    request.set_path(path);
    request.set_payload(b"data".to_vec());

    let client = CoAPClient::new(addr).unwrap();
    client.send(&request).unwrap();
    println!("Client request: coap://{}/{}", addr, path);

    match client.receive() {
        Ok(response) => {
            println!("Server reply: {}",
                     String::from_utf8(response.message.payload).unwrap());
        }
        Err(e) => {
            match e.kind() {
                ErrorKind::WouldBlock => println!("Request timeout"),   // Unix
                ErrorKind::TimedOut => println!("Request timeout"),     // Windows
                _ => println!("Request error: {:?}", e),
            }
        }
    }
}