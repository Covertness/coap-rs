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

#[derive(Default)]
pub struct Router<S = ()> {
    routes: Vec<(Route, BoxedHandler<S>)>,
    fallback: Option<BoxedHandler<S>>,
}

impl<S: Clone + Send + Sync + 'static> Router<S> {
    pub fn new() -> Self {
        Self {
            routes: Vec::new(),
            fallback: None,
        }
    }

    pub fn route(mut self, method: Method, path: impl ToString, handler: BoxedHandler<S>) -> Self {
        let route = Route::new(method, path);
        self.routes.push((route, handler));
        self
    }

    pub fn get<H, T>(self, path: impl ToString, handler: H) -> Self
    where
        T: Send + Sync + 'static,
        H: Handler<T, S>,
    {
        let handler = Box::new(HandlerWrapper::new(handler));
        self.route(Method::Get, path, handler)
    }

    pub fn post<H, T>(self, path: impl ToString, handler: H) -> Self
    where
        T: Send + Sync + 'static,
        H: Handler<T, S>,
    {
        let handler = Box::new(HandlerWrapper::new(handler));
        self.route(Method::Post, path, handler)
    }

    pub fn put<H, T>(self, path: impl ToString, handler: H) -> Self
    where
        T: Send + Sync + 'static,
        H: Handler<T, S>,
    {
        let handler = Box::new(HandlerWrapper::new(handler));
        self.route(Method::Put, path, handler)
    }

    pub fn delete<H, T>(self, path: impl ToString, handler: H) -> Self
    where
        T: Send + Sync + 'static,
        H: Handler<T, S>,
    {
        let handler = Box::new(HandlerWrapper::new(handler));
        self.route(Method::Delete, path, handler)
    }

    pub fn fallback<H, T>(mut self, handler: H) -> Self
    where
        T: Send + Sync + 'static,
        H: Handler<T, S>,
    {
        let handler = Box::new(HandlerWrapper::new(handler));
        self.fallback = Some(handler);
        self
    }

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
