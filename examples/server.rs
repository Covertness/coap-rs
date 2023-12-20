extern crate coap;

use std::net::SocketAddr;

use coap::Server;
use coap_lite::CoapRequest;
use coap_lite::RequestType as Method;
use tokio::runtime::Runtime;

fn main() {
    let addr = "127.0.0.1:5683";

    Runtime::new().unwrap().block_on(async move {
        let server = Server::new_udp(addr).unwrap();
        println!("Server up on {}", addr);

        server
            .run(|mut request: Box<CoapRequest<SocketAddr>>| async move {
                match request.get_method() {
                    &Method::Get => println!("request by get {}", request.get_path()),
                    &Method::Post => println!(
                        "request by post {}",
                        String::from_utf8(request.message.payload.clone()).unwrap()
                    ),
                    &Method::Put => println!(
                        "request by put {}",
                        String::from_utf8(request.message.payload.clone()).unwrap()
                    ),
                    _ => println!("request by other method"),
                };

                match request.response {
                    Some(ref mut message) => {
                        message.message.payload = b"OK".to_vec();
                    }
                    _ => {}
                };
                return request;
            })
            .await
            .unwrap();
    });
}
