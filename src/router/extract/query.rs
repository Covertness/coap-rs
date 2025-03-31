use crate::router::{
    extract::FromRequest,
    request::Request,
    response::{IntoResponse, Response, Status},
};
use serde::de::DeserializeOwned;
use std::ops::{Deref, DerefMut};

#[derive(Debug, Clone, Copy)]
pub enum QueryRejection {
    InvalidQuery,
    InvalidUtf8,
}

impl std::fmt::Display for QueryRejection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            QueryRejection::InvalidQuery => write!(f, "Invalid query string"),
            QueryRejection::InvalidUtf8 => write!(f, "Invalid UTF-8 in query"),
        }
    }
}

impl IntoResponse for QueryRejection {
    fn into_response(self) -> Response {
        let error_message = self.to_string();
        Response::new()
            .set_response_type(Status::BadRequest)
            .set_payload(error_message.into_bytes())
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Query<T>(pub T);

impl<S, T> FromRequest<S> for Query<T>
where
    S: Sync,
    T: DeserializeOwned,
{
    type Rejection = QueryRejection;

    async fn from_request(req: &Request, _state: &S) -> Result<Self, Self::Rejection> {
        let query = req.query();
        let deserializer =
            serde_html_form::Deserializer::new(url::form_urlencoded::parse(query.as_bytes()));
        let value = serde_path_to_error::deserialize(deserializer)
            .map_err(|_| QueryRejection::InvalidQuery)?;
        Ok(Query(value))
    }
}

impl<T> Deref for Query<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> DerefMut for Query<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
