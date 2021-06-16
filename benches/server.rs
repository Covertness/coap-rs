#![feature(test, async_closure)]

extern crate test;

use coap::{CoAPClient, Server};
use coap_lite::{CoapOption, CoapRequest, MessageType};
use std::{sync::mpsc, thread};
use tokio::runtime::Runtime;

#[bench]
fn bench_server_with_request(b: &mut test::Bencher) {
    let (tx, rx) = mpsc::channel();

    thread::spawn(move || {
        Runtime::new().unwrap().block_on(async move {
            let mut server = Server::new("127.0.0.1:0").unwrap();

            tx.send(server.socket_addr().unwrap().port()).unwrap();

            server
                .run(async move |request| {
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

    let server_port = rx.recv().unwrap();
    let client = CoAPClient::new(format!("127.0.0.1:{}", server_port)).unwrap();

    let mut request = CoapRequest::new();
    request.message.header.set_version(1);
    request.message.header.set_type(MessageType::Confirmable);
    request.message.header.set_code("0.01");
    request.message.header.message_id = 1;
    request.message.set_token(vec![0x51, 0x55, 0x77, 0xE8]);
    request
        .message
        .add_option(CoapOption::UriPath, "test".to_string().into_bytes());

    b.iter(|| {
        client.send(&request).unwrap();
        let recv_packet = client.receive().unwrap();
        assert_eq!(recv_packet.message.payload, b"test".to_vec());
    });
}
