#![feature(test, async_closure)]
extern crate test;

use std::net::SocketAddr;

use coap::{server::UdpCoapListener, Server, UdpCoAPClient};
use coap_lite::{CoapOption, CoapRequest, MessageType};
use tokio::{net::UdpSocket, runtime::Runtime};

#[bench]
fn bench_server_with_request(b: &mut test::Bencher) {
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();

    let rt = Runtime::new().unwrap();
    rt.spawn(async move {
        let sock = UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let addr = sock.local_addr().unwrap();
        let listener = Box::new(UdpCoapListener::from_socket(sock));
        let server = Server::from_listeners(vec![listener]);

        tx.send(addr.port()).unwrap();

        server
            .run(move |mut request: Box<CoapRequest<SocketAddr>>| async {
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

    let server_port = rx.blocking_recv().unwrap();
    let mut client = rt.block_on(async {
        UdpCoAPClient::new_udp(format!("127.0.0.1:{}", server_port))
            .await
            .unwrap()
    });

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
        rt.block_on(async {
            client.send_raw_request(&request).await.unwrap();
            let recv_packet = client.receive_raw_response().await.unwrap();
            assert_eq!(recv_packet.message.payload, b"test".to_vec());
        });
    });
}
