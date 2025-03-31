use crate::router::{request::Request, response::IntoResponse};
use std::future::Future;

mod body;
mod path;
mod query;
mod state;
mod tuple;

pub trait FromRequest<S>: Sized {
    type Rejection: IntoResponse;

    fn from_request(
        req: &Request,
        state: &S,
    ) -> impl Future<Output = Result<Self, Self::Rejection>> + Send;
}

pub trait FromRef<T> {
    fn from_ref(input: &T) -> Self;
}

impl<T: Clone> FromRef<T> for T {
    fn from_ref(input: &T) -> Self {
        input.clone()
    }
}

pub trait OptionalFromRequest<S>: Sized {
    type Rejection: IntoResponse;

    fn from_request(
        req: &Request,
        state: &S,
    ) -> impl Future<Output = Result<Option<Self>, Self::Rejection>> + Send;
}

impl<S, T> FromRequest<S> for Option<T>
where
    T: OptionalFromRequest<S>,
    S: Send + Sync,
{
    type Rejection = T::Rejection;

    async fn from_request(req: &Request, state: &S) -> Result<Option<T>, Self::Rejection> {
        T::from_request(req, state).await
    }
}

pub use body::Body;
pub use path::Path;
pub use query::Query;
pub use state::State;
