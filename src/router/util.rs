//! Utility types and functions for the router module.

use std::{ops::Deref, sync::Arc};

/// A wrapper around a percent-decoded string.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) struct PercentDecodedStr(Arc<str>);

impl PercentDecodedStr {
    /// Creates a new `PercentDecodedStr` from a percent-encoded string.
    /// Returns `None` if the input string is not valid UTF-8 after decoding.
    ///
    /// Usually, this is used to decode URI path segments or query parameters.
    pub(crate) fn new<S: AsRef<str>>(s: S) -> Option<Self> {
        percent_encoding::percent_decode(s.as_ref().as_bytes())
            .decode_utf8()
            .ok()
            .map(|decoded| Self(decoded.as_ref().into()))
    }

    /// Returns the inner string as a `&str`.
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl Deref for PercentDecodedStr {
    type Target = str;

    #[inline]
    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}
