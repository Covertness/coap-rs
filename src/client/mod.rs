use std::io::{Error, ErrorKind, Result};
use std::time::Duration;

use url::Url;
use regex::Regex;


mod blocking;
pub use blocking::CoAPClient;

mod nonblocking;
pub use nonblocking::{CoAPClientAsync, CoAPObserverAsync};


/// RequestOptions for configuring CoAP client requests
#[derive(Debug, Clone, PartialEq)]
#[cfg_attr(feature = "structopt", derive(structopt::StructOpt))]
pub struct RequestOptions {
    #[cfg_attr(feature = "structopt", structopt(long, default_value = "3"))]
    /// Number of request retries
    pub retries: usize,

    #[cfg_attr(feature = "structopt", structopt(long, default_value = "3s", parse(try_from_str = humantime::parse_duration)))]
    /// Timeout for request retries (in ms)
    pub timeout: Duration,
}

impl Default for RequestOptions {
    fn default() -> Self {
        Self {
            retries: 3,
            timeout: Duration::from_secs(2),
        }
    }
}


pub fn parse_coap_url(url: &str) -> Result<(String, String, u16, String)> {
    let url_params = match Url::parse(url) {
        Ok(url_params) => url_params,
        Err(_) => return Err(Error::new(ErrorKind::InvalidInput, "url error")),
    };

    let scheme = url_params.scheme().to_string();

    let host = match url_params.host_str() {
        Some("") => return Err(Error::new(ErrorKind::InvalidInput, "host error")),
        Some(h) => h,
        None => return Err(Error::new(ErrorKind::InvalidInput, "host error")),
    };
    let host = Regex::new(r"^\[(.*?)]$").unwrap().replace(&host, "$1").to_string();

    let port = match url_params.port() {
        Some(p) => p,
        None => 5683,
    };

    let path = url_params.path().to_string();

    return Ok((scheme, host.to_string(), port, path));
}

#[cfg(test)]
mod test {
    use super::*;
    use super::super::*;
    use std::time::Duration;
    use std::io::ErrorKind;

    #[test]
    fn test_parse_coap_url_good_url() {
        assert!(parse_coap_url("coap://127.0.0.1").is_ok());
        assert!(parse_coap_url("coap://127.0.0.1:5683").is_ok());
        assert!(parse_coap_url("coap://[::1]").is_ok());
        assert!(parse_coap_url("coap://[::1]:5683").is_ok());
        assert!(parse_coap_url("coap://[bbbb::9329:f033:f558:7418]").is_ok());
        assert!(parse_coap_url("coap://[bbbb::9329:f033:f558:7418]:5683").is_ok());
    }


   #[test]
    fn test_parse_coap_url_bad_url() {
        assert!(parse_coap_url("coap://127.0.0.1:65536").is_err());
        assert!(parse_coap_url("coap://").is_err());
        assert!(parse_coap_url("coap://:5683").is_err());
        assert!(parse_coap_url("127.0.0.1").is_err());
    }

}