//! A simple router for CoAP requests, inspired by web frameworks like Axum.

pub mod extract;
pub mod handler;
pub mod macros;
pub mod method_routing;
pub mod request;
pub mod response;
pub mod route;
pub mod util;

pub use coap_lite::RequestType as Method;
use handler::{BoxedHandler, Handler, HandlerWrapper};
use method_routing::MethodRouter;
pub use method_routing::{delete, fallback, get, post, put};
pub use request::Request;
use response::IntoResponse;
use route::{Route, RouteError};

/// A simple router that matches incoming CoAP requests to registered handlers based on method and path.
#[derive(Default)]
pub struct Router<S = ()> {
    /// Registered routes, stored in the order they were added. The first matching route will be used.
    routes: Vec<(Route, MethodRouter<S>)>,
    /// Optional fallback handler that is called when no routes match.
    fallback: Option<BoxedHandler<S>>,
    /// Shared application state available to handlers through the `State` extractor.
    state: S,
}

impl<S: Clone + Default + Send + Sync + 'static> Router<S> {
    /// Creates a new empty router with default state.
    pub fn new() -> Self {
        Self::from_state(Default::default())
    }
}

impl<S: Clone + Send + Sync + 'static> Router<S> {
    /// Creates a new empty router with the given state.
    pub fn from_state(state: S) -> Self {
        Self {
            routes: Vec::new(),
            fallback: None,
            state,
        }
    }

    /// Attaches shared application state to the router.
    pub fn with_state(self, state: S) -> Self {
        Self { state, ..self }
    }

    /// Registers a new route with the given path, and method router.
    ///
    /// # Arguments
    ///
    /// - `path`: The path pattern to match (e.g. `"/sensors/{id}"`).
    ///   Path parameters can be defined using `{param}` syntax.
    /// - `method_router`: The method router containing handlers for different CoAP methods.
    ///
    /// # Note
    ///
    /// Routes are matched in the order they are added.
    /// If multiple routes could match a request, the first one will be used.
    pub fn route(mut self, path: impl ToString, method_router: MethodRouter<S>) -> Self {
        let route = Route::new(path);
        self.routes.push((route, method_router));
        self
    }

    /// Sets a fallback handler that will be called when no registered routes match an incoming request.
    pub fn fallback<H, T>(mut self, handler: H) -> Self
    where
        T: Send + Sync + 'static,
        H: Handler<T, S>,
    {
        let handler = Box::new(HandlerWrapper::new(handler));
        self.fallback = Some(handler);
        self
    }

    /// Handles an incoming request by matching it against the registered routes and calling the appropriate handler.
    ///
    /// If no routes match, the fallback handler will be called if it is set, otherwise a Not Found response will be returned.
    pub(crate) async fn handle(&self, mut req: Request) -> Request {
        // routes are explored in order of registration
        for (route, handler) in &self.routes {
            if let Ok(path) = route.match_path(&req.path()) {
                req.path = path;
                match handler.try_handle(req, self.state.clone()).await {
                    Ok(req) => return req,
                    Err(req) => {
                        return self.handle_fallback(req).await;
                    }
                }
            }
        }
        // No route matched, use fallback or return not found
        self.handle_fallback(req).await
    }

    /// Handler for when no routes match an incoming request.
    ///
    /// If a fallback handler is set, it will be called with the request.
    /// Otherwise, a Not Found response will be generated and returned.
    async fn handle_fallback(&self, mut req: Request) -> Request {
        match self.fallback {
            Some(ref fallback) => fallback.call(req, self.state.clone()).await,
            None => {
                RouteError::NotFound.into_response().fill_response(&mut req);
                req
            }
        }
    }
}

#[cfg(test)]
/// Test utilities for the router module.
pub(crate) mod test_utils {
    use crate::{
        router::{
            extract::{Json, Path, Query, State},
            method_routing::{delete, fallback, get, post, put},
            response::{IntoResponse, Response, StatusCode},
            Router,
        },
        Server,
    };
    use serde::Deserialize;
    use std::{
        collections::HashMap,
        fmt::{Display, Formatter, Result},
        sync::{Arc, Mutex},
    };

    /// Simple app state for defining key-value properties that can be shared across handlers.
    #[derive(Debug, Clone, Default)]
    pub(crate) struct AppState(Arc<Mutex<HashMap<String, String>>>);

