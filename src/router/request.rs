use crate::router::util::PercentDecodedStr;
use coap_lite::{CoapOption, CoapRequest};
use std::{net::SocketAddr, sync::Arc};

pub struct Request {
    pub req: Box<CoapRequest<SocketAddr>>,
    pub(crate) path: Vec<(Arc<str>, PercentDecodedStr)>,
}

impl Request {
    pub fn new(req: Box<CoapRequest<SocketAddr>>) -> Self {
        Self {
            req,
            path: Vec::new(),
        }
    }

    pub fn response(&self) -> Option<&coap_lite::CoapResponse> {
        self.req.response.as_ref()
    }

    pub fn response_mut(&mut self) -> Option<&mut coap_lite::CoapResponse> {
        self.req.response.as_mut()
    }

    pub fn method(&self) -> coap_lite::RequestType {
        *self.req.get_method()
    }

    pub fn path_as_vec(&self) -> Vec<String> {
        self.req.get_path_as_vec().unwrap_or_default()
    }

    pub fn path(&self) -> String {
        self.path_as_vec().join("/")
    }

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

    pub fn query(&self) -> String {
        self.query_as_vec().join("&")
    }

    pub fn payload(&self) -> Vec<u8> {
        self.req.message.payload.clone()
    }
}
