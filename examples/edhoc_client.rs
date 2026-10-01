/// This example performs an EDHOC handshake, in the Initiator role, against the `edhoc_server`
/// example, prints the derived OSCORE key material, and then makes a normal echo request to
/// make sure the connection is still a perfectly ordinary CoAP one.
///
/// The credentials and private keys below are the RFC 9529 test vectors, used here only so the
/// example is self-contained; a real deployment must use its own keys.
extern crate coap;

use coap::{
    edhoc::{edhoc_initiate, EdhocIdentity, EDHOC_WELL_KNOWN_PATH},
    request::RequestBuilder,
    UdpCoAPClient,
};
use coap_lite::RequestType as Method;
use data_encoding_macro::hexlower_permissive_array;
use lakers::Credential;

hexlower_permissive_array!("const CRED_I" = "A2027734322D35302D33312D46462D45462D33372D33322D333908A101A5010202412B2001215820AC75E9ECE3E50BFC8ED60399889522405C47BF16DF96660A41298CB4307F7EB62258206E5DE611388A4B8A8211334AC7D37ECB52A387D257E6DB3C2A93DF21FF3AFFC8");
hexlower_permissive_array!(
    "const I" = "fb13adeb6518cee5f88417660841142e830a81fe334380a953406a1305e8706b"
);
hexlower_permissive_array!("const CRED_R" = "A2026008A101A5010202410A2001215820BBC34960526EA4D32E940CAD2A234148DDC21791A12AFBCBAC93622046DD44F02258204519E257236B2A0CE2023F0931F1F386CA7AFDA64FCDE0108C224C51EABF6072");

#[tokio::main]
async fn main() {
    let identity = EdhocIdentity {
        private_key: I,
        credential: Credential::parse_ccs(&CRED_I).unwrap(),
    };
    let trusted_responder = Credential::parse_ccs(&CRED_R).unwrap();

    let client = UdpCoAPClient::new("127.0.0.1:5683").await.unwrap();

    println!("Performing EDHOC handshake against 127.0.0.1:5683...");
    let secrets = edhoc_initiate(
        &client,
        EDHOC_WELL_KNOWN_PATH,
        &identity,
        Some(trusted_responder),
    )
    .await
    .expect("EDHOC handshake failed");

    println!("EDHOC handshake successful: {secrets:?}");
    println!("  master_secret = {:02x?}", secrets.master_secret);
    println!("  master_salt   = {:02x?}", secrets.master_salt);
    println!("  sender_id     = {:02x?}", secrets.sender_id);
    println!("  recipient_id  = {:02x?}", secrets.recipient_id);

    let request = RequestBuilder::new("/hello", Method::Get).build();
    let response = client.send(request).await.unwrap();
    println!(
        "Echo reply: {}",
        String::from_utf8_lossy(&response.message.payload)
    );
}
