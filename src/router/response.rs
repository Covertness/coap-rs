use crate::router::request::Request;
pub use coap_lite::ResponseType as Status;
use std::convert::Infallible;

#[derive(Debug, Clone, Default)]
pub struct Response {
    pub response_type: Option<Status>,
    pub payload: Option<Vec<u8>>,
}

impl Response {
    pub fn new() -> Self {
        Self {
            response_type: None,
            payload: None,
        }
    }

    pub fn set_response_type(mut self, response_type: Status) -> Self {
        self.response_type = Some(response_type);
        self
    }

    pub fn set_payload(mut self, payload: Vec<u8>) -> Self {
        self.payload = Some(payload);
        self
    }

    pub fn fill_response(self, request: &mut Request) {
        if let Some(response) = request.response_mut() {
            if let Some(response_type) = self.response_type {
                response.set_status(response_type);
            }
            if let Some(payload) = &self.payload {
                response.message.payload = payload.clone();
            }
        }
    }
}

/// Trait for generating responses.
///
/// Types that implement `IntoResponse` can be returned from handlers.
pub trait IntoResponse {
    /// Create a response.
    fn into_response(self) -> Response;
}

impl IntoResponse for Response {
    fn into_response(self) -> Response {
        self
    }
}

impl IntoResponse for () {
    fn into_response(self) -> Response {
        Response::new()
    }
}

impl IntoResponse for Infallible {
    fn into_response(self) -> Response {
        match self {}
    }
}

impl<T: IntoResponse, E: IntoResponse> IntoResponse for Result<T, E> {
    fn into_response(self) -> Response {
        match self {
            Ok(value) => value.into_response(),
            Err(err) => err.into_response(),
        }
    }
}

impl IntoResponse for Vec<u8> {
    fn into_response(self) -> Response {
        Response::new().set_payload(self)
    }
}

impl<const N: usize> IntoResponse for [u8; N] {
    fn into_response(self) -> Response {
        self.to_vec().into_response()
    }
}

impl<const N: usize> IntoResponse for &[u8; N] {
    fn into_response(self) -> Response {
        self.to_vec().into_response()
    }
}

impl IntoResponse for &[u8] {
    fn into_response(self) -> Response {
        self.to_vec().into_response()
    }
}

impl IntoResponse for String {
    fn into_response(self) -> Response {
        self.into_bytes().into_response()
    }
}

impl IntoResponse for &str {
    fn into_response(self) -> Response {
        self.as_bytes().into_response()
    }
}

impl<T: IntoResponse> IntoResponse for Box<T> {
    fn into_response(self) -> Response {
        (*self).into_response()
    }
}