    impl AppState {
        /// Creates a new empty app state.
        pub(crate) fn new() -> Self {
            AppState(Arc::new(Mutex::new(HashMap::new())))
        }

        /// Inserts a key-value property into the app state.
        pub(crate) fn insert(&self, key: String, value: String) {
            let mut map = self.0.lock().unwrap();
            map.insert(key, value);
        }

        pub(crate) fn clear_state(&self) {
            let mut map = self.0.lock().unwrap();
            map.clear();
        }

        pub(crate) fn remove(&self, key: &str) -> Option<String> {
            let mut map = self.0.lock().unwrap();
            map.remove(key)
        }

        /// Retrieves a value from the app state by key.
        pub(crate) fn get_property(&self, key: &str) -> Option<String> {
            let map = self.0.lock().unwrap();
            map.get(key).cloned()
        }

        pub(crate) fn get_state(&self) -> HashMap<String, String> {
            self.0.lock().unwrap().clone()
        }
    }
    /// Define the arguments you expect in the query string
    #[derive(Debug, Deserialize)]
    pub(crate) struct QueryArgs {
        /// The key to look up in the app state.
        pub key: String,
        /// Optional value
        pub value: Option<String>,
    }

    /// Defines the errors that can occur in the application
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(crate) enum Error {
        /// Error indicating that a property was not found
        PropertyNotFound,
    }

    /// Implements the `Display` trait for the `Error` enum.
    ///
    /// This allows for custom error messages to be displayed when the error is printed.
    impl Display for Error {
        fn fmt(&self, f: &mut Formatter) -> Result {
            match self {
                Error::PropertyNotFound => write!(f, "Property not found"),
            }
        }
    }

    impl IntoResponse for Error {
        fn into_response(self) -> Response {
            let payload = self.to_string().into_bytes();
            Response::new()
                .set_status_code(StatusCode::NotFound)
                .set_payload(payload)
        }
    }

    /// Implements the `Error` trait for the `Error` enum.
    ///
    /// This allows the error to be used in contexts where a standard error is expected.
    impl std::error::Error for Error {}

    /// Example handler that demonstrates using the `State` extractor to generate a response.s
    pub(crate) async fn get_state(state: State<AppState>) -> impl IntoResponse {
        let state = state.get_state();
        Json(state).into_response()
    }

    pub(crate) async fn delete_state(state: State<AppState>) -> impl IntoResponse {
        state.clear_state();
        StatusCode::Valid.into_response()
    }

    /// Example handler that demonstrates using the `Query` and `State` extractors to generate a response.
    pub(crate) async fn get_property(
        query: Query<QueryArgs>,
        state: State<AppState>,
    ) -> impl IntoResponse {
        match state.get_property(&query.key) {
            Some(value) => (StatusCode::Valid, Json(value)).into_response(),
            None => Json(Error::PropertyNotFound).into_response(),
        }
    }

    pub(crate) async fn post_property_query(
        query: Query<QueryArgs>,
        state: State<AppState>,
    ) -> impl IntoResponse {
        if let Some(value) = query.value.as_ref() {
            if state.get_property(&query.key).is_some() {
                (StatusCode::BadRequest, "Property already exists").into_response()
            } else {
                state.insert(query.key.clone(), value.clone());
                StatusCode::Valid.into_response()
            }
        } else {
            (StatusCode::BadRequest, "Missing value in query").into_response()
        }
    }

    /// Example POST handler that demonstrates using the `Path`, `State`, and `Json` extractors to generate a response.
    pub(crate) async fn post_property(
        key: Path<String>,
        state: State<AppState>,
        Json(value): Json<String>,
    ) -> impl IntoResponse {
        if state.get_property(&key).is_some() {
            (StatusCode::BadRequest, "Property already exists").into_response()
        } else {
            state.insert(key.clone(), value);
            StatusCode::Valid.into_response()
        }
    }

    /// Example PUT handler that demonstrates using the `Path`, `State`, and `Json` extractors to generate a response.
    pub(crate) async fn put_property(
        key: Path<String>,
        state: State<AppState>,
        Json(value): Json<String>,
    ) -> impl IntoResponse {
        if state.get_property(&key).is_some() {
            state.insert(key.clone(), value);
            StatusCode::Valid.into_response()
        } else {
            (StatusCode::NotFound, "Property not found").into_response()
        }
    }

