use crate::router::{
    extract::FromRequest,
    request::Request,
    response::{IntoResponse, Response, Status},
};
use serde::de::DeserializeOwned;
use std::ops::{Deref, DerefMut};

#[derive(Debug, Clone, Copy)]
pub enum BodyRejection {
    InvalidBody,
    DeserializationError,
    InvalidUtf8,
}

impl std::fmt::Display for BodyRejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BodyRejection::InvalidBody => write!(f, "Invalid body string"),
            BodyRejection::DeserializationError => write!(f, "Failed to deserialize JSON body"),
            BodyRejection::InvalidUtf8 => write!(f, "Invalid UTF-8 in body"),
        }
    }
}

impl IntoResponse for BodyRejection {
    fn into_response(self) -> Response {
        let error_message = self.to_string();
        Response::new()
            .set_response_type(Status::BadRequest)
            .set_payload(error_message.into_bytes())
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Body<T>(pub T);

impl<S, T> FromRequest<S> for Body<T>
where
    S: Sync,
    T: DeserializeOwned,
{
    type Rejection = BodyRejection;

    async fn from_request(req: &Request, _state: &S) -> Result<Self, Self::Rejection> {
        // Convert payload bytes to UTF-8 string
        let body_str = String::from_utf8(req.payload()).map_err(|_| BodyRejection::InvalidUtf8)?;

        // Deserialize JSON
        serde_json::from_str::<T>(&body_str)
            .map(Body)
            .map_err(|_| BodyRejection::DeserializationError)
    }
}

impl<T> Deref for Body<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> DerefMut for Body<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
