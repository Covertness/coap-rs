//! Route matching and error types for the CoAP router.

use crate::router::{
    response::{IntoResponse, Response, StatusCode},
    util::PercentDecodedStr,
};
use regex::Regex;
use std::{hash::Hash, sync::Arc};

/// Route matching and error types.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RouteError {
    /// The request method does not match the route's method.
    DifferentMethod,
    /// The request does not have a URI path.
    NoUriPath,
    /// The request URI path does not match the route's path.
    DifferentPath,
    /// The server failed to decode the URI path parameters.
    FailedToDecodePath,
    /// The requested route was not found.
    NotFound,
}

impl std::fmt::Display for RouteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RouteError::DifferentMethod => write!(f, "Different method"),
            RouteError::NoUriPath => write!(f, "No URI path"),
            RouteError::DifferentPath => write!(f, "Different path"),
            RouteError::FailedToDecodePath => write!(f, "Failed to decode path"),
            RouteError::NotFound => write!(f, "Not found"),
        }
    }
}

impl IntoResponse for RouteError {
    fn into_response(self) -> Response {
        let error_message = self.to_string();
        let status_code = match self {
            RouteError::NotFound => StatusCode::NotFound,
            _ => StatusCode::BadRequest,
        };
        Response::new()
            .set_status_code(status_code)
            .set_payload(error_message.into_bytes())
    }
}

/// A route definition, including the HTTP method, the route pattern, and the compiled regex for matching.
#[derive(Debug, Clone)]
pub struct Route {
    /// The compiled regex pattern for matching request paths against this route.
    regex: Regex,
    /// The names of the path parameters in the order they appear in the route pattern.
    param_names: Vec<String>,
}

impl Route {
    /// Creates a new `Route` from the given route pattern string.
    pub fn new<S: ToString>(route: S) -> Self {
        let route = route.to_string();
        let (regex, param_names) = Self::build_regex(&route);

        Route { regex, param_names }
    }

    /// Build a regex pattern from a route string
    fn build_regex(route: &str) -> (Regex, Vec<String>) {
        let mut param_names = Vec::new();
        let mut pattern = String::new();

        // Add start anchor
        pattern.push('^');

        // Process each path segment. We ignore leading and trailing slashes
        let segments = route.trim_matches('/').split('/');

        for (i, segment) in segments.enumerate() {
            if i > 0 {
                pattern.push('/');
            }

            if segment.is_empty() {
                continue;
            }

            // Track position as we scan the segment
            let mut pos = 0;
            let mut in_param = false;
            let mut param_start = 0;

            // Process the segment character by character
            for (byte_idx, ch) in segment.char_indices() {
                match ch {
                    '{' => {
                        if in_param {
                            panic!("Nested parameters are not allowed in route pattern: {route}");
                        }

                        in_param = true;
                        param_start = byte_idx;

                        if byte_idx > pos {
                            pattern.push_str(&regex::escape(&segment[pos..byte_idx]));
                        }
                    }
                    '}' => {
                        if !in_param {
                            panic!("Unmatched closing brace in route pattern: {route}");
                        }

                        in_param = false;

                        let param_name = &segment[param_start + '{'.len_utf8()..byte_idx];
                        param_names.push(param_name.to_string());
                        pattern.push_str("([^/]+)");

                        pos = byte_idx + ch.len_utf8();
                    }
                    _ => {}
                }
            }
            // If we ended in an unclosed parameter, that's an error
            if in_param {
                panic!("Unclosed parameter in route pattern: {route}");
            }

            // Handle any trailing literal text
            if pos < segment.len() {
                pattern.push_str(&regex::escape(&segment[pos..]));
            }
        }

        // Add end anchor
        pattern.push('$');

        // Compile the regex pattern
        let regex = match Regex::new(&pattern) {
            Ok(re) => re,
            Err(err) => panic!("Invalid route pattern: {} ({})", route, err),
        };

        (regex, param_names)
    }

