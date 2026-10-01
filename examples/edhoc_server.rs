/// This example shows how to serve EDHOC's Responder role over the router, next to a normal
/// `/hello` echo resource. Run it, then run the `edhoc_client` example to perform a handshake
/// against it and follow up with a plain echo request.
///
/// The credentials and private keys below are the RFC 9529 test vectors, used here only so the
/// example is self-contained; a real deployment must use its own keys.
extern crate coap;

use async_trait::async_trait;
use coap::{
    edhoc::{
        router::{edhoc_route, EdhocRouterState},
        EdhocIdentity, EdhocResponderStore, EdhocSessionHook, OscoreSecrets, EDHOC_WELL_KNOWN_PATH,
    },
    router::{get, Router},
    Server,
};
use data_encoding_macro::hexlower_permissive_array;
use lakers::Credential;
use std::{net::SocketAddr, sync::Arc};

hexlower_permissive_array!("const CRED_I" = "A2027734322D35302D33312D46462D45462D33372D33322D333908A101A5010202412B2001215820AC75E9ECE3E50BFC8ED60399889522405C47BF16DF96660A41298CB4307F7EB62258206E5DE611388A4B8A8211334AC7D37ECB52A387D257E6DB3C2A93DF21FF3AFFC8");
hexlower_permissive_array!("const CRED_R" = "A2026008A101A5010202410A2001215820BBC34960526EA4D32E940CAD2A234148DDC21791A12AFBCBAC93622046DD44F02258204519E257236B2A0CE2023F0931F1F386CA7AFDA64FCDE0108C224C51EABF6072");
hexlower_permissive_array!(
    "const R" = "72cc4761dbd4c78f758931aa589d348d1ef874a7e303ede2f140dcf3e6aa4aac"
);

struct PrintSecrets;

#[async_trait]
impl EdhocSessionHook for PrintSecrets {
    async fn on_established(&self, peer: SocketAddr, secrets: OscoreSecrets) {
        println!("EDHOC session with {peer} established: {secrets:?}");
        println!("  master_secret = {:02x?}", secrets.master_secret);
        println!("  master_salt   = {:02x?}", secrets.master_salt);
        println!("  sender_id     = {:02x?}", secrets.sender_id);
        println!("  recipient_id  = {:02x?}", secrets.recipient_id);
        // This is where you would install `secrets` as an OSCORE Security Context for `peer`.
    }
}

async fn hello() -> &'static str {
    "hello edhoc"
}

#[tokio::main]
async fn main() {
    let addr = "127.0.0.1:5683";

    let identity = EdhocIdentity {
        private_key: R,
        credential: Credential::parse_ccs(&CRED_R).unwrap(),
    };
    let trusted_initiator = Credential::parse_ccs(&CRED_I).unwrap();

    let state = EdhocRouterState {
        store: Arc::new(EdhocResponderStore::new(identity)),
        credentials: Arc::new(trusted_initiator),
        hook: Some(Arc::new(PrintSecrets)),
    };

    let router = Router::from_state(state)
        .route(EDHOC_WELL_KNOWN_PATH, edhoc_route())
        .route("/hello", get(hello));

    let server = Server::new_udp(addr).unwrap();
    println!("EDHOC server up on {addr}");
    server.serve(router).await.unwrap();
}
