extern crate coap;

use coap::{CoAPServer, CoAPClient, CoAPRequest, CoAPResponse, CoAPOption};
use coap::IsMessage;

fn request_handler(request: CoAPRequest) -> Option<CoAPResponse> {
    let uri_path = request.get_option(CoAPOption::UriPath).unwrap();

    return match request.response {
        Some(mut response) => {
            response.set_payload(uri_path.front().unwrap().clone());
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

    let response: CoAPResponse = CoAPClient::request(url).unwrap();
    println!("Server reply: {}",
             String::from_utf8(response.message.payload).unwrap());
}
