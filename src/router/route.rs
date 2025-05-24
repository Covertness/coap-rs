use crate::router::{
    request::Request,
    response::{IntoResponse, Response, Status},
    util::PercentDecodedStr,
};
use coap_lite::RequestType as Method;
use regex::Regex;
use std::{hash::Hash, sync::Arc};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RouteError {
    DifferentMethod,
    NoUriPath,
    DifferentPath,
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

#[derive(Debug, Clone)]
pub struct Route {
    method: Method,
    route: String,
    regex: Regex,
    param_names: Vec<String>,
}

impl Route {
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

    pub fn route(&self) -> &str {
        &self.route
    }

    #[inline]
    pub fn match_method(&self, method: Method) -> Result<(), RouteError> {
        match self.method == method {
            true => Ok(()),
            false => Err(RouteError::DifferentMethod),
        }
    }

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

    pub(crate) fn match_request(
        &self,
        req: &mut Request,
    ) -> Result<Vec<(Arc<str>, PercentDecodedStr)>, RouteError> {
        self.match_method(req.method())?;
        self.match_path(&req.path())
    }
}
