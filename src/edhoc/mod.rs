//! Support for the EDHOC key exchange ([RFC 9528](https://tools.ietf.org/html/rfc9528)),
//! built on top of the [lakers](https://github.com/openwsn-berkeley/lakers) crate and its
//! pure-Rust `lakers-crypto-rustcrypto` backend.
//!
//! Only authentication mode STAT-STAT with cipher suite 2 is supported, matching what `lakers`
//! itself currently implements, and credentials are always transferred by reference, so every
//! identity used here needs a `kid` (see `Credential::with_kid`).
//!
//! Messages are transported over CoAP as described in RFC 9528 Appendix A.2: `message_1` is
//! POSTed to `EDHOC_WELL_KNOWN_PATH` prefixed with a CBOR `true` (`0xf5`) so the server can tell
//! it apart from `message_3`, whose POST is prefixed with the responder's connection identifier
//! `C_R` instead. A completed handshake, on either side, yields `OscoreSecrets`: the master
//! secret, master salt, and Sender/Recipient IDs needed to derive an OSCORE Security Context
//! (RFC 9528 Appendix A.1). This module does not implement OSCORE itself; the server-side
//! `EdhocSessionHook` is the integration point for handing that off.

use async_trait::async_trait;
use coap_lite::{option_value::OptionValueU16, CoapOption, RequestType as Method, ResponseType};
use lakers::{
    credential_check_or_fetch, generate_connection_identifier_cbor, BufferMessage1, BufferMessage2,
    BufferMessage3, BufferMessage4, BytesP256ElemLen, CBORDecoder, ConnId, Credential,
    CredentialTransfer, EDHOCError, EDHOCMethod, EDHOCSuite, EdhocInitiator, EdhocMessageBuffer,
    EdhocResponder, EdhocResponderWaitM3, IdCred,
};
use rand_core::RngCore;
use std::{
    collections::HashMap,
    fmt, io,
    net::SocketAddr,
    sync::Mutex,
    time::{Duration, Instant},
};

use crate::{
    client::{ClientTransport, CoAPClient},
    request::RequestBuilder,
};

#[cfg(feature = "router")]
pub mod router;

/// The well-known resource path EDHOC is transported over, per RFC 9528 Appendix A.2.
pub const EDHOC_WELL_KNOWN_PATH: &str = ".well-known/edhoc";

/// The CoAP Content-Format of `application/edhoc+cbor-seq` (RFC 9528 Section 10.9), used for
/// `message_2`, `message_4` and EDHOC error messages sent in responses.
pub const CONTENT_FORMAT_EDHOC_CBOR_SEQ: u16 = 64;

/// The CoAP Content-Format of `application/cid-edhoc+cbor-seq` (RFC 9528 Section 10.9), used for
/// the `message_1` and `message_3` requests.
pub const CONTENT_FORMAT_CID_EDHOC_CBOR_SEQ: u16 = 65;

fn content_format_option(content_format: u16) -> (CoapOption, Vec<u8>) {
    (
        CoapOption::ContentFormat,
        OptionValueU16(content_format).into(),
    )
}

/// How long a completed Responder session's `message_4` is kept around to answer retransmitted
/// `message_3` requests: CoAP's `EXCHANGE_LIFETIME` (RFC 7252 Section 4.8.2), after which a
/// Confirmable request can no longer be retransmitted.
const COMPLETED_SESSION_LIFETIME: Duration = Duration::from_secs(247);

/// The crypto backend used for EDHOC operations: the pure-Rust `lakers-crypto-rustcrypto`
/// backend, seeded from the operating system's CSPRNG.
pub type EdhocCrypto = lakers_crypto_rustcrypto::Crypto<rand_core::OsRng>;

fn new_crypto() -> EdhocCrypto {
    lakers_crypto_rustcrypto::Crypto::new(rand_core::OsRng)
}

/// An EDHOC identity: a static Diffie-Hellman private key and the credential that vouches for
/// its public counterpart.
///
/// The credential's `kid` must be set, since this module always transfers credentials by
/// reference (see `Credential::with_kid`).
#[derive(Clone, Copy)]
pub struct EdhocIdentity {
    /// The static private key matching the public key embedded in `credential`.
    pub private_key: BytesP256ElemLen,
    /// This side's own credential, as presented to the peer.
    pub credential: Credential,
}

/// Key material derived from a completed EDHOC handshake, suitable for establishing an OSCORE
/// Security Context as described in RFC 9528 Appendix A.1.
#[derive(Clone)]
pub struct OscoreSecrets {
    /// The OSCORE Master Secret (16 bytes, for the default AES-CCM-16-64-128 algorithm).
    pub master_secret: Vec<u8>,
    /// The OSCORE Master Salt (8 bytes, for the default AES-CCM-16-64-128 algorithm).
    pub master_salt: Vec<u8>,
    /// This side's OSCORE Sender ID.
    pub sender_id: Vec<u8>,
    /// This side's OSCORE Recipient ID.
    pub recipient_id: Vec<u8>,
    /// The EDHOC `PRK_out`, from which further application-specific keys can be exported via
    /// `edhoc_exporter`/`edhoc_key_update` for sessions that outlive the OSCORE context.
    pub prk_out: [u8; 32],
}

// Secret key material is deliberately not printed.
impl fmt::Debug for OscoreSecrets {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("OscoreSecrets")
            .field("master_secret", &"<redacted>")
            .field("master_salt", &"<redacted>")
            .field("sender_id", &self.sender_id)
            .field("recipient_id", &self.recipient_id)
            .field("prk_out", &"<redacted>")
            .finish()
    }
}

/// Errors that can occur while performing or serving an EDHOC handshake.
#[derive(Debug)]
pub enum EdhocError {
    /// The EDHOC protocol itself rejected the exchange (bad MAC, unsupported suite, ...).
    Protocol(EDHOCError),
    /// A transport-level I/O error occurred.
    Io(io::Error),
    /// The peer replied with a CoAP status other than 2.04 Changed.
    UnexpectedResponse(ResponseType),
    /// A message could not be parsed.
    Malformed(&'static str),
    /// `message_3` referred to a connection identifier with no matching pending session (it may
    /// have expired, already been used, or never existed).
    UnknownSession,
    /// The peer's credential could not be verified against the configured trust store.
    UntrustedCredential,
    /// Every connection identifier is already taken by a pending session, so no new session can
    /// be started until some complete.
    NoConnectionIdentifierAvailable,
    /// The peer replied with an EDHOC error message (RFC 9528 Section 6).
    Peer(EdhocErrorMessage),
}

impl fmt::Display for EdhocError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EdhocError::Protocol(e) => write!(f, "EDHOC error: {e:?}"),
            EdhocError::Io(e) => write!(f, "{e}"),
            EdhocError::UnexpectedResponse(status) => write!(f, "Unexpected response {status:?}"),
            EdhocError::Malformed(what) => write!(f, "Malformed `{what}`"),
            EdhocError::UnknownSession => {
                write!(f, "No pending session for this connection identifier")
            }
            EdhocError::UntrustedCredential => write!(f, "Untrusted peer credential"),
            EdhocError::NoConnectionIdentifierAvailable => {
                write!(f, "No connection identifier available")
            }
            EdhocError::Peer(err) => match err.diagnostic() {
                Some(diagnostic) => write!(f, "Peer reported EDHOC error: {diagnostic}"),
                None => write!(f, "Peer reported EDHOC error code {}", err.err_code),
            },
        }
    }
}

