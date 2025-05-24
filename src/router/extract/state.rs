use crate::router::{
    extract::{FromRef, FromRequest},
    request::Request,
};
use std::{
    convert::Infallible,
    ops::{Deref, DerefMut},
};

#[derive(Debug, Default, Clone, Copy)]
pub struct State<S>(pub S);

impl<OuterState, InnerState> FromRequest<OuterState> for State<InnerState>
where
    InnerState: FromRef<OuterState>,
    OuterState: Send + Sync,
{
    type Rejection = Infallible;

    async fn from_request(_req: &Request, state: &OuterState) -> Result<Self, Self::Rejection> {
        let inner_state = InnerState::from_ref(state);
        Ok(State(inner_state))
    }
}

impl<S> Deref for State<S> {
    type Target = S;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<S> DerefMut for State<S> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}
