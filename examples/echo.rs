extern crate coap;

use std::net::SocketAddr;

use coap::{Server, UDPCoAPClient};
use coap_lite::CoapRequest;
#[tokio::main]
async fn main() {
    let _server_task = tokio::spawn(async move {
        let server = Server::new_udp("127.0.0.1:5683").unwrap();
        server
            .run(|mut request: Box<CoapRequest<SocketAddr>>| async {
                let uri_path = request.get_path().to_string();

                match request.response {
                    Some(ref mut response) => {
                        response.message.payload = uri_path.as_bytes().to_vec();
                    }
                    _ => {}
                };
                return request;
            })
            .await
            .unwrap();
    });

    let url = "coap://127.0.0.1:5683/Rust";
    println!("Client request: {}", url);

    // Maybe need sleep seconds before start client on some OS: https://github.com/Covertness/coap-rs/issues/75
    let response = UDPCoAPClient::get(url).await.unwrap();
    println!(
        "Server reply: {}",
        String::from_utf8(response.message.payload).unwrap()
    );
}