impl std::error::Error for EdhocError {}

impl From<EDHOCError> for EdhocError {
    fn from(err: EDHOCError) -> Self {
        EdhocError::Protocol(err)
    }
}

impl From<io::Error> for EdhocError {
    fn from(err: io::Error) -> Self {
        EdhocError::Io(err)
    }
}

/// An EDHOC error message (RFC 9528 Section 6) received from the peer.
#[derive(Debug, Clone, PartialEq)]
pub struct EdhocErrorMessage {
    /// The CoAP status the error message was sent with (4.00 or 5.00, per RFC 9528
    /// Appendix A.2.3).
    pub status: ResponseType,
    /// The `ERR_CODE`, e.g. 1 (Unspecified Error), 2 (Wrong Selected Cipher Suite) or 3 (Unknown
    /// Credential Referenced).
    pub err_code: i8,
    /// The CBOR-encoded `ERR_INFO` accompanying `err_code`.
    pub err_info: Vec<u8>,
}

impl EdhocErrorMessage {
    /// `ERR_CODE` for an unspecified error; `ERR_INFO` is a diagnostic text string.
    pub const UNSPECIFIED_ERROR: i8 = 1;
    /// `ERR_CODE` for a wrong selected cipher suite; `ERR_INFO` is `SUITES_R`.
    pub const WRONG_SELECTED_CIPHER_SUITE: i8 = 2;
    /// `ERR_CODE` for an unknown credential referenced; `ERR_INFO` is CBOR `true`.
    pub const UNKNOWN_CREDENTIAL_REFERENCED: i8 = 3;

    /// Decodes an EDHOC error message (a CBOR Sequence of `ERR_CODE` and `ERR_INFO`) from a
    /// response `payload` received with `status`.
    pub fn decode(status: ResponseType, payload: &[u8]) -> Option<Self> {
        let mut decoder = CBORDecoder::new(payload);
        let err_code = decoder.i8().ok()?;
        let err_info = decoder.any_as_encoded().ok()?.to_vec();
        decoder.ensure_finished().ok()?;
        Some(Self {
            status,
            err_code,
            err_info,
        })
    }

    /// The diagnostic message of an Unspecified Error, if this is one.
    pub fn diagnostic(&self) -> Option<&str> {
        if self.err_code != Self::UNSPECIFIED_ERROR {
            return None;
        }
        let mut decoder = CBORDecoder::new(&self.err_info);
        std::str::from_utf8(decoder.str().ok()?).ok()
    }
}

impl EdhocError {
    /// Encodes this error as an EDHOC error message (RFC 9528 Section 6), returning the CoAP
    /// status to send it with (4.00 or 5.00, per RFC 9528 Appendix A.2.3) and the payload.
    pub fn to_error_message(&self) -> (ResponseType, Vec<u8>) {
        match self {
            // SUITES_R: the only cipher suite supported, 2, encoded as a single CBOR int.
            EdhocError::Protocol(EDHOCError::UnsupportedCipherSuite) => (
                ResponseType::BadRequest,
                vec![EdhocErrorMessage::WRONG_SELECTED_CIPHER_SUITE as u8, 0x02],
            ),
            EdhocError::UntrustedCredential => (
                ResponseType::BadRequest,
                vec![EdhocErrorMessage::UNKNOWN_CREDENTIAL_REFERENCED as u8, 0xf5],
            ),
            EdhocError::Protocol(_) | EdhocError::Malformed(_) | EdhocError::UnknownSession => (
                ResponseType::BadRequest,
                unspecified_error(&self.to_string()),
            ),
            EdhocError::Io(_)
            | EdhocError::UnexpectedResponse(_)
            | EdhocError::NoConnectionIdentifierAvailable
            | EdhocError::Peer(_) => (
                ResponseType::InternalServerError,
                unspecified_error(&self.to_string()),
            ),
        }
    }
}

/// Encodes an Unspecified Error EDHOC error message: `ERR_CODE` 1 followed by `diagnostic` as a
/// CBOR text string, truncated to 255 bytes (the longest text string lakers-based peers decode).
fn unspecified_error(diagnostic: &str) -> Vec<u8> {
    let mut len = diagnostic.len().min(u8::MAX as usize);
    while !diagnostic.is_char_boundary(len) {
        len -= 1;
    }
    let mut payload = vec![EdhocErrorMessage::UNSPECIFIED_ERROR as u8];
    if len < 24 {
        payload.push(0x60 | len as u8);
    } else {
        payload.extend_from_slice(&[0x78, len as u8]);
    }
    payload.extend_from_slice(&diagnostic.as_bytes()[..len]);
    payload
}

fn expect_changed(response: &coap_lite::CoapResponse) -> Result<(), EdhocError> {
    let status = *response.get_status();
    if status == ResponseType::Changed {
        return Ok(());
    }
    let is_edhoc = response
        .message
        .get_first_option_as::<OptionValueU16>(CoapOption::ContentFormat)
        .and_then(Result::ok)
        .is_some_and(|cf| cf.0 == CONTENT_FORMAT_EDHOC_CBOR_SEQ);
    match EdhocErrorMessage::decode(status, &response.message.payload) {
        Some(err) if is_edhoc => Err(EdhocError::Peer(err)),
        _ => Err(EdhocError::UnexpectedResponse(status)),
    }
}

/// Performs an EDHOC handshake in the Initiator role against `path` on the peer reachable
/// through `client`, and returns the key material needed to establish an OSCORE Security
/// Context.
///
/// `own` is this side's static identity. `peer_credential`, if given, is the credential expected
/// from the Responder; when `None`, whatever credential the Responder presents by value is
/// trusted on first use (only appropriate outside of adversarial networks).
pub async fn edhoc_initiate<T: ClientTransport + 'static>(
    client: &CoAPClient<T>,
    path: &str,
    own: &EdhocIdentity,
    peer_credential: Option<Credential>,
) -> Result<OscoreSecrets, EdhocError> {
    let initiator = EdhocInitiator::new(
        new_crypto(),
        EDHOCMethod::StatStat,
        EDHOCSuite::CipherSuite2,
    );
    let c_i = generate_connection_identifier_cbor(&mut new_crypto());

    let (initiator, message_1) = initiator.prepare_message_1(Some(c_i), &None)?;
    let mut payload = Vec::with_capacity(1 + message_1.len);
    payload.push(0xf5); // CBOR `true`: marks this POST as carrying message_1
    payload.extend_from_slice(message_1.as_slice());

    let response = client
        .send(
            RequestBuilder::new(path, Method::Post)
                .options(vec![content_format_option(
                    CONTENT_FORMAT_CID_EDHOC_CBOR_SEQ,
                )])
                .data(Some(payload))
                .build(),
        )
        .await?;
    expect_changed(&response)?;

    let message_2: BufferMessage2 = EdhocMessageBuffer::new_from_slice(&response.message.payload)
        .map_err(|_| EdhocError::Malformed("message_2"))?;
    let (mut initiator, c_r, id_cred_r, _ead_2) = initiator.parse_message_2(&message_2)?;
    let valid_cred_r = credential_check_or_fetch(peer_credential, id_cred_r)?;
    initiator.set_identity(own.private_key, own.credential)?;
    let initiator = initiator.verify_message_2(valid_cred_r)?;

    let (initiator, message_3, prk_out) =
        initiator.prepare_message_3(CredentialTransfer::ByReference, &None)?;
    let mut payload = Vec::from(c_r.as_cbor());
    payload.extend_from_slice(message_3.as_slice());

    let response = client
        .send(
            RequestBuilder::new(path, Method::Post)
                .options(vec![content_format_option(
                    CONTENT_FORMAT_CID_EDHOC_CBOR_SEQ,
                )])
                .data(Some(payload))
                .build(),
        )
        .await?;
    expect_changed(&response)?;

    let message_4: BufferMessage4 = EdhocMessageBuffer::new_from_slice(&response.message.payload)
        .map_err(|_| EdhocError::Malformed("message_4"))?;
    let (mut initiator, _ead_4) = initiator.process_message_4(&message_4)?;

    let master_secret = initiator.edhoc_exporter(0, &[], 16)[..16].to_vec();
    let master_salt = initiator.edhoc_exporter(1, &[], 8)[..8].to_vec();

    // RFC 9528 Appendix A.1: the Initiator's OSCORE Sender ID is the Responder's connection
    // identifier C_R, and its Recipient ID is its own connection identifier C_I.
    Ok(OscoreSecrets {
        master_secret,
        master_salt,
        sender_id: c_r.as_slice().to_vec(),
        recipient_id: c_i.as_slice().to_vec(),
        prk_out,
    })
}