    /// Example DELETE handler that demonstrates using the `Path` and `State` extractors to generate a response.
    pub(crate) async fn delete_property(
        Path(key): Path<String>,
        state: State<AppState>,
    ) -> impl IntoResponse {
        match state.remove(&key) {
            Some(_) => StatusCode::Valid.into_response(),
            None => Json(Error::PropertyNotFound).into_response(),
        }
    }

    pub(crate) async fn fallback_handler() -> impl IntoResponse {
        (StatusCode::BadRequest, "Fallback response").into_response()
    }

    pub(crate) async fn route_fallback_handler() -> impl IntoResponse {
        (StatusCode::BadRequest, "Route fallback response").into_response()
    }

    pub(crate) fn build_router() -> Router<AppState> {
        Router::new()
            .with_state(AppState::new())
            .route(
                "/state",
                delete(delete_state)
                    .get(get_state)
                    .fallback(route_fallback_handler),
            )
            .route(
                "/state/property/{key}",
                put(put_property)
                    .post(post_property)
                    .delete(delete_property),
            )
            .route("/property", get(get_property).post(post_property_query))
            .route(
                "/property/{key}",
                post(post_property)
                    .put(put_property)
                    .delete(delete_property),
            )
            .fallback(fallback_handler)
            .route("/fallback", fallback(route_fallback_handler))
    }

    pub(crate) async fn run_router<S: ToString>(addr: S) {
        let router = build_router();
        let server = Server::new_udp(addr.to_string()).unwrap();
        server.serve(router).await;
    }

