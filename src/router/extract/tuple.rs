//! Helpers for extracting multiple values from a request using tuples.

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::router::response::StatusCode;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Default)]
    struct TestState {
        calls: AtomicUsize,
    }

    #[derive(Debug, Clone, Copy)]
    struct TestRejection(&'static str);

    impl IntoResponse for TestRejection {
        fn into_response(self) -> Response {
            Response::new()
                .set_status_code(StatusCode::BadRequest)
                .set_payload(self.0.as_bytes().to_vec())
        }
    }

    #[derive(Debug, PartialEq, Eq)]
    struct OkA;

    #[derive(Debug, PartialEq, Eq)]
    struct OkB;

    #[derive(Debug, PartialEq, Eq)]
    struct Fail;

    #[derive(Debug, PartialEq, Eq)]
    struct Never;

    impl FromRequest<TestState> for OkA {
        type Rejection = TestRejection;

        async fn from_request(_req: &Request, state: &TestState) -> Result<Self, Self::Rejection> {
            state.calls.fetch_add(1, Ordering::SeqCst);
            Ok(OkA)
        }
    }

    impl FromRequest<TestState> for OkB {
        type Rejection = TestRejection;

        async fn from_request(_req: &Request, state: &TestState) -> Result<Self, Self::Rejection> {
            state.calls.fetch_add(1, Ordering::SeqCst);
            Ok(OkB)
        }
    }

    impl FromRequest<TestState> for Fail {
        type Rejection = TestRejection;

        async fn from_request(_req: &Request, state: &TestState) -> Result<Self, Self::Rejection> {
            state.calls.fetch_add(1, Ordering::SeqCst);
            Err(TestRejection("forced failure"))
        }
    }

    impl FromRequest<TestState> for Never {
        type Rejection = TestRejection;

        async fn from_request(_req: &Request, state: &TestState) -> Result<Self, Self::Rejection> {
            state.calls.fetch_add(100, Ordering::SeqCst);
            Ok(Never)
        }
    }

    fn dummy_request() -> Request {
        Request::new(Box::new(coap_lite::CoapRequest::new()))
    }

    #[tokio::test]
    async fn unit_tuple_is_extractable() {
        let req = dummy_request();
        let state = TestState::default();

        <() as FromRequest<TestState>>::from_request(&req, &state)
            .await
            .unwrap();

        assert_eq!(state.calls.load(Ordering::SeqCst), 0);
    }

    #[tokio::test]
    async fn one_element_tuple_is_extractable() {
        let req = dummy_request();
        let state = TestState::default();

        let value = <(OkA,) as FromRequest<TestState>>::from_request(&req, &state)
            .await
            .unwrap();

        assert_eq!(value, (OkA,));
        assert_eq!(state.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn multi_element_tuple_is_extractable() {
        let req = dummy_request();
        let state = TestState::default();

        let value = <(OkA, OkB) as FromRequest<TestState>>::from_request(&req, &state)
            .await
            .unwrap();

        assert_eq!(value, (OkA, OkB));
        assert_eq!(state.calls.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn tuple_rejection_is_mapped_to_response() {
        let req = dummy_request();
        let state = TestState::default();

        let rejection = <(Fail,) as FromRequest<TestState>>::from_request(&req, &state)
            .await
            .unwrap_err();

        assert_eq!(rejection.status_code, Some(StatusCode::BadRequest));
        assert_eq!(rejection.payload, Some(b"forced failure".to_vec()));
        assert_eq!(state.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn tuple_stops_after_first_rejection() {
        let req = dummy_request();
        let state = TestState::default();

        let rejection = <(Fail, Never) as FromRequest<TestState>>::from_request(&req, &state)
            .await
            .unwrap_err();

        assert_eq!(rejection.status_code, Some(StatusCode::BadRequest));
        assert_eq!(rejection.payload, Some(b"forced failure".to_vec()));
        assert_eq!(state.calls.load(Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn from_request_for_never_is_executed() {
        let req = dummy_request();
        let state = TestState::default();

        let value = <(Never,) as FromRequest<TestState>>::from_request(&req, &state)
            .await
            .unwrap();

        assert_eq!(value, (Never,));
        assert_eq!(state.calls.load(Ordering::SeqCst), 100);
    }
}