/// A source of trusted peer credentials, used by an EDHOC Responder to verify an Initiator's
/// credential.
///
/// Implementations typically look the credential up by the key identifier carried in `id_cred`;
/// see `Credential::by_kid` for how such identifiers are formed.
pub trait EdhocCredentialStore: Send + Sync {
    /// Returns the trusted credential `id_cred` refers to, if any.
    fn lookup(&self, id_cred: &IdCred) -> Option<Credential>;
}

/// A trust store that only ever trusts one, pre-configured peer credential. This covers the
/// common case of a device that only ever talks EDHOC to a single, known peer.
impl EdhocCredentialStore for Credential {
    fn lookup(&self, _id_cred: &IdCred) -> Option<Credential> {
        Some(*self)
    }
}

impl EdhocCredentialStore for HashMap<Vec<u8>, Credential> {
    fn lookup(&self, id_cred: &IdCred) -> Option<Credential> {
        self.get(id_cred.as_full_value()).copied()
    }
}

#[async_trait]
/// Notified once an EDHOC Responder session completes successfully, with the key material
/// needed to establish an OSCORE Security Context (RFC 9528 Appendix A.1).
///
/// This is the integration point for wiring EDHOC into an OSCORE implementation: implement this
/// trait to install `secrets` as the OSCORE context to use for `peer`.
pub trait EdhocSessionHook: Send + Sync {
    async fn on_established(&self, peer: SocketAddr, secrets: OscoreSecrets);
}

#[async_trait]
impl<F, Fut> EdhocSessionHook for F
where
    F: Fn(SocketAddr, OscoreSecrets) -> Fut + Send + Sync,
    Fut: std::future::Future<Output = ()> + Send,
{
    async fn on_established(&self, peer: SocketAddr, secrets: OscoreSecrets) {
        self(peer, secrets).await
    }
}

/// Server-side storage for EDHOC Responder sessions that are waiting for `message_3`, keyed by
/// the connection identifier `C_R` this server chose while replying to `message_1`.
///
/// Connection identifiers are compact one-byte integers (-24 to 23), so at most 48 sessions can
/// be pending at once; further `message_1`s are rejected until some complete. Entries for
/// exchanges that never complete (e.g. because the Initiator disappears after `message_1`) are
/// never removed automatically, so a peer that keeps starting handshakes without finishing them
/// can exhaust them. Deployments exposed to untrusted networks should take this into account.
///
/// Completed sessions are remembered for CoAP's `EXCHANGE_LIFETIME`, so that a retransmitted
/// `message_3` (e.g. because the response carrying `message_4` got lost) is answered with the
/// same `message_4` again instead of failing.
pub struct EdhocResponderStore {
    own: EdhocIdentity,
    pending: Mutex<HashMap<Vec<u8>, PendingSession>>,
    /// `message_4`s sent for recently completed sessions, keyed by the `message_3` request
    /// payload they answered.
    completed: Mutex<HashMap<Vec<u8>, CompletedSession>>,
}

/// A Responder session that has already answered `message_3`.
struct CompletedSession {
    message_4: Vec<u8>,
    expires: Instant,
}

/// A Responder session that has sent `message_2` and is waiting for `message_3`, along with the
/// Initiator's connection identifier `C_I` from `message_1`.
type PendingSession = (ConnId, EdhocResponderWaitM3<EdhocCrypto>);

impl EdhocResponderStore {
    /// Creates a new, empty store that serves EDHOC as `own`.
    pub fn new(own: EdhocIdentity) -> Self {
        Self {
            own,
            pending: Mutex::new(HashMap::new()),
            completed: Mutex::new(HashMap::new()),
        }
    }

    /// The number of sessions currently awaiting `message_3`.
    pub fn pending_sessions(&self) -> usize {
        self.pending.lock().unwrap().len()
    }
}

/// Processes one leg of an EDHOC exchange transported per RFC 9528 Appendix A.2: either a
/// `message_1` POST (prefixed with CBOR `true`) or a `message_3` POST (prefixed with `C_R`).
///
/// On success, returns the raw payload (`message_2` or `message_4`, respectively) to send back
/// with a 2.04 Changed response. `credentials` is consulted to authenticate the Initiator's
/// credential while processing `message_3`; `hook`, if given, is notified with the resulting
/// OSCORE key material once the handshake completes.
pub async fn process_edhoc_message(
    store: &EdhocResponderStore,
    credentials: &(impl EdhocCredentialStore + ?Sized),
    hook: Option<&dyn EdhocSessionHook>,
    peer: SocketAddr,
    payload: &[u8],
) -> Result<Vec<u8>, EdhocError> {
    match payload.first() {
        Some(0xf5) => process_message_1(store, &payload[1..]),
        _ => process_message_3(store, credentials, hook, peer, payload).await,
    }
}

fn process_message_1(store: &EdhocResponderStore, payload: &[u8]) -> Result<Vec<u8>, EdhocError> {
    let message_1: BufferMessage1 = EdhocMessageBuffer::new_from_slice(payload)
        .map_err(|_| EdhocError::Malformed("message_1"))?;

    let responder = EdhocResponder::new(
        new_crypto(),
        EDHOCMethod::StatStat,
        store.own.private_key,
        store.own.credential,
    );
    let (responder, c_i, _ead_1) = responder.process_message_1(&message_1)?;

    // Choosing C_R and registering the session happen under one lock, so concurrent exchanges
    // can never end up with the same C_R.
    let mut pending = store.pending.lock().unwrap();
    let c_r = allocate_connection_identifier(&pending, &c_i)?;
    let (responder, message_2) =
        responder.prepare_message_2(CredentialTransfer::ByReference, Some(c_r), &None)?;
    pending.insert(c_r.as_slice().to_vec(), (c_i, responder));

    Ok(message_2.as_slice().to_vec())
}

