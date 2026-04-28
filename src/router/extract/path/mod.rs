//! Extractors for deserializing path parameters from the request URI into a specified type `T`.

use crate::router::{
    extract::FromRequest,
    request::Request,
    response::{IntoResponse, Response, StatusCode},
};
use serde::de::DeserializeOwned;
use std::ops::Deref;

mod de;

/// Extractor for deserializing path parameters from the request URI into a specified type `T`.
#[derive(Debug)]
pub struct Path<T>(pub T);

impl<T> Deref for Path<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<S, T> FromRequest<S> for Path<T>
where
    S: Sync,
    T: DeserializeOwned,
{
    type Rejection = PathDeserializationError;

    async fn from_request(req: &Request, _state: &S) -> Result<Self, Self::Rejection> {
        match T::deserialize(de::PathDeserializer::new(&req.path)) {
            Ok(value) => Ok(Path(value)),
            Err(e) => Err(e),
        }
    }
}

/// Error type for path deserialization failures.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathDeserializationError {
    /// The URI contained the wrong number of parameters.
    WrongNumberOfParameters {
        /// The number of actual parameters in the URI.
        got: usize,
        /// The number of expected parameters.
        expected: usize,
    },

    /// Failed to parse the value at a specific key into the expected type.
    ///
    /// This variant is used when deserializing into types that have named fields, such as structs.
    ParseErrorAtKey {
        /// The key at which the value was located.
        key: String,
        /// The value from the URI.
        value: String,
        /// The expected type of the value.
        expected_type: &'static str,
    },

    /// Failed to parse the value at a specific index into the expected type.
    ///
    /// This variant is used when deserializing into sequence types, such as tuples.
    ParseErrorAtIndex {
        /// The index at which the value was located.
        index: usize,
        /// The value from the URI.
        value: String,
        /// The expected type of the value.
        expected_type: &'static str,
    },

    /// Failed to parse a value into the expected type.
    ///
    /// This variant is used when deserializing into a primitive type (such as `String` and `u32`).
    ParseError {
        /// The value from the URI.
        value: String,
        /// The expected type of the value.
        expected_type: &'static str,
    },

    /// A parameter contained text that, once percent decoded, wasn't valid UTF-8.
    InvalidUtf8InPathParam {
        /// The key at which the invalid value was located.
        key: String,
    },

    /// Tried to serialize into an unsupported type such as nested maps.
    ///
    /// This error kind is caused by programmer errors and thus gets converted into a `500 Internal
    /// Server Error` response.
    UnsupportedType {
        /// The name of the unsupported type.
        name: &'static str,
    },

    /// Failed to deserialize the value with a custom deserialization error.
    DeserializeError {
        /// The key at which the invalid value was located.
        key: String,
        /// The value that failed to deserialize.
        value: String,
        /// The deserializaation failure message.
        message: String,
    },

    /// Catch-all variant for errors that don't fit any other variant.
    Message(String),
}

impl std::fmt::Display for PathDeserializationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Message(error) => error.fmt(f),
            Self::InvalidUtf8InPathParam { key } => write!(f, "Invalid UTF-8 in `{key}`"),
            Self::WrongNumberOfParameters { got, expected } => {
                write!(
                    f,
                    "Wrong number of path arguments for `Path`. Expected {expected} but got {got}"
                )?;

                if *expected == 1 {
                    write!(f, ". Note that multiple parameters must be extracted with a tuple `Path<(_, _)>` or a struct `Path<YourParams>`")?;
                }

                Ok(())
            }
            Self::UnsupportedType { name } => write!(f, "Unsupported type `{name}`"),
            Self::ParseErrorAtKey {
                key,
                value,
                expected_type,
            } => write!(
                f,
                "Cannot parse `{key}` with value `{value}` to a `{expected_type}`"
            ),
            Self::ParseError {
                value,
                expected_type,
            } => write!(f, "Cannot parse `{value}` to a `{expected_type}`"),
            Self::ParseErrorAtIndex {
                index,
                value,
                expected_type,
            } => write!(
                f,
                "Cannot parse value at index {index} with value `{value}` to a `{expected_type}`"
            ),
            Self::DeserializeError {
                key,
                value,
                message,
            } => write!(f, "Cannot parse `{key}` with value `{value}`: {message}"),
        }
    }
}

impl IntoResponse for PathDeserializationError {
    fn into_response(self) -> Response {
        let error_message = self.to_string();
        Response::new()
            .set_status_code(StatusCode::BadRequest)
            .set_payload(error_message.into_bytes())
    }
}

impl serde::de::Error for PathDeserializationError {
    #[inline]
    fn custom<T>(msg: T) -> Self
    where
        T: std::fmt::Display,
    {
        Self::Message(msg.to_string())
    }
}

impl std::error::Error for PathDeserializationError {}

#[cfg(test)]
mod tests {

    use super::*;
    use crate::{
        router::{get, Router},
        Server, UdpCoAPClient as Client,
    };
    use serde::{Deserialize, Serialize};
    use tokio::time::{sleep, Duration};

    #[derive(Deserialize, Serialize, Debug, Clone)]
    struct TestParams {
        id: u32,
        name: String,
    }

    async fn extract_str(Path(value): Path<String>) -> impl IntoResponse {
        value
    }

    async fn extract_bool(Path(value): Path<bool>) -> impl IntoResponse {
        value.to_string()
    }

    async fn extract_u32(Path(value): Path<u32>) -> impl IntoResponse {
        value.to_string()
    }

    async fn extract_tuple(Path((a, b)): Path<(u32, String)>) -> impl IntoResponse {
        format!("a: {a}, b: {b}")
    }

