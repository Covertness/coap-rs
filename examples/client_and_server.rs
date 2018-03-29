extern crate coap;

use coap::{CoAPServer, CoAPClient, CoAPRequest, CoAPResponse};
use coap::IsMessage;

fn request_handler(request: CoAPRequest) -> Option<CoAPResponse> {
    let uri_path = request.get_path().to_string();

    return match request.response {
        Some(mut response) => {
            response.set_payload(uri_path.as_bytes().to_vec());
            Some(response)
        }
        _ => None,
    };
}

fn main() {
    let mut server = CoAPServer::new("127.0.0.1:5683").unwrap();
    server.handle(request_handler).unwrap();

    let url = "coap://127.0.0.1:5683/Rust";
    println!("Client request: {}", url);

    let response = CoAPClient::get(url).unwrap();
    println!("Server reply: {}",
             String::from_utf8(response.message.payload).unwrap());
}
