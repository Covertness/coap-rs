//! Types and functions related to handling incoming CoAP requests in the router.

use crate::router::util::PercentDecodedStr;
use coap_lite::{CoapOption, CoapRequest};
use std::{net::SocketAddr, sync::Arc};

/// A wrapper around `CoapRequest` that includes additional information extracted from the request,
/// such as the decoded path segments. This is the type that handlers will receive when a request is matched.
pub struct Request {
    /// The original CoAP request, wrapped in a `Box`.
    pub req: Box<CoapRequest<SocketAddr>>,
    /// The decoded path segments of the request, stored as a vector of (raw, decoded) pairs.
    pub(crate) path: Vec<(Arc<str>, PercentDecodedStr)>,
}

impl Request {
    /// Creates a new `Request` from a `CoapRequest`.
    pub fn new(req: Box<CoapRequest<SocketAddr>>) -> Self {
        Self {
            req,
            path: Vec::new(),
        }
    }

    /// Returns a mutable reference to the CoAP response associated with this request, if it exists.
    pub fn response_mut(&mut self) -> Option<&mut coap_lite::CoapResponse> {
        self.req.response.as_mut()
    }

    /// Returns the CoAP request method (e.g. GET, POST).
    pub fn method(&self) -> coap_lite::RequestType {
        *self.req.get_method()
    }

    /// Returns the raw path segments of the request as a vector of strings.
    pub fn path_as_vec(&self) -> Vec<String> {
        self.req.get_path_as_vec().unwrap_or_default()
    }

    /// Returns the decoded path of the request as a single string, with segments joined by "/".
    pub fn path(&self) -> String {
        self.path_as_vec().join("/")
    }

    /// Returns the query parameters of the request as a vector of strings.
    pub fn query_as_vec(&self) -> Vec<String> {
        let mut vec = Vec::new();
        if let Some(options) = self.req.message.get_option(CoapOption::UriQuery) {
            for option in options.iter() {
                if let Ok(seg) = core::str::from_utf8(option) {
                    vec.push(seg.to_string());
                }
            }
        };
        vec
    }

    /// Returns the query parameters of the request as a single string, with parameters joined by "&".
    pub fn query(&self) -> String {
        self.query_as_vec().join("&")
    }

    /// Returns the payload of the request as a vector of bytes.
    pub fn payload(&self) -> Vec<u8> {
        self.req.message.payload.clone()
    }
}