    /// Checks if the given path matches this route's path pattern and extracts any path parameters.
    ///
    /// Returns a vector of (parameter name, parameter value) pairs if the path matches, or a `RouteError` if it does not.
    #[inline]
    pub(crate) fn match_path(
        &self,
        path: &str,
    ) -> Result<Vec<(Arc<str>, PercentDecodedStr)>, RouteError> {
        let path = path.trim_matches('/');
        if let Some(captures) = self.regex.captures(path) {
            let mut params = Vec::new();
            for (i, name) in self.param_names.iter().enumerate() {
                if let Some(matched) = captures.get(i + 1) {
                    let value = matched.as_str();
                    if let Some(decoded) = PercentDecodedStr::new(value) {
                        params.push((name.clone().into(), decoded));
                    } else {
                        return Err(RouteError::FailedToDecodePath);
                    }
                }
            }
            Ok(params)
        } else {
            Err(RouteError::DifferentPath)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_into_response() {
        let error = RouteError::DifferentMethod;
        let response = error.into_response();
        assert_eq!(response.status_code, Some(StatusCode::BadRequest));
        assert_eq!(response.payload, Some(b"Different method".to_vec()));

        let error = RouteError::NoUriPath;
        let response = error.into_response();
        assert_eq!(response.status_code, Some(StatusCode::BadRequest));
        assert_eq!(response.payload, Some(b"No URI path".to_vec()));

        let error = RouteError::DifferentPath;
        let response = error.into_response();
        assert_eq!(response.status_code, Some(StatusCode::BadRequest));
        assert_eq!(response.payload, Some(b"Different path".to_vec()));

        let error = RouteError::NotFound;
        let response = error.into_response();
        assert_eq!(response.status_code, Some(StatusCode::NotFound));
        assert_eq!(response.payload, Some(b"Not found".to_vec()));
    }

    #[test]
    #[should_panic(
        expected = "Nested parameters are not allowed in route pattern: /sensors/{id{nested}}"
    )]
    fn test_nested_parameters() {
        let _route = Route::new("/sensors/{id{nested}}");
    }

    #[test]
    #[should_panic(expected = "Unmatched closing brace in route pattern: /sensors/{id}}")]
    fn test_unmatched_closing_brace() {
        let _route = Route::new("/sensors/{id}}");
    }

    #[test]
    #[should_panic(expected = "Unclosed parameter in route pattern: /sensors/{id")]
    fn test_unclosed_parameter() {
        let _route = Route::new("/sensors/{id");
    }

    #[test]
    fn test_build_regex() {
        let route = Route::new("/sensors/{id}/readings/{type}");
        assert_eq!(route.param_names, vec!["id", "type"]);
        assert!(route.regex.is_match("sensors/123/readings/temperature"));
        assert!(route.regex.is_match("sensors/abc/readings/humidity"));
        assert!(!route.regex.is_match("sensors/123/readings"));
        assert!(!route
            .regex
            .is_match("sensors/123/readings/temperature/extra"));
    }

    #[test]
    fn test_match_path() {
        let route = Route::new("/sensors/{id}/readings/{type}");
        let params = route
            .match_path("sensors/123/readings/temperature")
            .expect("Path should match");
        assert_eq!(params.len(), 2);
        assert_eq!(params[0].0.as_ref(), "id");
        assert_eq!(params[0].1.as_str(), "123");
        assert_eq!(params[1].0.as_ref(), "type");
        assert_eq!(params[1].1.as_str(), "temperature");
    }

    #[test]
    fn test_no_match_path() {
        let route = Route::new("/sensors/{id}/readings/{type}");
        assert!(route.match_path("sensors/123/readings").is_err());
        assert!(route
            .match_path("sensors/123/readings/temperature/extra")
            .is_err());
        assert!(route
            .match_path("devices/123/readings/temperature")
            .is_err());
    }

    #[test]
    fn test_preceding_literal_text() {
        let route = Route::new("/sensors/{id}abc");
        let params = route
            .match_path("sensors/123abc")
            .expect("Path should match");
        assert_eq!(params.len(), 1);
        assert_eq!(params[0].0.as_ref(), "id");
        assert_eq!(params[0].1.as_str(), "123");

        let route = Route::new("/sensors/abc{id}");
        let params = route
            .match_path("sensors/abc123")
            .expect("Path should match");
        assert_eq!(params.len(), 1);
        assert_eq!(params[0].0.as_ref(), "id");
        assert_eq!(params[0].1.as_str(), "123");
    }

    #[test]
    fn test_empty_segments() {
        let route = Route::new("/sensors/{id}//readings/{type}/");
        let params = route
            .match_path("/sensors/123//readings/temperature/")
            .expect("Path should match");
        assert_eq!(params.len(), 2);
        assert_eq!(params[0].0.as_ref(), "id");
        assert_eq!(params[0].1.as_str(), "123");
        assert_eq!(params[1].0.as_ref(), "type");
        assert_eq!(params[1].1.as_str(), "temperature");
    }
}
