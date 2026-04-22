//! Route matching and error types for the CoAP router.

use crate::router::{
    request::Request,
    response::{IntoResponse, Response, Status},
    util::PercentDecodedStr,
};
use coap_lite::RequestType as Method;
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
    /// The requested route was not found.
    NotFound,
}

impl std::fmt::Display for RouteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RouteError::DifferentMethod => write!(f, "Different method"),
            RouteError::NoUriPath => write!(f, "No URI path"),
            RouteError::DifferentPath => write!(f, "Different path"),
            RouteError::NotFound => write!(f, "Not found"),
        }
    }
}

impl IntoResponse for RouteError {
    fn into_response(self) -> Response {
        let error_message = self.to_string();
        Response::new()
            .set_response_type(Status::BadRequest)
            .set_payload(error_message.into_bytes())
    }
}

/// A route definition, including the HTTP method, the route pattern, and the compiled regex for matching.
#[derive(Debug, Clone)]
pub struct Route {
    /// The HTTP method (e.g. GET, POST) that this route matches.
    method: Method,
    /// The original route pattern string (e.g. `"/sensors/{id}"`).
    route: String,
    /// The compiled regex pattern for matching request paths against this route.
    regex: Regex,
    /// The names of the path parameters in the order they appear in the route pattern.
    param_names: Vec<String>,
}

impl Route {
    /// Creates a new `Route` from the given method and route pattern string.
    pub fn new<S: ToString>(method: Method, route: S) -> Self {
        let route = route.to_string();
        let (regex, param_names) = Self::build_regex(&route);

        Route {
            method,
            route,
            regex,
            param_names,
        }
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
            let chars: Vec<char> = segment.chars().collect();
            for i in 0..chars.len() {
                match chars[i] {
                    '{' => {
                        if in_param {
                            // Nested parameter start, which is invalid
                            panic!("Nested parameters are not allowed in route pattern: {route}");
                        }

                        // Start of parameter
                        in_param = true;
                        param_start = i;

                        // Add preceding literal text with proper escaping
                        if i > pos {
                            pattern.push_str(&regex::escape(&segment[pos..i]));
                        }
                    }
                    '}' => {
                        if !in_param {
                            // Unmatched closing brace, which is invalid
                            panic!("Unmatched closing brace in route pattern: {route}",);
                        }
                        // End of parameter
                        in_param = false;

                        // Extract parameter name
                        let param_name = &segment[param_start + 1..i];
                        param_names.push(param_name.to_string());
                        // Add capture group to pattern
                        pattern.push_str("([^/]+)");

                        // Update position
                        pos = i + 1;
                    }
                    _ => {} // Skip other characters
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

        println!("Regex pattern: {}", pattern);
        println!("Parameter names: {:?}", param_names);

        // Compile the regex pattern
        let regex = match Regex::new(&pattern) {
            Ok(re) => re,
            Err(err) => panic!("Invalid route pattern: {} ({})", route, err),
        };

        (regex, param_names)
    }

    /// Returns the pattern string for this route.
    #[inline]
    pub fn route(&self) -> &str {
        &self.route
    }

    /// Checks if the given method matches this route's method.
    #[inline]
    pub fn match_method(&self, method: Method) -> Result<(), RouteError> {
        match self.method == method {
            true => Ok(()),
            false => Err(RouteError::DifferentMethod),
        }
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
                        return Err(RouteError::NoUriPath);
                    }
                }
            }
            Ok(params)
        } else {
            Err(RouteError::DifferentPath)
        }
    }

    /// Checks if the given request matches this route's method and path pattern.
    ///
    /// Returns a vector of (parameter name, parameter value) pairs if the request matches, or a `RouteError` if it does not.
    pub(crate) fn match_request(
        &self,
        req: &mut Request,
    ) -> Result<Vec<(Arc<str>, PercentDecodedStr)>, RouteError> {
        self.match_method(req.method())?;
        self.match_path(&req.path())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[should_panic(
        expected = "Nested parameters are not allowed in route pattern: /sensors/{id{nested}}"
    )]
    fn test_nested_parameters() {
        let _route = Route::new(Method::Get, "/sensors/{id{nested}}");
    }

    #[test]
    #[should_panic(expected = "Unmatched closing brace in route pattern: /sensors/{id}}")]
    fn test_unmatched_closing_brace() {
        let _route = Route::new(Method::Get, "/sensors/{id}}");
    }

    #[test]
    #[should_panic(expected = "Unclosed parameter in route pattern: /sensors/{id")]
    fn test_unclosed_parameter() {
        let _route = Route::new(Method::Get, "/sensors/{id");
    }

    #[test]
    fn test_build_regex() {
        let route = Route::new(Method::Get, "/sensors/{id}/readings/{type}");
        assert_eq!(route.param_names, vec!["id", "type"]);
        assert!(route.regex.is_match("sensors/123/readings/temperature"));
        assert!(route.regex.is_match("sensors/abc/readings/humidity"));
        assert!(!route.regex.is_match("sensors/123/readings"));
        assert!(!route
            .regex
            .is_match("sensors/123/readings/temperature/extra"));
    }

    #[test]
    fn test_match_method() {
        let route = Route::new(Method::Get, "/sensors/{id}");
        assert!(route.match_method(Method::Get).is_ok());
        assert!(route.match_method(Method::Post).is_err());
    }

    #[test]
    fn test_match_path() {
        let route = Route::new(Method::Get, "/sensors/{id}/readings/{type}");
        let params = route
            .match_path("sensors/123/readings/temperature")
            .expect("Path should match");
        assert_eq!(params.len(), 2);
        assert_eq!(params[0].0.as_ref(), "id");
        assert_eq!(params[0].1.as_str(), "123");
        assert_eq!(params[1].0.as_ref(), "type");
        assert_eq!(params[1].1.as_str(), "temperature");
    }
}
