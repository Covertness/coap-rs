/// This example shows how to use DTLS with coap-rs. If you want to use PKI, please take
/// a look at the test in dtls.rs
extern crate coap;
use coap::client::CoAPClient;
use coap::dtls::DtlsConfig;
use coap::Server;
use coap_lite::{CoapRequest, RequestType as Method};
use std::future::Future;
use std::net::{SocketAddr, ToSocketAddrs};
use std::sync::Arc;
use tokio::sync::mpsc;
use webrtc_dtls::cipher_suite::CipherSuiteId;
use webrtc_dtls::config::Config;
use webrtc_dtls::listener::listen;
use webrtc_util::conn::Listener as WebRtcListener;

pub fn spawn_dtls_server<
    F: Fn(Box<CoapRequest<SocketAddr>>) -> HandlerRet + Send + Sync + 'static,
    HandlerRet,
>(
    ip: &'static str,
    request_handler: F,
    config: webrtc_dtls::config::Config,
) -> mpsc::UnboundedReceiver<u16>
where
    HandlerRet: Future<Output = Box<CoapRequest<SocketAddr>>> + Send,
{
    let (tx, rx) = mpsc::unbounded_channel();
    tokio::spawn(async move {
        let listener = listen(ip, config).await.unwrap();
        let listen_port = listener.addr().await.unwrap().port();
        let listener = Box::new(listener);
        let server = Server::from_listeners(vec![listener]);
        tx.send(listen_port).unwrap();
        server.run(request_handler).await.unwrap();
    });

    rx
}

#[tokio::main]
async fn main() {
    let config = Config {
        psk: Some(Arc::new(|_| Ok(vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 0]))),
        psk_identity_hint: Some("webrtc-rs DTLS Server".as_bytes().to_vec()),
        cipher_suites: vec![CipherSuiteId::Tls_Psk_With_Aes_128_Ccm_8],
        server_name: "localhost".to_string(),
        ..Default::default()
    };
    let server_port = spawn_dtls_server(
        "127.0.0.1:0",
        |mut req| async {
            req.response.as_mut().unwrap().message.payload = b"hello dtls".to_vec();
            req
        },
        config.clone(),
    )
    .recv()
    .await
    .unwrap();

    let dtls_config = DtlsConfig {
        config,
        dest_addr: ("127.0.0.1", server_port)
            .to_socket_addrs()
            .unwrap()
            .next()
            .unwrap(),
    };

    let mut client = CoAPClient::from_dtls_config(dtls_config)
        .await
        .expect("could not create client");
    let domain = format!("127.0.0.1:{}", server_port);
    let resp = client
        .request_path("/hello", Method::Get, None, None, Some(domain.to_string()))
        .await
        .unwrap();
    println!(
        "receive on client:  {}",
        std::str::from_utf8(&resp.message.payload).unwrap()
    );
    assert_eq!(resp.message.payload, b"hello dtls".to_vec());
}
