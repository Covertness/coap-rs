//! Extractors for request CBOR body data.

use crate::router::{
    extract::FromRequest,
    request::Request,
    response::{IntoResponse, Response, StatusCode},
};
use serde::{de::DeserializeOwned, Serialize};
use std::ops::Deref;

/// Error types that can occur when extracting data from the request CBOR body.
#[derive(Debug, Clone, Copy)]
pub enum CborRejection {
    /// The request CBOR body has an invalid format.
    InvalidBody,
    /// Failed to deserialize the CBOR body.
    DeserializationError,
}

impl std::fmt::Display for CborRejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CborRejection::InvalidBody => write!(f, "Invalid CBOR body"),
            CborRejection::DeserializationError => write!(f, "Failed to deserialize CBOR body"),
        }
    }
}

impl IntoResponse for CborRejection {
    fn into_response(self) -> Response {
        let error_message = self.to_string();
        Response::new()
            .set_status_code(StatusCode::BadRequest)
            .set_payload(error_message.into_bytes())
    }
}

/// Extractor for deserializing CBOR body data into a specified type `T`.
#[derive(Debug, Clone, Copy, Default)]
pub struct Cbor<T>(pub T);

impl<S, T> FromRequest<S> for Cbor<T>
where
    S: Sync,
    T: DeserializeOwned,
{
    type Rejection = CborRejection;

    async fn from_request(req: &Request, _state: &S) -> Result<Self, Self::Rejection> {
        // Deserialize CBOR into type T
        ciborium::from_reader(req.payload().as_slice())
            .map(Cbor)
            .map_err(|_| CborRejection::DeserializationError)
    }
}

impl<T: Serialize> IntoResponse for Cbor<T> {
    fn into_response(self) -> Response {
        // Serialize the inner value to CBOR
        let mut cbor_vec = Vec::new();
        match ciborium::into_writer(&self.0, &mut cbor_vec) {
            Ok(_) => Response::new().set_payload(cbor_vec),
            Err(_) => Response::new()
                .set_status_code(StatusCode::InternalServerError)
                .set_payload(b"Failed to serialize response body".to_vec()),
        }
    }
}

impl<T> Deref for Cbor<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn test_into_response() {
        let rejection = CborRejection::InvalidBody;
        let response = rejection.into_response();

        assert_eq!(response.status_code, Some(StatusCode::BadRequest));
        assert!(response.payload.is_some());
        let payload_str = String::from_utf8(response.payload.unwrap()).unwrap();
        assert_eq!(payload_str, "Invalid CBOR body");

        let rejection = CborRejection::DeserializationError;
        let response = rejection.into_response();
        assert_eq!(response.status_code, Some(StatusCode::BadRequest));
        assert!(response.payload.is_some());
        let payload_str = String::from_utf8(response.payload.unwrap()).unwrap();
        assert_eq!(payload_str, "Failed to deserialize CBOR body");

        let cbor = Cbor(vec![1, 2, 3]);
        let response = cbor.into_response();
        assert_eq!(response.status_code, None);
        let payload_bytes = response.payload.unwrap();
        assert_eq!(payload_bytes, [0x83, 0x01, 0x02, 0x03]);

        let cbor = Cbor(vec![1, 2, 3]);
        let response = cbor.into_response();
        assert_eq!(response.status_code, None);
        let payload_bytes = response.payload.unwrap();
        assert_eq!(payload_bytes, [0x83, 0x01, 0x02, 0x03]);

        let mut payload = HashMap::new();
        payload.insert("key".to_string(), "value".to_string());
        payload.insert("number".to_string(), "42".to_string());

        let cbor = Cbor(payload.clone());
        let response = cbor.into_response();
        assert_eq!(response.status_code, None);
        let payload_bytes = response.payload.unwrap();
        let payload_hashmap: HashMap<String, String> =
            ciborium::from_reader(payload_bytes.as_slice()).unwrap();
        assert_eq!(payload, payload_hashmap);
    }
}
