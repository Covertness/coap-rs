//! A simple router for CoAP requests, inspired by web frameworks like Axum.

pub mod extract;
pub mod handler;
pub mod macros;
pub mod request;
pub mod response;
pub mod route;
pub mod util;

pub use coap_lite::RequestType as Method;
use handler::{BoxedHandler, Handler, HandlerWrapper};
pub use request::Request;
use response::IntoResponse;
use route::{Route, RouteError};

/// A simple router that matches incoming CoAP requests to registered handlers based on method and path.
#[derive(Default)]
pub struct Router<S = ()> {
    /// Registered routes, stored in the order they were added. The first matching route will be used.
    routes: Vec<(Route, BoxedHandler<S>)>,
    /// Optional fallback handler that is called when no routes match.
    fallback: Option<BoxedHandler<S>>,
}

impl<S: Clone + Send + Sync + 'static> Router<S> {
    /// Creates a new empty router.
    pub fn new() -> Self {
        Self {
            routes: Vec::new(),
            fallback: None,
        }
    }

    /// Registers a new route with the given method, path, and handler.
    ///
    /// # Arguments
    ///
    /// - `method`: The CoAP [request type](Method) (e.g. GET, POST) to match.
    /// - `path`: The path pattern to match (e.g. `"/sensors/{id}"`).
    ///   Path parameters can be defined using `{param}` syntax.
    /// - `handler`: The handler function that will be called when a request matches this route.
    ///
    /// # Note
    ///
    /// Routes are matched in the order they are added.
    /// If multiple routes could match a request, the first one will be used.
    pub fn route(mut self, method: Method, path: impl ToString, handler: BoxedHandler<S>) -> Self {
        let route = Route::new(method, path);
        self.routes.push((route, handler));
        self
    }

    /// Convenience method for registering a GET route. See [`route`](Self::route) for details.
    pub fn get<H, T>(self, path: impl ToString, handler: H) -> Self
    where
        T: Send + Sync + 'static,
        H: Handler<T, S>,
    {
        let handler = Box::new(HandlerWrapper::new(handler));
        self.route(Method::Get, path, handler)
    }

    /// Convenience method for registering a POST route. See [`route`](Self::route) for details.
    pub fn post<H, T>(self, path: impl ToString, handler: H) -> Self
    where
        T: Send + Sync + 'static,
        H: Handler<T, S>,
    {
        let handler = Box::new(HandlerWrapper::new(handler));
        self.route(Method::Post, path, handler)
    }

    /// Convenience method for registering a PUT route. See [`route`](Self::route) for details.
    pub fn put<H, T>(self, path: impl ToString, handler: H) -> Self
    where
        T: Send + Sync + 'static,
        H: Handler<T, S>,
    {
        let handler = Box::new(HandlerWrapper::new(handler));
        self.route(Method::Put, path, handler)
    }

    /// Convenience method for registering a DELETE route. See [`route`](Self::route) for details.
    pub fn delete<H, T>(self, path: impl ToString, handler: H) -> Self
    where
        T: Send + Sync + 'static,
        H: Handler<T, S>,
    {
        let handler = Box::new(HandlerWrapper::new(handler));
        self.route(Method::Delete, path, handler)
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
    pub(crate) async fn handle(&self, mut req: Request, state: S) -> Request {
        // routes are explored in order of registration
        for (route, handler) in &self.routes {
            if let Ok(path) = route.match_request(&mut req) {
                req.path = path;
                return handler.call(req, state).await;
            }
        }
        // No route matched, use fallback or return not found
        match self.fallback {
            Some(ref fallback) => fallback.call(req, state).await,
            None => {
                RouteError::NotFound.into_response().fill_response(&mut req);
                req
            }
        }
    }
}
