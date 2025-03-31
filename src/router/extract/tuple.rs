use crate::router::{
    extract::FromRequest,
    macros::all_the_tuples,
    request::Request,
    response::{IntoResponse, Response},
};
use std::convert::Infallible;

impl<S: Sync> FromRequest<S> for () {
    type Rejection = Infallible;

    async fn from_request(_req: &Request, _state: &S) -> Result<Self, Self::Rejection> {
        Ok::<Self, Self::Rejection>(())
    }
}

macro_rules! impl_from_request {
    (
        [$($ty:ident),*], $last:ident
    ) => {
        #[allow(non_snake_case, unused_mut, unused_variables)]
        impl<S, $($ty,)* $last> FromRequest<S> for ($($ty,)* $last,)
        where
            S: Send + Sync,
            $( $ty: FromRequest<S> + Send, )*
            $last: FromRequest<S> + Send,
        {
            type Rejection = Response;

            async fn from_request(req: &Request, state: &S) -> Result<Self, Self::Rejection> {
                $(
                    let $ty = $ty::from_request(req, state)
                        .await
                        .map_err(|err| err.into_response())?;
                )*
                let $last = $last::from_request(req, state)
                    .await
                    .map_err(|err| err.into_response())?;

                Ok(($($ty,)* $last,))
            }
        }
    };
}

all_the_tuples!(impl_from_request);
