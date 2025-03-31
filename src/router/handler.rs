use crate::router::{
    extract::FromRequest, macros::all_the_tuples, request::Request, response::IntoResponse,
};
use std::{future::Future, pin::Pin};

// Type-erased handler trait (no T parameter)
pub trait BoxableHandler<S>: Send + Sync + 'static {
    fn call(&self, req: Request, state: S) -> Pin<Box<dyn Future<Output = Request> + Send>>;
    fn clone_box(&self) -> Box<dyn BoxableHandler<S>>;
}

// Enable cloning of boxed handlers
impl<S: 'static> Clone for Box<dyn BoxableHandler<S>> {
    fn clone(&self) -> Self {
        self.clone_box()
    }
}

impl<T, S, H> BoxableHandler<S> for HandlerWrapper<T, S, H>
where
    T: Sync + Send + 'static,
    S: Send + Sync + 'static,
    H: Handler<T, S> + Clone + Send + Sync + 'static,
{
    fn call(&self, req: Request, state: S) -> Pin<Box<dyn Future<Output = Request> + Send>> {
        let clone = self.clone();
        Box::pin(H::call(clone.handler, req, state))
    }
    fn clone_box(&self) -> Box<dyn BoxableHandler<S>> {
        Box::new(self.clone())
    }
}

// Updated BoxedHandler type
pub type BoxedHandler<S> = Box<dyn BoxableHandler<S>>;

pub struct HandlerWrapper<T, S, H: Handler<T, S>> {
    handler: H,
    _marker: std::marker::PhantomData<(T, S)>,
}

impl<T, S, H> HandlerWrapper<T, S, H>
where
    H: Handler<T, S> + 'static,
    T: 'static,
{
    pub fn new(handler: H) -> Self {
        Self {
            handler,
            _marker: std::marker::PhantomData,
        }
    }
}

impl<T, S, H> Clone for HandlerWrapper<T, S, H>
where
    H: Handler<T, S> + Clone,
{
    fn clone(&self) -> Self {
        Self {
            handler: self.handler.clone(),
            _marker: std::marker::PhantomData,
        }
    }
}

pub trait Handler<T, S>: Clone + Send + Sync + 'static {
    /// The type of future calling this handler returns.
    type Future: Future<Output = Request> + Send + 'static;

    /// Call the handler with the given request.
    fn call(self, req: Request, state: S) -> Self::Future;
}

impl<S, F, Fut, Res> Handler<(), S> for F
where
    F: FnOnce() -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = Res> + Send,
    Res: IntoResponse + 'static,
{
    type Future = Pin<Box<dyn Future<Output = Request> + Send>>;

    fn call(self, mut req: Request, _state: S) -> Self::Future {
        Box::pin(async move {
            let result = self().await.into_response();
            result.fill_response(&mut req);
            req
        })
    }
}

macro_rules! impl_handler {
    (
        [$($ty:ident),*], $last:ident
    ) => {
        #[allow(non_snake_case, unused_mut)]
        impl<$($ty,)* $last, S, F, Fut, Res> Handler<($($ty,)* $last,), S> for F
        where
            S: Send + Sync + 'static,
            F: FnOnce($($ty,)* $last,) -> Fut + Clone + Send + Sync + 'static,
            Fut: Future<Output = Res> + Send,
            Res: IntoResponse,
            $( $ty: FromRequest<S> + Send, )*
            $last: FromRequest<S> + Send,
        {
            type Future = Pin<Box<dyn Future<Output = Request> + Send>>;

            fn call(self, mut req: Request, state: S) -> Self::Future {
                Box::pin(async move {
                    $(
                        let $ty = match $ty::from_request(&req, &state).await {
                            Ok(value) => value,
                            Err(rejection) => {
                                rejection.into_response().fill_response(&mut req);
                                return req;
                            }
                        };
                    )*

                    let $last = match $last::from_request( &req, &state).await {
                        Ok(value) => value,
                        Err(rejection) => {
                            rejection.into_response().fill_response(&mut req);
                            return req;
                        }
                    };

                    let response = self($($ty,)* $last,).await;
                    response.into_response().fill_response(&mut req);
                    req
                })
            }
        }
    };
}

all_the_tuples!(impl_handler);