    async fn extract_struct(Path(TestParams { id, name }): Path<TestParams>) -> impl IntoResponse {
        format!("id: {id}, name: {name}")
    }

    fn build_router() -> Router {
        Router::new()
            .route("/str/{value}", get(extract_str))
            .route("/bool/{value}", get(extract_bool))
            .route("/u32/{value}", get(extract_u32))
            .route("/tuple/{a}/{b}", get(extract_tuple))
            .route("/struct/{id}/{name}", get(extract_struct))
    }

    async fn run_router<S: ToString>(addr: S) {
        let router = build_router();
        let server = Server::new_udp(addr.to_string()).unwrap();
        server.serve(router).await;
    }

    #[test]
    fn test_into_response() {
        let error = PathDeserializationError::WrongNumberOfParameters {
            got: 2,
            expected: 3,
        };
        let response = error.into_response();
        assert_eq!(response.status_code, Some(StatusCode::BadRequest));
        let payload_str = String::from_utf8(response.payload.unwrap()).unwrap();
        assert_eq!(
            payload_str,
            "Wrong number of path arguments for `Path`. Expected 3 but got 2"
        );

        let error = PathDeserializationError::UnsupportedType { name: "HashMap" };
        let response = error.into_response();
        assert_eq!(response.status_code, Some(StatusCode::BadRequest));
        let payload_str = String::from_utf8(response.payload.unwrap()).unwrap();
        assert_eq!(payload_str, "Unsupported type `HashMap`");

        let error = PathDeserializationError::ParseErrorAtKey {
            key: "id".to_string(),
            value: "abc".to_string(),
            expected_type: "u32",
        };
        let response = error.into_response();
        assert_eq!(response.status_code, Some(StatusCode::BadRequest));
        let payload_str = String::from_utf8(response.payload.unwrap()).unwrap();
        assert_eq!(payload_str, "Cannot parse `id` with value `abc` to a `u32`");

        let error = PathDeserializationError::ParseError {
            value: "abc".to_string(),
            expected_type: "u32",
        };
        let response = error.into_response();
        assert_eq!(response.status_code, Some(StatusCode::BadRequest));
        let payload_str = String::from_utf8(response.payload.unwrap()).unwrap();
        assert_eq!(payload_str, "Cannot parse `abc` to a `u32`");

        let error = PathDeserializationError::ParseErrorAtIndex {
            index: 0,
            value: "abc".to_string(),
            expected_type: "u32",
        };
        let response = error.into_response();
        assert_eq!(response.status_code, Some(StatusCode::BadRequest));
        let payload_str = String::from_utf8(response.payload.unwrap()).unwrap();
        assert_eq!(
            payload_str,
            "Cannot parse value at index 0 with value `abc` to a `u32`"
        );

        let error = PathDeserializationError::DeserializeError {
            key: "id".to_string(),
            value: "abc".to_string(),
            message: "custom error message".to_string(),
        };
        let response = error.into_response();
        assert_eq!(response.status_code, Some(StatusCode::BadRequest));
        let payload_str = String::from_utf8(response.payload.unwrap()).unwrap();
        assert_eq!(
            payload_str,
            "Cannot parse `id` with value `abc`: custom error message"
        );

        let error = PathDeserializationError::InvalidUtf8InPathParam {
            key: "id".to_string(),
        };
        let response = error.into_response();
        assert_eq!(response.status_code, Some(StatusCode::BadRequest));
        let payload_str = String::from_utf8(response.payload.unwrap()).unwrap();
        assert_eq!(payload_str, "Invalid UTF-8 in `id`");

        let error = PathDeserializationError::Message("custom error".to_string());
        let response = error.into_response();
        assert_eq!(response.status_code, Some(StatusCode::BadRequest));
        let payload_str = String::from_utf8(response.payload.unwrap()).unwrap();
        assert_eq!(payload_str, "custom error");
    }

    #[tokio::test]
    async fn test_extract_path() {
        let addr = "127.0.0.1:5683";
        let server_handle = tokio::spawn(async move {
            run_router(addr).await;
        });
        sleep(Duration::from_millis(100)).await;

        // Test string extraction
        let response = Client::get(&format!("coap://{}/str/hello", addr)).await;
        assert!(response.is_ok());
        let response = response.unwrap();
        let payload = String::from_utf8(response.message.payload).unwrap();
        assert_eq!(payload, "hello");

        // Test bool extraction
        let response = Client::get(&format!("coap://{}/bool/true", addr)).await;
        assert!(response.is_ok());
        let response = response.unwrap();
        let payload = String::from_utf8(response.message.payload).unwrap();
        assert_eq!(payload, "true");

        // Test u32 extraction
        let response = Client::get(&format!("coap://{}/u32/42", addr)).await;
        assert!(response.is_ok());
        let response = response.unwrap();
        let payload = String::from_utf8(response.message.payload).unwrap();
        assert_eq!(payload, "42");

        // Test tuple extraction
        let response = Client::get(&format!("coap://{}/tuple/42/world", addr)).await;
        assert!(response.is_ok());
        let response = response.unwrap();
        let payload = String::from_utf8(response.message.payload).unwrap();
        assert_eq!(payload, "a: 42, b: world");

        // Test struct extraction
        let response = Client::get(&format!("coap://{}/struct/42/world", addr)).await;
        assert!(response.is_ok());
        let response = response.unwrap();
        let payload = String::from_utf8(response.message.payload).unwrap();
        assert_eq!(payload, "id: 42, name: world");

        server_handle.abort();
        let _ = server_handle.await;
    }
}
