//! Extractors for request data.

use crate::router::{request::Request, response::IntoResponse};
use std::future::Future;

mod json;
mod path;
mod query;
mod state;
mod tuple;

pub use json::Json;
pub use path::Path;
pub use query::Query;
pub use state::State;

/// Extractor trait for extracting data from a request and router state, with error handling.
pub trait FromRequest<S>: Sized {
    /// The type of error that can occur during extraction.
    type Rejection: IntoResponse;

    /// Extracts data from the given request and router state, returning either the extracted value or a rejection error.
    fn from_request(
        req: &Request,
        state: &S,
    ) -> impl Future<Output = Result<Self, Self::Rejection>> + Send;
}

/// Helper trait for extracting a value from a reference.
pub trait FromRef<T> {
    fn from_ref(input: &T) -> Self;
}

impl<T: Clone> FromRef<T> for T {
    fn from_ref(input: &T) -> Self {
        input.clone()
    }
}
