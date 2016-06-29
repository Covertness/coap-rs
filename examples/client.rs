extern crate coap;

use std::io::ErrorKind;
use coap::{CoAPClient, CoAPRequest, IsMessage, MessageType, CoAPOption};

fn main() {
    let addr = "127.0.0.1:5683";
    let endpoint = "test";

    let client = CoAPClient::new(addr).unwrap();
    let mut request = CoAPRequest::new();
    request.set_version(1);
    request.set_type(MessageType::Confirmable);
    request.set_code("0.01");
    request.set_message_id(1);
    request.set_token(vec![0x51, 0x55, 0x77, 0xE8]);
    request.add_option(CoAPOption::UriPath, endpoint.to_string().into_bytes());
    client.send(&request).unwrap();
    println!("Client request: coap://{}/{}", addr, endpoint);

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
