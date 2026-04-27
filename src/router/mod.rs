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
