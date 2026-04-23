//! Extractors for request JSON body data.

use crate::router::{
    extract::FromRequest,
    request::Request,
    response::{IntoResponse, Response, Status},
};
use serde::{de::DeserializeOwned, Serialize};
use std::ops::{Deref, DerefMut};

/// Error types that can occur when extracting data from the request JSON body.
#[derive(Debug, Clone, Copy)]
pub enum JsonRejection {
    /// The request JSON body has an invalid format.
    InvalidBody,
    /// Failed to deserialize the JSON body.
    DeserializationError,
    /// The JSON body contains invalid UTF-8.
    InvalidUtf8,
}

impl std::fmt::Display for JsonRejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JsonRejection::InvalidBody => write!(f, "Invalid JSON body string"),
            JsonRejection::DeserializationError => write!(f, "Failed to deserialize JSON body"),
            JsonRejection::InvalidUtf8 => write!(f, "Invalid UTF-8 in JSON body"),
        }
    }
}

impl IntoResponse for JsonRejection {
    fn into_response(self) -> Response {
        let error_message = self.to_string();
        Response::new()
            .set_response_type(Status::BadRequest)
            .set_payload(error_message.into_bytes())
    }
}

/// Extractor for deserializing JSON body data into a specified type `T`.
#[derive(Debug, Clone, Copy, Default)]
pub struct Json<T>(pub T);

impl<S, T> FromRequest<S> for Json<T>
where
    S: Sync,
    T: DeserializeOwned,
{
    type Rejection = JsonRejection;

    async fn from_request(req: &Request, _state: &S) -> Result<Self, Self::Rejection> {
        // Convert payload bytes to UTF-8 string
        let body_str = String::from_utf8(req.payload()).map_err(|_| JsonRejection::InvalidUtf8)?;

        // Deserialize JSON string into type T
        serde_json::from_str::<T>(&body_str)
            .map(Json)
            .map_err(|_| JsonRejection::DeserializationError)
    }
}

impl<T: Serialize> IntoResponse for Json<T> {
    fn into_response(self) -> Response {
        // Serialize the inner value to JSON string
        match serde_json::to_string(&self.0) {
            Ok(json_str) => Response::new().set_payload(json_str.into_bytes()),
            Err(_) => Response::new()
                .set_response_type(Status::InternalServerError)
                .set_payload(b"Failed to serialize response body".to_vec()),
        }
    }
}

impl<T> Deref for Json<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> DerefMut for Json<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
