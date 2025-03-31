use crate::router::{
    extract::FromRequest,
    request::Request,
    response::{IntoResponse, Response, Status},
};
use serde::de::DeserializeOwned;
use std::ops::{Deref, DerefMut};

mod de;

#[derive(Debug)]
pub struct Path<T>(pub T);

impl<T> Deref for Path<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> DerefMut for Path<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
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
            .set_response_type(Status::BadRequest)
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
