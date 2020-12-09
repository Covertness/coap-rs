extern crate coap;

use std::io;
use std::io::ErrorKind;
use coap_lite::{RequestType as Method, CoapRequest};
use coap::CoAPClient;

fn main() {
    println!("Request by GET:");
    example_get();

    println!("Request by POST:");
    example_post();

    println!("PUT data:");
    example_put();

    println!("Observing:");
    example_observe();
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

    let mut request = CoapRequest::new();
    request.set_method(Method::Post);
    request.set_path(path);
    request.message.payload = b"data".to_vec();

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

fn example_put() {
    let addr = "127.0.0.1:5683";
    let path = "/hello/put";

    let mut request = CoapRequest::new();
    request.set_method(Method::Put);
    request.set_path(path);
    request.message.payload = b"data".to_vec();

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

fn example_observe() {
    let mut client = CoAPClient::new("127.0.0.1:5683").unwrap();
    client.observe("/hello/put", |msg| {
        println!("resource changed {}", String::from_utf8(msg.payload).unwrap());
    }).unwrap();

    println!("Press any key to stop...");

    io::stdin().read_line(&mut String::new()).unwrap();
}