/// Picks a random connection identifier `C_R` that is neither used by a pending session nor
/// equal to the Initiator's `C_I` (which RFC 9528 Section 3.3.2 forbids).
fn allocate_connection_identifier(
    pending: &HashMap<Vec<u8>, PendingSession>,
    c_i: &ConnId,
) -> Result<ConnId, EdhocError> {
    // Every connection identifier encodable as a single CBOR byte: 0 to 23, then -1 to -24.
    let free: Vec<ConnId> = (0x00..=0x17)
        .chain(0x20..=0x37)
        .map(|raw: u8| {
            ConnId::from_decoder(&mut CBORDecoder::new(&[raw]))
                .expect("single-byte CBOR integers are valid connection identifiers")
        })
        .filter(|c_r| c_r.as_slice() != c_i.as_slice() && !pending.contains_key(c_r.as_slice()))
        .collect();
    if free.is_empty() {
        return Err(EdhocError::NoConnectionIdentifierAvailable);
    }
    let index = rand_core::OsRng.next_u32() as usize % free.len();
    Ok(free[index])
}

async fn process_message_3(
    store: &EdhocResponderStore,
    credentials: &(impl EdhocCredentialStore + ?Sized),
    hook: Option<&dyn EdhocSessionHook>,
    peer: SocketAddr,
    payload: &[u8],
) -> Result<Vec<u8>, EdhocError> {
    {
        let mut completed = store.completed.lock().unwrap();
        let now = Instant::now();
        completed.retain(|_, session| session.expires > now);
        if let Some(session) = completed.get(payload) {
            // A retransmission of a message_3 already processed: replay its message_4.
            return Ok(session.message_4.clone());
        }
    }

    let mut decoder = CBORDecoder::new(payload);
    let c_r =
        ConnId::from_decoder(&mut decoder).map_err(|_| EdhocError::Malformed("C_R prefix"))?;
    let rest = decoder
        .remaining_buffer()
        .map_err(|_| EdhocError::Malformed("message_3"))?;
    let message_3: BufferMessage3 =
        EdhocMessageBuffer::new_from_slice(rest).map_err(|_| EdhocError::Malformed("message_3"))?;

    let (c_i, responder) = store
        .pending
        .lock()
        .unwrap()
        .remove(c_r.as_slice())
        .ok_or(EdhocError::UnknownSession)?;

    let (responder, id_cred_i, _ead_3) = responder.parse_message_3(&message_3)?;
    let expected = credentials.lookup(&id_cred_i);
    let valid_cred_i = credential_check_or_fetch(expected, id_cred_i)
        .map_err(|_| EdhocError::UntrustedCredential)?;
    let (responder, prk_out) = responder.verify_message_3(valid_cred_i)?;
    let (mut responder, message_4) = responder.prepare_message_4(&None)?;
    let message_4 = message_4.as_slice().to_vec();
    store.completed.lock().unwrap().insert(
        payload.to_vec(),
        CompletedSession {
            message_4: message_4.clone(),
            expires: Instant::now() + COMPLETED_SESSION_LIFETIME,
        },
    );

    let master_secret = responder.edhoc_exporter(0, &[], 16)[..16].to_vec();
    let master_salt = responder.edhoc_exporter(1, &[], 8)[..8].to_vec();

    // RFC 9528 Appendix A.1: the Responder's OSCORE Sender ID is the Initiator's connection
    // identifier C_I, and its Recipient ID is its own connection identifier C_R.
    let secrets = OscoreSecrets {
        master_secret,
        master_salt,
        sender_id: c_i.as_slice().to_vec(),
        recipient_id: c_r.as_slice().to_vec(),
        prk_out,
    };
    if let Some(hook) = hook {
        hook.on_established(peer, secrets).await;
    }

    Ok(message_4)
}

