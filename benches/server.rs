#![feature(test, async_closure)]

extern crate test;

use std::{
    thread,
    sync::mpsc,
};
use coap::{Server, CoAPClient, CoAPRequest, IsMessage, MessageType, CoAPOption};
use tokio::runtime::Runtime;

#[bench]
fn bench_server_with_request(b: &mut test::Bencher) {
    let (tx, rx) = mpsc::channel();

    thread::spawn(move || {
        Runtime::new().unwrap().block_on(async move {
            let mut server = Server::new("127.0.0.1:0").unwrap();

            tx.send(server.socket_addr().unwrap().port()).unwrap();

            server.run(async move |request| {
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
    
    let server_port = rx.recv().unwrap();
    let client = CoAPClient::new(format!("127.0.0.1:{}", server_port)).unwrap();

    let mut request = CoAPRequest::new();
    request.set_version(1);
    request.set_type(MessageType::Confirmable);
    request.set_code("0.01");
    request.set_message_id(1);
    request.set_token(vec!(0x51, 0x55, 0x77, 0xE8));
    request.add_option(CoAPOption::UriPath, "test".to_string().into_bytes());

    b.iter(|| {
        client.send(&request).unwrap();
        let recv_packet = client.receive().unwrap();
        assert_eq!(recv_packet.message.payload, b"test".to_vec());
    });
}