    pub(crate) fn encode_payload(payload: &str) -> Vec<u8> {
        if serde_json::from_str::<serde_json::Value>(payload).is_ok() {
            payload.as_bytes().to_vec()
        } else {
            serde_json::to_vec(payload).expect("string payload should always serialize to JSON")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::response::StatusCode;
    use super::test_utils::*;
    use crate::client::UdpCoAPClient as Client;
    use coap_lite::RequestType as Method;
    use tokio::time::{sleep, Duration};

    #[tokio::test]
    async fn test_router() {
        // Start the server in the background
        let addr = "127.0.0.1:5684";
        let server_handle = tokio::spawn(async move {
            run_router(addr).await;
        });
        sleep(Duration::from_millis(100)).await;

        // Wrong path (should trigger fallback)
        let response = Client::get(&format!("coap://{}/wrong_path", addr)).await;
        assert!(response.is_ok());
        let response = response.unwrap();
        let status = *response.get_status();
        let payload = String::from_utf8(response.message.payload).unwrap();
        assert_eq!(status, StatusCode::BadRequest);
        assert_eq!(payload, "Fallback response");

        // Wrong method (should trigger route fallback)
        let response = Client::post(&format!("coap://{}/state", addr), b"{}".to_vec()).await;
        assert!(response.is_ok());
        let response = response.unwrap();
        let status = *response.get_status();
        let payload = String::from_utf8(response.message.payload).unwrap();
        assert_eq!(status, StatusCode::BadRequest);
        assert_eq!(payload, "Route fallback response");

        // Another wrong method (should trigger route fallback)
        let response = Client::get(&format!("coap://{}/fallback", addr)).await;
        assert!(response.is_ok());
        let response = response.unwrap();
        let status = *response.get_status();
        let payload = String::from_utf8(response.message.payload).unwrap();
        assert_eq!(status, StatusCode::BadRequest);
        assert_eq!(payload, "Route fallback response");

        // Unknown method on existing path with no route fallback (should trigger global fallback)
        let response =
            Client::request(&format!("coap://{}/property", addr), Method::Fetch, None).await;
        assert!(response.is_ok());
        let response = response.unwrap();
        let status = *response.get_status();
        let payload = String::from_utf8(response.message.payload).unwrap();
        assert_eq!(status, StatusCode::BadRequest);
        assert_eq!(payload, "Fallback response");

        // Get state (should be empty)
        let response = Client::get(&format!("coap://{}/state", addr)).await;
        assert!(response.is_ok());
        let response = response.unwrap();
        let status = *response.get_status();
        let payload = String::from_utf8(response.message.payload).unwrap();
        assert_eq!(status, StatusCode::Content);
        assert_eq!(payload, "{}");

        // Get a property with wrong query format
        let response = Client::get(&format!("coap://{}/property?wrong=test_key", addr)).await;
        assert!(response.is_ok());
        let response = response.unwrap();
        let status = *response.get_status();
        let payload = String::from_utf8(response.message.payload).unwrap();
        assert_eq!(status, StatusCode::BadRequest);
        assert_eq!(payload, "Invalid query string");

        // Get a non-existent property
        let response = Client::get(&format!("coap://{}/property?key=test_key", addr)).await;
        assert!(response.is_ok());
        let response = response.unwrap();
        let status = *response.get_status();
        let payload = String::from_utf8(response.message.payload).unwrap();
        assert_eq!(status, StatusCode::NotFound);
        assert_eq!(payload, "Property not found");

        // Put a non-existent property (should return not found)
        let response = Client::put(
            &format!("coap://{}/property/test_key", addr),
            encode_payload("test_value"),
        )
        .await;
        assert!(response.is_ok());
        let response = response.unwrap();
        let status = *response.get_status();
        let payload = String::from_utf8(response.message.payload).unwrap();
        assert_eq!(status, StatusCode::NotFound);
        assert_eq!(payload, "Property not found");

        // Post a new property
        let response = Client::post(
            &format!("coap://{}/property/test_key", addr),
            encode_payload("test_value"),
        )
        .await;
        assert!(response.is_ok());
        let response = response.unwrap();
        let status = *response.get_status();
        assert_eq!(status, StatusCode::Valid);
        let payload = String::from_utf8(response.message.payload).unwrap();
        assert_eq!(payload, "");

        // Get the property we just set
        let response = Client::get(&format!("coap://{}/property?key=test_key", addr)).await;
        assert!(response.is_ok());
        let response = response.unwrap();
        let status = *response.get_status();
        let payload = String::from_utf8(response.message.payload).unwrap();
        assert_eq!(status, StatusCode::Valid);
        assert_eq!(payload, r#""test_value""#);

        // Post the same property again (should return bad request)
        let response = Client::post(
            &format!("coap://{}/property/test_key", addr),
            encode_payload("test_value"),
        )
        .await;
        assert!(response.is_ok());
        let response = response.unwrap();
        let status = *response.get_status();
        let payload = String::from_utf8(response.message.payload).unwrap();
        assert_eq!(status, StatusCode::BadRequest);
        assert_eq!(payload, "Property already exists");

        // Put an existing property (should update it)
        let response = Client::put(
            &format!("coap://{}/property/test_key", addr),
            encode_payload("updated_value"),
        )
        .await;
        assert!(response.is_ok());
        let response = response.unwrap();
        let status = *response.get_status();
        assert_eq!(status, StatusCode::Valid);
        let payload = String::from_utf8(response.message.payload).unwrap();
        assert_eq!(payload, "");

        // Delete the property
        let response = Client::delete(&format!("coap://{}/property/test_key", addr)).await;
        assert!(response.is_ok());
        let response = response.unwrap();
        let status = *response.get_status();
        assert_eq!(status, StatusCode::Valid);
        let payload = String::from_utf8(response.message.payload).unwrap();
        assert_eq!(payload, "");

        // Try to add property with post_property_query without value (should return bad request)
        let response = Client::post(
            &format!("coap://{}/property?key=query_key", addr),
            Vec::new(),
        )
        .await;
        assert!(response.is_ok());
        let response = response.unwrap();
        let status = *response.get_status();
        let payload = String::from_utf8(response.message.payload).unwrap();
        assert_eq!(status, StatusCode::BadRequest);
        assert_eq!(payload, "Missing value in query");

        // Add a new property with post_property_query
        let response = Client::post(
            &format!("coap://{}/property?key=query_key&value=query_value", addr),
            Vec::new(),
        )
        .await;
        assert!(response.is_ok());
        let response = response.unwrap();
        let status = *response.get_status();
        assert_eq!(status, StatusCode::Valid);
        let payload = String::from_utf8(response.message.payload).unwrap();
        assert_eq!(payload, "");

        // Clear state
        let response = Client::delete(&format!("coap://{}/state", addr)).await;
        assert!(response.is_ok());
        let response = response.unwrap();
        let status = *response.get_status();
        assert_eq!(status, StatusCode::Valid);
        let payload = String::from_utf8(response.message.payload).unwrap();
        assert_eq!(payload, "");

        server_handle.abort();
        let _ = server_handle.await;
    }
}
