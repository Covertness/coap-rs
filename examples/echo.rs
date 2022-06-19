extern crate coap;

use coap::{CoAPClient, Server};
use std::thread;
use tokio::runtime::Runtime;

fn main() {
    thread::spawn(move || {
        Runtime::new().unwrap().block_on(async move {
            let mut server = Server::new("127.0.0.1:5683").unwrap();

            server
                .run(|request| async {
                    let uri_path = request.get_path().to_string();

                    return match request.response {
                        Some(mut response) => {
                            response.message.payload = uri_path.as_bytes().to_vec();
                            Some(response)
                        }
                        _ => None,
                    };
                })
                .await
                .unwrap();
        });
    });

    let url = "coap://127.0.0.1:5683/Rust";
    println!("Client request: {}", url);

    // Maybe need sleep seconds before start client on some OS: https://github.com/Covertness/coap-rs/issues/75
    let response = CoAPClient::get(url).unwrap();
    println!(
        "Server reply: {}",
        String::from_utf8(response.message.payload).unwrap()
    );
}
