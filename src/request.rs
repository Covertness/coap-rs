use std::net::SocketAddr;

pub use coap_lite::{
    CoapOption, CoapRequest, MessageClass, MessageType, ObserveOption, Packet,
    RequestType as Method, ResponseType as Status,
};

/// A builder for creating CoAP requests using coap_lite
pub struct RequestBuilder<'a> {
    path: &'a str,
    method: Method,
    data: Option<Vec<u8>>,
    queries: Option<Vec<u8>>,
    domain: String,
    confirmable: bool,
    token: Option<Vec<u8>>,
    options: Vec<(CoapOption, Vec<u8>)>,
}
impl<'a> RequestBuilder<'a> {
    pub fn new(path: &'a str, method: Method) -> Self {
        Self {
            path,
            method,
            data: None,
            queries: None,
            token: None,
            domain: "".to_string(),
            confirmable: true,
            options: vec![],
        }
    }

    /// Create a new request with the given path, method, optional payload, optional query, and
    /// domain.
    pub fn request_path(
        path: &'a str,
        method: Method,
        payload: Option<Vec<u8>>,
        query: Option<Vec<u8>>,
        domain: Option<String>,
    ) -> Self {
        let new_self = Self::new(path, method);
        Self {
            data: payload,
            queries: query,
            domain: domain.unwrap_or_else(|| "".to_string()),
            ..new_self
        }
    }

    /// Set the payload of the request.
    pub fn data(mut self, data: Option<Vec<u8>>) -> Self {
        self.data = data;
        self
    }
    /// set the queries of the request.
    pub fn queries(mut self, queries: Option<Vec<u8>>) -> Self {
        self.queries = queries;
        self
    }
    /// set the domain of the request.
    pub fn domain(mut self, domain: String) -> Self {
        self.domain = domain;
        self
    }
    /// set whether the request is confirmable.
    pub fn confirmable(mut self, confirmable: bool) -> Self {
        self.confirmable = confirmable;
        self
    }
    /// set the token of the request
    pub fn token(mut self, token: Option<Vec<u8>>) -> Self {
        self.token = token;
        self
    }
    /// set the options of the request
    pub fn options(mut self, options: Vec<(CoapOption, Vec<u8>)>) -> Self {
        self.options = options;
        self
    }

    pub fn build(self) -> CoapRequest<SocketAddr> {
        let mut request = CoapRequest::new();
        request.set_method(self.method);
        request.set_path(self.path);
        if let Some(queries) = self.queries {
            request.message.add_option(CoapOption::UriQuery, queries);
        }
        for (opt, opt_data) in self.options {
            assert_ne!(opt, CoapOption::UriQuery, "Use queries instead");
            request.message.add_option(opt, opt_data);
        }
        if self.domain.len() != 0 {
            request.message.add_option(
                CoapOption::UriHost,
                self.domain.as_str().as_bytes().to_vec(),
            );
        }
        if let Some(data) = self.data {
            request.message.payload = data;
        }
        match self.confirmable {
            true => request.message.header.set_type(MessageType::Confirmable),
            false => request.message.header.set_type(MessageType::NonConfirmable),
        };
        if let Some(tok) = self.token {
            request.message.set_token(tok);
        }
        return request;
    }
}
#[cfg(test)]
pub mod test {
    pub use super::*;

    #[test]
    fn test_request_has_payload() {
        let build = RequestBuilder::request_path(
            "/",
            Method::Put,
            Some(b"hello, world!".to_vec()),
            None,
            None,
        )
        .build();
        assert_eq!(build.message.payload.as_slice(), b"hello, world!");
    }
    #[test]
    fn test_domain() {
        let build = RequestBuilder::request_path(
            "/",
            Method::Put,
            None,
            None,
            Some("example.com".to_string()),
        )
        .build();
        assert_eq!(
            build.message.get_first_option(CoapOption::UriHost).unwrap(),
            b"example.com"
        );
    }

    #[test]
    fn test_domain_and_other_options() {
        let options = vec![
            (CoapOption::ProxyUri, b"coap://foo.com".to_vec()),
            (CoapOption::Block2, b"fake".to_vec()),
        ];
        let build = RequestBuilder::request_path(
            "/",
            Method::Put,
            None,
            b"query=hello".to_vec().into(),
            Some("example.com".to_string()),
        )
        .options(options)
        .build();
        assert_eq!(
            build.message.get_first_option(CoapOption::UriHost).unwrap(),
            b"example.com"
        );
        assert_eq!(
            build
                .message
                .get_first_option(CoapOption::UriQuery)
                .unwrap(),
            b"query=hello"
        );
        assert_eq!(
            build
                .message
                .get_first_option(CoapOption::ProxyUri)
                .unwrap(),
            b"coap://foo.com"
        );
        assert_eq!(
            build.message.get_first_option(CoapOption::Block2).unwrap(),
            b"fake"
        )
    }

    #[test]
    fn test_request_token() {
        let build = RequestBuilder::new("/", Method::Put)
            .token(Some(b"token".to_vec()))
            .build();
        assert_eq!(build.message.get_token(), b"token");
    }
    #[test]
    fn test_confirmable_request() {
        let build = RequestBuilder::new("/", Method::Put)
            .confirmable(true)
            .build();
        assert_eq!(build.message.header.get_type(), MessageType::Confirmable);
    }
}
