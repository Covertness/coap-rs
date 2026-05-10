use crate::router::{
    handler::{BoxedHandler, Handler, HandlerWrapper},
    Request,
};
use coap_lite::RequestType as Method;

/// A simple router that matches incoming CoAP requests to registered handlers based on method.
pub struct MethodRouter<S> {
    /// Handler for GET requests, if registered.
    get: Option<BoxedHandler<S>>,
    /// Handler for DELETE requests, if registered.
    delete: Option<BoxedHandler<S>>,
    /// Handler for POST requests, if registered.
    post: Option<BoxedHandler<S>>,
    /// Handler for PUT requests, if registered.
    put: Option<BoxedHandler<S>>,
    /// Optional fallback handler that is called when no method-specific handler matches.
    fallback: Option<BoxedHandler<S>>,
}

impl<S: Clone + Send + Sync + 'static> MethodRouter<S> {
    /// Adds a handler for GET requests to this method router.
    pub fn get<H, T>(self, handler: H) -> Self
    where
        T: Send + Sync + 'static,
        H: Handler<T, S>,
    {
        let handler = Box::new(HandlerWrapper::new(handler));
        Self {
            get: Some(handler),
            ..self
        }
    }

    /// Adds a handler for DELETE requests to this method router.
    pub fn delete<H, T>(self, handler: H) -> Self
    where
        T: Send + Sync + 'static,
        H: Handler<T, S>,
    {
        let handler = Box::new(HandlerWrapper::new(handler));
        Self {
            delete: Some(handler),
            ..self
        }
    }

    /// Adds a handler for POST requests to this method router.
    pub fn post<H, T>(self, handler: H) -> Self
    where
        T: Send + Sync + 'static,
        H: Handler<T, S>,
    {
        let handler = Box::new(HandlerWrapper::new(handler));
        Self {
            post: Some(handler),
            ..self
        }
    }

    /// Adds a handler for PUT requests to this method router.
    pub fn put<H, T>(self, handler: H) -> Self
    where
        T: Send + Sync + 'static,
        H: Handler<T, S>,
    {
        let handler = Box::new(HandlerWrapper::new(handler));
        Self {
            put: Some(handler),
            ..self
        }
    }

    /// Adds a fallback handler to this method router.
    ///
    /// The fallback handler will be called if a request matches the route but does not have a handler for its method.
    pub fn fallback<H, T>(self, handler: H) -> Self
    where
        T: Send + Sync + 'static,
        H: Handler<T, S>,
    {
        let handler = Box::new(HandlerWrapper::new(handler));
        Self {
            fallback: Some(handler),
            ..self
        }
    }

    /// Tries to handle the given request using the handler for its method, if it exists.
    /// If the handler returns an error, the fallback handler will be tried if it exists.
    /// If no handler matches, the request is returned as an error.
    pub(crate) async fn try_handle(&self, request: Request, state: S) -> Result<Request, Request> {
        let handler = match request.method() {
            Method::Get => self.get.as_ref(),
            Method::Delete => self.delete.as_ref(),
            Method::Post => self.post.as_ref(),
            Method::Put => self.put.as_ref(),
            _ => None,
        };
        if let Some(handler) = handler {
            Ok(handler.clone().call(request, state.clone()).await)
        } else if let Some(fallback) = self.fallback.as_ref() {
            Ok(fallback.clone().call(request, state.clone()).await)
        } else {
            Err(request)
        }
    }
}

/// Helper function to create a new `MethodRouter` with a handler for GET requests.
pub fn get<H, T, S>(handler: H) -> MethodRouter<S>
where
    T: Send + Sync + 'static,
    H: Handler<T, S>,
    S: Send + Sync + 'static,
{
    let handler = Box::new(HandlerWrapper::new(handler));
    MethodRouter {
        get: Some(handler),
        delete: None,
        post: None,
        put: None,
        fallback: None,
    }
}

/// Helper function to create a new `MethodRouter` with a handler for DELETE requests.
pub fn delete<H, T, S>(handler: H) -> MethodRouter<S>
where
    T: Send + Sync + 'static,
    H: Handler<T, S>,
    S: Send + Sync + 'static,
{
    let handler = Box::new(HandlerWrapper::new(handler));
    MethodRouter {
        get: None,
        delete: Some(handler),
        post: None,
        put: None,
        fallback: None,
    }
}

/// Helper function to create a new `MethodRouter` with a handler for POST requests.
pub fn post<H, T, S>(handler: H) -> MethodRouter<S>
where
    T: Send + Sync + 'static,
    H: Handler<T, S>,
    S: Send + Sync + 'static,
{
    let handler = Box::new(HandlerWrapper::new(handler));
    MethodRouter {
        get: None,
        delete: None,
        post: Some(handler),
        put: None,
        fallback: None,
    }
}

/// Helper function to create a new `MethodRouter` with a handler for PUT requests.
pub fn put<H, T, S>(handler: H) -> MethodRouter<S>
where
    T: Send + Sync + 'static,
    H: Handler<T, S>,
    S: Send + Sync + 'static,
{
    let handler = Box::new(HandlerWrapper::new(handler));
    MethodRouter {
        get: None,
        delete: None,
        post: None,
        put: Some(handler),
        fallback: None,
    }
}

/// Helper function to create a new `MethodRouter` with a fallback handler.
pub fn fallback<H, T, S>(handler: H) -> MethodRouter<S>
where
    T: Send + Sync + 'static,
    H: Handler<T, S>,
    S: Send + Sync + 'static,
{
    let handler = Box::new(HandlerWrapper::new(handler));
    MethodRouter {
        get: None,
        delete: None,
        post: None,
        put: None,
        fallback: Some(handler),
    }
}
