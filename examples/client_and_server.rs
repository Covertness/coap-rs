extern crate coap;

use std::thread;
use coap::{Server, CoAPClient};
use coap::IsMessage;
use tokio::runtime::Runtime;

fn main() {
    let mut server = Server::new("127.0.0.1:5683").unwrap();

    thread::spawn(move || {
		Runtime::new().unwrap().block_on(async move {
			server.run(move |request| {
                let uri_path = request.get_path().to_string();

                return match request.response {
                    Some(mut response) => {
                        response.set_payload(uri_path.as_bytes().to_vec());
                        Some(response)
                    }
                    _ => None,
                };
            }).await.unwrap();
		});
	});

    let url = "coap://127.0.0.1:5683/Rust";
    println!("Client request: {}", url);

    let response = CoAPClient::get(url).unwrap();
    println!("Server reply: {}",
             String::from_utf8(response.message.payload).unwrap());
}
