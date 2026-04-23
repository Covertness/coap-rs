//! Response types and traits for the router.

use crate::router::request::Request;
pub use coap_lite::ResponseType as StatusCode;
use std::convert::Infallible;

/// The response type returned by handlers.
#[derive(Debug, Clone, Default)]
pub struct Response {
    /// The CoAP response type (e.g. `Content`, `BadRequest`, etc.) to set in the response message.
    pub status_code: Option<StatusCode>,
    /// The payload to include in the response message, if any.
    pub payload: Option<Vec<u8>>,
}

impl Response {
    /// Creates a new empty response with no response type or payload.
    pub fn new() -> Self {
        Self {
            status_code: None,
            payload: None,
        }
    }

    /// Sets the status code for this response.
    pub fn set_status_code(mut self, status_code: StatusCode) -> Self {
        self.status_code = Some(status_code);
        self
    }

    /// Sets the payload for this response.
    pub fn set_payload(mut self, payload: Vec<u8>) -> Self {
        self.payload = Some(payload);
        self
    }

    /// Fills the given request's response with the response type and payload from this `Response`.
    ///
    /// This method consumes the `Response` and modifies the request's response in-place.
    pub fn fill_response(self, request: &mut Request) {
        if let Some(response) = request.response_mut() {
            if let Some(status_code) = self.status_code {
                response.set_status(status_code);
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

// Implement IntoResponse for common types to allow easy response generation from handlers.

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

impl IntoResponse for StatusCode {
    fn into_response(self) -> Response {
        Response::new().set_status_code(self)
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

impl<T: IntoResponse> IntoResponse for (StatusCode, T) {
    fn into_response(self) -> Response {
        let (status, payload) = self;
        payload.into_response().set_status_code(status)
    }
}