#[cfg(all(test, feature = "router"))]
mod tests {
    use super::*;
    use crate::{
        client::UdpCoAPClient,
        edhoc::router::{edhoc_route, EdhocRouterState},
        router::{extract::FromRef, get, Router},
        server::UdpCoapListener,
        Server,
    };
    use data_encoding_macro::hexlower_permissive_array;
    use std::sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    };

    // RFC 9529 test vectors, also used by lakers's own test suite.
    hexlower_permissive_array!("const CRED_I" = "A2027734322D35302D33312D46462D45462D33372D33322D333908A101A5010202412B2001215820AC75E9ECE3E50BFC8ED60399889522405C47BF16DF96660A41298CB4307F7EB62258206E5DE611388A4B8A8211334AC7D37ECB52A387D257E6DB3C2A93DF21FF3AFFC8");
    hexlower_permissive_array!(
        "const I" = "fb13adeb6518cee5f88417660841142e830a81fe334380a953406a1305e8706b"
    );
    hexlower_permissive_array!("const CRED_R" = "A2026008A101A5010202410A2001215820BBC34960526EA4D32E940CAD2A234148DDC21791A12AFBCBAC93622046DD44F02258204519E257236B2A0CE2023F0931F1F386CA7AFDA64FCDE0108C224C51EABF6072");
    hexlower_permissive_array!(
        "const R" = "72cc4761dbd4c78f758931aa589d348d1ef874a7e303ede2f140dcf3e6aa4aac"
    );
    // A message_1 selecting a cipher suite this implementation does not support, from
    // draft-ietf-lake-traces (also used, verbatim, by lakers's own test suite).
    hexlower_permissive_array!(
        "const MESSAGE_1_UNSUPPORTED_SUITE" =
            "03065820741a13d7ba048fbb615e94386aa3b61bea5b3d8f65f32620b749bee8d278efa90e"
    );

    async fn spawn_edhoc_server(state: EdhocRouterState) -> u16 {
        spawn_router(Router::from_state(state).route(EDHOC_WELL_KNOWN_PATH, edhoc_route())).await
    }

    async fn spawn_router<S: Clone + Send + Sync + 'static>(router: Router<S>) -> u16 {
        let sock = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let port = sock.local_addr().unwrap().port();
        let listener = Box::new(UdpCoapListener::from_socket(sock));
        let server = Server::from_listeners(vec![listener]);
        tokio::spawn(async move {
            server.serve(router).await.unwrap();
        });
        port
    }

    struct CountingHook(Arc<AtomicUsize>);

    #[async_trait]
    impl EdhocSessionHook for CountingHook {
        async fn on_established(&self, _peer: SocketAddr, secrets: OscoreSecrets) {
            assert_eq!(secrets.master_secret.len(), 16);
            assert_eq!(secrets.master_salt.len(), 8);
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[tokio::test]
    async fn test_edhoc_handshake_over_coap() {
        let cred_i = Credential::parse_ccs(&CRED_I).unwrap();
        let cred_r = Credential::parse_ccs(&CRED_R).unwrap();

        let hook_calls = Arc::new(AtomicUsize::new(0));
        let state = EdhocRouterState {
            store: Arc::new(EdhocResponderStore::new(EdhocIdentity {
                private_key: R,
                credential: cred_r,
            })),
            credentials: Arc::new(cred_i),
            hook: Some(Arc::new(CountingHook(hook_calls.clone()))),
        };
        let port = spawn_edhoc_server(state).await;

        let client = UdpCoAPClient::new(("127.0.0.1", port)).await.unwrap();
        let client_identity = EdhocIdentity {
            private_key: I,
            credential: cred_i,
        };

        let initiator_secrets = edhoc_initiate(
            &client,
            EDHOC_WELL_KNOWN_PATH,
            &client_identity,
            Some(cred_r),
        )
        .await
        .unwrap();

        assert_eq!(initiator_secrets.master_secret.len(), 16);
        assert_eq!(initiator_secrets.master_salt.len(), 8);
        // The Initiator's Sender ID must become the Responder's Recipient ID, and vice versa
        // (RFC 9528 Appendix A.1), so the two sides can talk OSCORE to each other.
        assert_ne!(initiator_secrets.sender_id, initiator_secrets.recipient_id);

        // The Debug impl must redact secret key material but still show the (non-secret)
        // connection identifiers.
        let debug = format!("{initiator_secrets:?}");
        assert!(debug.contains(r#"master_secret: "<redacted>""#));
        assert!(debug.contains(r#"master_salt: "<redacted>""#));
        assert!(debug.contains(r#"prk_out: "<redacted>""#));
        assert!(debug.contains(&format!("{:?}", initiator_secrets.sender_id)));
        assert!(debug.contains(&format!("{:?}", initiator_secrets.recipient_id)));

        // Give the responder's hook a moment to run; it fires synchronously within the request
        // handler, but the client already got its response by the time it does.
        for _ in 0..50 {
            if hook_calls.load(Ordering::SeqCst) == 1 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert_eq!(hook_calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_edhoc_untrusted_initiator_is_rejected() {
        let cred_i = Credential::parse_ccs(&CRED_I).unwrap();
        let cred_r = Credential::parse_ccs(&CRED_R).unwrap();

        let state = EdhocRouterState {
            store: Arc::new(EdhocResponderStore::new(EdhocIdentity {
                private_key: R,
                credential: cred_r,
            })),
            // The server only trusts cred_r itself, never cred_i, so message_3 must be rejected.
            credentials: Arc::new(cred_r),
            hook: None,
        };
        let port = spawn_edhoc_server(state).await;

        let client = UdpCoAPClient::new(("127.0.0.1", port)).await.unwrap();
        let client_identity = EdhocIdentity {
            private_key: I,
            credential: cred_i,
        };

        let result = edhoc_initiate(
            &client,
            EDHOC_WELL_KNOWN_PATH,
            &client_identity,
            Some(cred_r),
        )
        .await;
        match result {
            Err(EdhocError::Peer(err)) => {
                assert_eq!(err.status, ResponseType::BadRequest);
                assert_eq!(
                    err.err_code,
                    EdhocErrorMessage::UNKNOWN_CREDENTIAL_REFERENCED
                );
                assert_eq!(err.err_info, vec![0xf5]);
            }
            other => panic!("unexpected result: {other:?}"),
        }
    }

    /// Decodes a response as an EDHOC error message, checking its Content-Format.
    fn expect_error_message(response: coap_lite::CoapResponse) -> EdhocErrorMessage {
        assert_eq!(
            response
                .message
                .get_first_option_as::<OptionValueU16>(CoapOption::ContentFormat)
                .unwrap()
                .unwrap()
                .0,
            CONTENT_FORMAT_EDHOC_CBOR_SEQ
        );
        EdhocErrorMessage::decode(*response.get_status(), &response.message.payload).unwrap()
    }

    async fn edhoc_endpoint(port: u16) -> String {
        format!("coap://127.0.0.1:{port}/{EDHOC_WELL_KNOWN_PATH}")
    }

    #[tokio::test]
    async fn test_edhoc_message_1_rejects_unsupported_cipher_suite() {
        let cred_r = Credential::parse_ccs(&CRED_R).unwrap();
        let state = EdhocRouterState {
            store: Arc::new(EdhocResponderStore::new(EdhocIdentity {
                private_key: R,
                credential: cred_r,
            })),
            credentials: Arc::new(cred_r),
            hook: None,
        };
        let port = spawn_edhoc_server(state).await;

        let mut payload = vec![0xf5];
        payload.extend_from_slice(&MESSAGE_1_UNSUPPORTED_SUITE);

        let response = UdpCoAPClient::post(&edhoc_endpoint(port).await, payload)
            .await
            .unwrap();

        let err = expect_error_message(response);
        assert_eq!(err.status, ResponseType::BadRequest);
        assert_eq!(err.err_code, EdhocErrorMessage::WRONG_SELECTED_CIPHER_SUITE);
        // SUITES_R: cipher suite 2, the only one supported.
        assert_eq!(err.err_info, vec![0x02]);
    }

    #[tokio::test]
    async fn test_edhoc_message_1_rejects_oversized_payload() {
        let cred_r = Credential::parse_ccs(&CRED_R).unwrap();
        let state = EdhocRouterState {
            store: Arc::new(EdhocResponderStore::new(EdhocIdentity {
                private_key: R,
                credential: cred_r,
            })),
            credentials: Arc::new(cred_r),
            hook: None,
        };
        let port = spawn_edhoc_server(state).await;

        // lakers::MAX_MESSAGE_SIZE_LEN (192 by default) is exceeded, so this can never be a
        // valid message_1, regardless of its content.
        let mut payload = vec![0xf5];
        payload.resize(301, 0);

        let response = UdpCoAPClient::post(&edhoc_endpoint(port).await, payload)
            .await
            .unwrap();

        let err = expect_error_message(response);
        assert_eq!(err.status, ResponseType::BadRequest);
        assert_eq!(err.diagnostic(), Some("Malformed `message_1`"));
    }

    #[tokio::test]
    async fn test_edhoc_message_3_unknown_session_is_rejected() {
        let cred_r = Credential::parse_ccs(&CRED_R).unwrap();
        let state = EdhocRouterState {
            store: Arc::new(EdhocResponderStore::new(EdhocIdentity {
                private_key: R,
                credential: cred_r,
            })),
            credentials: Arc::new(cred_r),
            hook: None,
        };
        let port = spawn_edhoc_server(state).await;

        // A single-byte CBOR-encoded connection identifier that was never registered by a prior
        // message_1, followed by arbitrary bytes standing in for message_3.
        let payload = vec![0x05, 0x00, 0x00];

        let response = UdpCoAPClient::post(&edhoc_endpoint(port).await, payload)
            .await
            .unwrap();

        let err = expect_error_message(response);
        assert_eq!(err.status, ResponseType::BadRequest);
        assert_eq!(
            err.diagnostic(),
            Some("No pending session for this connection identifier")
        );
    }

    #[tokio::test]
    async fn test_edhoc_initiate_rejects_non_changed_response() {
        let cred_i = Credential::parse_ccs(&CRED_I).unwrap();
        let cred_r = Credential::parse_ccs(&CRED_R).unwrap();
        let state = EdhocRouterState {
            store: Arc::new(EdhocResponderStore::new(EdhocIdentity {
                private_key: R,
                credential: cred_r,
            })),
            credentials: Arc::new(cred_i),
            hook: None,
        };
        let port = spawn_edhoc_server(state).await;

        let client = UdpCoAPClient::new(("127.0.0.1", port)).await.unwrap();
        let client_identity = EdhocIdentity {
            private_key: I,
            credential: cred_i,
        };

        // Nothing is routed at this path, so the router's default fallback replies 4.04 Not
        // Found instead of 2.04 Changed.
        let result = edhoc_initiate(
            &client,
            "not-the-edhoc-path",
            &client_identity,
            Some(cred_r),
        )
        .await;
        assert!(matches!(
            result,
            Err(EdhocError::UnexpectedResponse(ResponseType::NotFound))
        ));
    }

    #[tokio::test]
    async fn test_edhoc_initiate_reports_io_error_on_timeout() {
        let cred_i = Credential::parse_ccs(&CRED_I).unwrap();
        let cred_r = Credential::parse_ccs(&CRED_R).unwrap();
        let client_identity = EdhocIdentity {
            private_key: I,
            credential: cred_i,
        };

        // Reserve a free port, then drop the socket so nothing is listening there.
        let dead_port = {
            let sock = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
            sock.local_addr().unwrap().port()
        };

        let mut client = UdpCoAPClient::new(("127.0.0.1", dead_port)).await.unwrap();
        client.set_receive_timeout(std::time::Duration::from_millis(100));
        client.set_transport_retries(1);

        let result = edhoc_initiate(
            &client,
            EDHOC_WELL_KNOWN_PATH,
            &client_identity,
            Some(cred_r),
        )
        .await;
        assert!(matches!(result, Err(EdhocError::Io(_))));
    }

    #[test]
    fn test_hashmap_credential_store_looks_up_by_kid() {
        let cred_i = Credential::parse_ccs(&CRED_I).unwrap();
        let cred_r = Credential::parse_ccs(&CRED_R).unwrap();

        let mut store: HashMap<Vec<u8>, Credential> = HashMap::new();
        store.insert(cred_i.by_kid().unwrap().as_full_value().to_vec(), cred_i);

        assert_eq!(store.lookup(&cred_i.by_kid().unwrap()), Some(cred_i));
        assert_eq!(store.lookup(&cred_r.by_kid().unwrap()), None);
    }

    #[tokio::test]
    async fn test_closure_session_hook_is_invoked() {
        let calls = Arc::new(AtomicUsize::new(0));
        let hook = {
            let calls = calls.clone();
            move |_peer: SocketAddr, _secrets: OscoreSecrets| {
                let calls = calls.clone();
                async move {
                    calls.fetch_add(1, Ordering::SeqCst);
                }
            }
        };

        // Placeholder key material: this test only checks that the closure is invoked with
        // whatever it is given, not any particular protocol content.
        let secrets = OscoreSecrets {
            master_secret: vec![0u8; 16],
            master_salt: vec![0u8; 8],
            sender_id: vec![0x01],
            recipient_id: vec![0x02],
            prk_out: [0u8; 32],
        };
        hook.on_established("127.0.0.1:0".parse().unwrap(), secrets)
            .await;

        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn test_pending_sessions_reports_pending_count() {
        let cred_r = Credential::parse_ccs(&CRED_R).unwrap();
        let store = EdhocResponderStore::new(EdhocIdentity {
            private_key: R,
            credential: cred_r,
        });
        assert_eq!(store.pending_sessions(), 0);

        // Drive a real message_1 through the Initiator side to get well-formed bytes without
        // going over the network.
        let initiator = EdhocInitiator::new(
            new_crypto(),
            EDHOCMethod::StatStat,
            EDHOCSuite::CipherSuite2,
        );
        let (_initiator, message_1) = initiator.prepare_message_1(None, &None).unwrap();

        let message_2 = process_message_1(&store, message_1.as_slice()).unwrap();
        assert_eq!(store.pending_sessions(), 1);
        assert!(!message_2.is_empty());
    }

    #[derive(Clone)]
    struct AppState {
        edhoc: EdhocRouterState,
        greeting: &'static str,
    }

    impl FromRef<AppState> for EdhocRouterState {
        fn from_ref(state: &AppState) -> Self {
            state.edhoc.clone()
        }
    }

    impl FromRef<AppState> for &'static str {
        fn from_ref(state: &AppState) -> Self {
            state.greeting
        }
    }

    #[tokio::test]
    async fn test_edhoc_route_with_composed_state() {
        let cred_i = Credential::parse_ccs(&CRED_I).unwrap();
        let cred_r = Credential::parse_ccs(&CRED_R).unwrap();
        let state = AppState {
            edhoc: EdhocRouterState {
                store: Arc::new(EdhocResponderStore::new(EdhocIdentity {
                    private_key: R,
                    credential: cred_r,
                })),
                credentials: Arc::new(cred_i),
                hook: None,
            },
            greeting: "hello",
        };
        async fn hello(
            crate::router::extract::State(greeting): crate::router::extract::State<&'static str>,
        ) -> &'static str {
            greeting
        }
        let port = spawn_router(
            Router::from_state(state)
                .route(EDHOC_WELL_KNOWN_PATH, edhoc_route())
                .route("hello", get(hello)),
        )
        .await;

        let client = UdpCoAPClient::new(("127.0.0.1", port)).await.unwrap();
        let client_identity = EdhocIdentity {
            private_key: I,
            credential: cred_i,
        };
        edhoc_initiate(
            &client,
            EDHOC_WELL_KNOWN_PATH,
            &client_identity,
            Some(cred_r),
        )
        .await
        .unwrap();

        let response = UdpCoAPClient::get(&format!("coap://127.0.0.1:{port}/hello"))
            .await
            .unwrap();
        assert_eq!(response.message.payload, b"hello");
    }

    #[tokio::test]
    async fn test_edhoc_responses_carry_content_format() {
        let cred_r = Credential::parse_ccs(&CRED_R).unwrap();
        let state = EdhocRouterState {
            store: Arc::new(EdhocResponderStore::new(EdhocIdentity {
                private_key: R,
                credential: cred_r,
            })),
            credentials: Arc::new(cred_r),
            hook: None,
        };
        let port = spawn_edhoc_server(state).await;

        let initiator = EdhocInitiator::new(
            new_crypto(),
            EDHOCMethod::StatStat,
            EDHOCSuite::CipherSuite2,
        );
        let (_initiator, message_1) = initiator.prepare_message_1(None, &None).unwrap();
        let mut payload = vec![0xf5];
        payload.extend_from_slice(message_1.as_slice());

        let response = UdpCoAPClient::post(&edhoc_endpoint(port).await, payload)
            .await
            .unwrap();
        assert_eq!(*response.get_status(), ResponseType::Changed);
        assert_eq!(
            response
                .message
                .get_first_option_as::<OptionValueU16>(CoapOption::ContentFormat)
                .unwrap()
                .unwrap()
                .0,
            CONTENT_FORMAT_EDHOC_CBOR_SEQ
        );
    }

    #[tokio::test]
    async fn test_retransmitted_message_3_replays_message_4() {
        let cred_i = Credential::parse_ccs(&CRED_I).unwrap();
        let cred_r = Credential::parse_ccs(&CRED_R).unwrap();
        let store = EdhocResponderStore::new(EdhocIdentity {
            private_key: R,
            credential: cred_r,
        });
        let hook_calls = Arc::new(AtomicUsize::new(0));
        let hook = CountingHook(hook_calls.clone());
        let peer: SocketAddr = "127.0.0.1:0".parse().unwrap();

        let initiator = EdhocInitiator::new(
            new_crypto(),
            EDHOCMethod::StatStat,
            EDHOCSuite::CipherSuite2,
        );
        let (initiator, message_1) = initiator.prepare_message_1(None, &None).unwrap();
        let mut payload = vec![0xf5];
        payload.extend_from_slice(message_1.as_slice());
        let message_2 = process_edhoc_message(&store, &cred_i, Some(&hook), peer, &payload)
            .await
            .unwrap();

        let message_2: BufferMessage2 = EdhocMessageBuffer::new_from_slice(&message_2).unwrap();
        let (mut initiator, c_r, id_cred_r, _ead_2) =
            initiator.parse_message_2(&message_2).unwrap();
        let valid_cred_r = credential_check_or_fetch(Some(cred_r), id_cred_r).unwrap();
        initiator.set_identity(I, cred_i).unwrap();
        let initiator = initiator.verify_message_2(valid_cred_r).unwrap();
        let (_initiator, message_3, _prk_out) = initiator
            .prepare_message_3(CredentialTransfer::ByReference, &None)
            .unwrap();
        let mut payload = Vec::from(c_r.as_cbor());
        payload.extend_from_slice(message_3.as_slice());

        let message_4 = process_edhoc_message(&store, &cred_i, Some(&hook), peer, &payload)
            .await
            .unwrap();
        let replayed = process_edhoc_message(&store, &cred_i, Some(&hook), peer, &payload)
            .await
            .unwrap();
        assert_eq!(message_4, replayed);
        // The session is only established once, however often message_3 arrives.
        assert_eq!(hook_calls.load(Ordering::SeqCst), 1);
        assert_eq!(store.pending_sessions(), 0);
    }

    #[test]
    fn test_connection_identifiers_never_collide() {
        let cred_r = Credential::parse_ccs(&CRED_R).unwrap();
        let store = EdhocResponderStore::new(EdhocIdentity {
            private_key: R,
            credential: cred_r,
        });

        // 48 identifiers exist, one of which may be ruled out as each message_1's own C_I.
        for sessions in 1.. {
            let initiator = EdhocInitiator::new(
                new_crypto(),
                EDHOCMethod::StatStat,
                EDHOCSuite::CipherSuite2,
            );
            let (_initiator, message_1) = initiator.prepare_message_1(None, &None).unwrap();
            match process_message_1(&store, message_1.as_slice()) {
                Ok(_) => assert_eq!(store.pending_sessions(), sessions),
                Err(EdhocError::NoConnectionIdentifierAvailable) => {
                    assert!(store.pending_sessions() >= 47);
                    break;
                }
                Err(err) => panic!("unexpected error: {err}"),
            }
            assert!(sessions <= 48);
        }
    }

    #[test]
    fn test_connection_identifier_differs_from_c_i() {
        let pending = HashMap::new();
        let c_i = ConnId::from_decoder(&mut CBORDecoder::new(&[0x05])).unwrap();
        for _ in 0..200 {
            let c_r = allocate_connection_identifier(&pending, &c_i).unwrap();
            assert_ne!(c_r.as_slice(), c_i.as_slice());
        }
    }

    #[test]
    fn test_error_message_round_trip() {
        let (status, payload) = EdhocError::UnknownSession.to_error_message();
        let err = EdhocErrorMessage::decode(status, &payload).unwrap();
        assert_eq!(err.status, ResponseType::BadRequest);
        assert_eq!(err.err_code, EdhocErrorMessage::UNSPECIFIED_ERROR);
        assert_eq!(
            err.diagnostic(),
            Some("No pending session for this connection identifier")
        );

        let (status, _) = EdhocError::NoConnectionIdentifierAvailable.to_error_message();
        assert_eq!(status, ResponseType::InternalServerError);

        // Long diagnostics are truncated to 255 bytes, on a character boundary.
        let long = "ä".repeat(200);
        let payload = unspecified_error(&long);
        let err = EdhocErrorMessage::decode(ResponseType::BadRequest, &payload).unwrap();
        assert_eq!(err.diagnostic(), Some("ä".repeat(127).as_str()));
    }

    #[test]
    fn test_error_display() {
        let peer = |err_code, err_info: &[u8]| {
            EdhocError::Peer(EdhocErrorMessage {
                status: ResponseType::BadRequest,
                err_code,
                err_info: err_info.to_vec(),
            })
        };
        let cases = [
            (
                EdhocError::Protocol(EDHOCError::MacVerificationFailed),
                "EDHOC error: MacVerificationFailed",
            ),
            (
                EdhocError::Io(io::Error::new(io::ErrorKind::TimedOut, "timed out")),
                "timed out",
            ),
            (
                EdhocError::UnexpectedResponse(ResponseType::NotFound),
                "Unexpected response NotFound",
            ),
            (EdhocError::UntrustedCredential, "Untrusted peer credential"),
            (
                EdhocError::NoConnectionIdentifierAvailable,
                "No connection identifier available",
            ),
            // Unspecified Error with a text diagnostic: "oops".
            (
                peer(EdhocErrorMessage::UNSPECIFIED_ERROR, b"\x64oops"),
                "Peer reported EDHOC error: oops",
            ),
            // Unknown Credential Referenced carries no diagnostic.
            (
                peer(EdhocErrorMessage::UNKNOWN_CREDENTIAL_REFERENCED, b"\xf5"),
                "Peer reported EDHOC error code 3",
            ),
        ];
        for (err, expected) in cases {
            assert_eq!(err.to_string(), expected);
        }
    }

    /// Collects the key material of every session the Responder establishes.
    struct CapturingHook(Arc<Mutex<Vec<OscoreSecrets>>>);

    #[async_trait]
    impl EdhocSessionHook for CapturingHook {
        async fn on_established(&self, _peer: SocketAddr, secrets: OscoreSecrets) {
            self.0.lock().unwrap().push(secrets);
        }
    }

    /// Everything a `spawn_proxy` proxy saw on the wire.
    #[derive(Default)]
    struct Wire {
        requests: Vec<coap_lite::Packet>,
        responses: Vec<coap_lite::Packet>,
    }

    /// Spawns a UDP proxy for a single client in front of the server on `server_port`, recording
    /// every packet and optionally dropping the first response to `message_3`.
    async fn spawn_proxy(server_port: u16, drop_first_message_4: bool) -> (u16, Arc<Mutex<Wire>>) {
        let downstream = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        let upstream = tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();
        upstream.connect(("127.0.0.1", server_port)).await.unwrap();
        let port = downstream.local_addr().unwrap().port();
        let wire = Arc::new(Mutex::new(Wire::default()));

        let recorded = wire.clone();
        tokio::spawn(async move {
            let mut client = None;
            let mut last_was_message_3 = false;
            let mut dropped = !drop_first_message_4;
            let (mut down_buf, mut up_buf) = ([0u8; 1500], [0u8; 1500]);
            loop {
                tokio::select! {
                    Ok((len, from)) = downstream.recv_from(&mut down_buf) => {
                        client = Some(from);
                        let packet = coap_lite::Packet::from_bytes(&down_buf[..len]).unwrap();
                        last_was_message_3 = packet.payload.first() != Some(&0xf5);
                        recorded.lock().unwrap().requests.push(packet);
                        upstream.send(&down_buf[..len]).await.unwrap();
                    }
                    Ok(len) = upstream.recv(&mut up_buf) => {
                        let packet = coap_lite::Packet::from_bytes(&up_buf[..len]).unwrap();
                        recorded.lock().unwrap().responses.push(packet);
                        if last_was_message_3 && !dropped {
                            dropped = true;
                            continue;
                        }
                        downstream.send_to(&up_buf[..len], client.unwrap()).await.unwrap();
                    }
                }
            }
        });
        (port, wire)
    }

    fn content_format(packet: &coap_lite::Packet) -> Option<u16> {
        packet
            .get_first_option_as::<OptionValueU16>(CoapOption::ContentFormat)
            .map(|cf| cf.unwrap().0)
    }

    fn responder_state(hook: Option<Arc<dyn EdhocSessionHook>>) -> EdhocRouterState {
        EdhocRouterState {
            store: Arc::new(EdhocResponderStore::new(EdhocIdentity {
                private_key: R,
                credential: Credential::parse_ccs(&CRED_R).unwrap(),
            })),
            credentials: Arc::new(Credential::parse_ccs(&CRED_I).unwrap()),
            hook,
        }
    }

    fn initiator_identity() -> EdhocIdentity {
        EdhocIdentity {
            private_key: I,
            credential: Credential::parse_ccs(&CRED_I).unwrap(),
        }
    }

    #[tokio::test]
    async fn test_edhoc_content_formats_on_the_wire() {
        let server_port = spawn_edhoc_server(responder_state(None)).await;
        let (proxy_port, wire) = spawn_proxy(server_port, false).await;

        let client = UdpCoAPClient::new(("127.0.0.1", proxy_port)).await.unwrap();
        let cred_r = Credential::parse_ccs(&CRED_R).unwrap();
        edhoc_initiate(
            &client,
            EDHOC_WELL_KNOWN_PATH,
            &initiator_identity(),
            Some(cred_r),
        )
        .await
        .unwrap();

        let wire = wire.lock().unwrap();
        assert_eq!(wire.requests.len(), 2);
        assert_eq!(wire.requests[0].payload[0], 0xf5);
        for request in &wire.requests {
            assert_eq!(
                content_format(request),
                Some(CONTENT_FORMAT_CID_EDHOC_CBOR_SEQ)
            );
        }
        assert_eq!(wire.responses.len(), 2);
        for response in &wire.responses {
            assert_eq!(
                response.header.code,
                coap_lite::MessageClass::Response(ResponseType::Changed)
            );
            assert_eq!(
                content_format(response),
                Some(CONTENT_FORMAT_EDHOC_CBOR_SEQ)
            );
        }
    }

    #[tokio::test]
    async fn test_edhoc_handshake_survives_lost_message_4() {
        let established = Arc::new(Mutex::new(Vec::new()));
        let state = responder_state(Some(Arc::new(CapturingHook(established.clone()))));
        let server_port = spawn_edhoc_server(state).await;
        let (proxy_port, wire) = spawn_proxy(server_port, true).await;

        let mut client = UdpCoAPClient::new(("127.0.0.1", proxy_port)).await.unwrap();
        client.set_receive_timeout(std::time::Duration::from_millis(200));
        let cred_r = Credential::parse_ccs(&CRED_R).unwrap();
        let initiator_secrets = edhoc_initiate(
            &client,
            EDHOC_WELL_KNOWN_PATH,
            &initiator_identity(),
            Some(cred_r),
        )
        .await
        .unwrap();

        {
            let wire = wire.lock().unwrap();
            // message_1, then message_3 and its retransmission, answered with the same message_4.
            assert_eq!(wire.requests.len(), 3);
            assert_eq!(wire.requests[1].to_bytes(), wire.requests[2].to_bytes());
            assert_eq!(wire.responses.len(), 3);
            assert_eq!(wire.responses[1].payload, wire.responses[2].payload);
        }

        // Only one session may be established, matching what the Initiator derived.
        let established = established.lock().unwrap();
        assert_eq!(established.len(), 1);
        let responder_secrets = &established[0];
        assert_eq!(
            responder_secrets.master_secret,
            initiator_secrets.master_secret
        );
        assert_eq!(responder_secrets.master_salt, initiator_secrets.master_salt);
        assert_eq!(responder_secrets.sender_id, initiator_secrets.recipient_id);
        assert_eq!(responder_secrets.recipient_id, initiator_secrets.sender_id);
    }

    #[tokio::test]
    async fn test_edhoc_concurrent_handshakes_all_succeed() {
        let established = Arc::new(Mutex::new(Vec::new()));
        let state = responder_state(Some(Arc::new(CapturingHook(established.clone()))));
        let store = state.store.clone();
        let port = spawn_edhoc_server(state).await;

        // 30 of 48 possible connection identifiers: randomly picked ones would collide.
        const HANDSHAKES: usize = 30;
        let mut handshakes = tokio::task::JoinSet::new();
        for _ in 0..HANDSHAKES {
            handshakes.spawn(async move {
                let client = UdpCoAPClient::new(("127.0.0.1", port)).await.unwrap();
                let cred_r = Credential::parse_ccs(&CRED_R).unwrap();
                edhoc_initiate(
                    &client,
                    EDHOC_WELL_KNOWN_PATH,
                    &initiator_identity(),
                    Some(cred_r),
                )
                .await
            });
        }
        let mut initiator_master_secrets = Vec::new();
        while let Some(result) = handshakes.join_next().await {
            initiator_master_secrets.push(result.unwrap().unwrap().master_secret);
        }

        assert_eq!(store.pending_sessions(), 0);
        let mut responder_master_secrets: Vec<_> = established
            .lock()
            .unwrap()
            .iter()
            .map(|secrets| secrets.master_secret.clone())
            .collect();
        initiator_master_secrets.sort();
        responder_master_secrets.sort();
        assert_eq!(initiator_master_secrets.len(), HANDSHAKES);
        assert_eq!(initiator_master_secrets, responder_master_secrets);
    }

    #[tokio::test]
    async fn test_completed_session_expires() {
        let cred_i = Credential::parse_ccs(&CRED_I).unwrap();
        let store = EdhocResponderStore::new(EdhocIdentity {
            private_key: R,
            credential: Credential::parse_ccs(&CRED_R).unwrap(),
        });
        let peer: SocketAddr = "127.0.0.1:0".parse().unwrap();
        let payload = b"\x05not-really-a-message-3".to_vec();
        store.completed.lock().unwrap().insert(
            payload.clone(),
            CompletedSession {
                message_4: vec![0x42],
                expires: Instant::now() + COMPLETED_SESSION_LIFETIME,
            },
        );

        // Replayed within EXCHANGE_LIFETIME...
        let replayed = process_edhoc_message(&store, &cred_i, None, peer, &payload)
            .await
            .unwrap();
        assert_eq!(replayed, vec![0x42]);

        // ...and forgotten afterwards.
        store
            .completed
            .lock()
            .unwrap()
            .get_mut(&payload)
            .unwrap()
            .expires = Instant::now() - std::time::Duration::from_secs(1);
        let result = process_edhoc_message(&store, &cred_i, None, peer, &payload).await;
        assert!(matches!(result, Err(EdhocError::UnknownSession)));
        assert!(store.completed.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn test_edhoc_error_for_exhausted_connection_identifiers() {
        let state = responder_state(None);
        let store = state.store.clone();
        let port = spawn_edhoc_server(state).await;

        while store.pending_sessions() < 48 {
            // Fails when the last free identifier is this message_1's C_I; retry until taken.
            let initiator = EdhocInitiator::new(
                new_crypto(),
                EDHOCMethod::StatStat,
                EDHOCSuite::CipherSuite2,
            );
            let (_initiator, message_1) = initiator.prepare_message_1(None, &None).unwrap();
            let _ = process_message_1(&store, message_1.as_slice());
        }

        let client = UdpCoAPClient::new(("127.0.0.1", port)).await.unwrap();
        let cred_r = Credential::parse_ccs(&CRED_R).unwrap();
        let result = edhoc_initiate(
            &client,
            EDHOC_WELL_KNOWN_PATH,
            &initiator_identity(),
            Some(cred_r),
        )
        .await;
        match result {
            Err(EdhocError::Peer(err)) => {
                assert_eq!(err.status, ResponseType::InternalServerError);
                assert_eq!(err.diagnostic(), Some("No connection identifier available"));
            }
            other => panic!("unexpected result: {other:?}"),
        }
    }
}
