use message::IsMessage;
use message::response::CoAPResponse;
use message::packet::{CoAPOption, Packet};
use message::header::{Header, MessageClass};
use std::net::SocketAddr;
use std::str;

pub use message::header::RequestType as Method;

#[derive(Debug)]
pub struct CoAPRequest {
    pub message: Packet,
    pub response: Option<CoAPResponse>,
    pub source: Option<SocketAddr>,
}

impl CoAPRequest {
    pub fn new() -> CoAPRequest {
        CoAPRequest {
            response: None,
            message: Packet::new(),
            source: None,
        }
    }

    pub fn from_packet(packet: Packet, source: &SocketAddr) -> CoAPRequest {
        CoAPRequest {
            response: CoAPResponse::new(&packet),
            message: packet,
            source: Some(source.clone()),
        }
    }

    pub fn set_method(&mut self, method: Method) {
        self.message.header.code = MessageClass::Request(method);
    }

    pub fn get_method(&self) -> &Method {
        match self.message.header.code {
            MessageClass::Request(Method::Get) => &Method::Get,
            MessageClass::Request(Method::Post) => &Method::Post,
            MessageClass::Request(Method::Put) => &Method::Put,
            MessageClass::Request(Method::Delete) => &Method::Delete,
            _ => &Method::UnKnown,
        }
    }

    pub fn set_path(&mut self, path: &str) {
        self.clear_option(CoAPOption::UriPath);

        let segs = path.split("/");
        for s in segs {
            if s.len() == 0 {
                continue;
            }

            self.add_option(CoAPOption::UriPath, s.as_bytes().to_vec());
        }
    }

    pub fn get_path(&self) -> String {
        match self.get_option(CoAPOption::UriPath) {
            Some(options) => {
                let mut vec = Vec::new();
                for option in options.iter() {
                    if let Ok(seg) = str::from_utf8(option) {
                        vec.push(seg);
                    }
                }
                vec.join("/")
            }
            _ => "".to_string(),
        }
    }
}

impl IsMessage for CoAPRequest {
    fn get_message(&self) -> &Packet {
        &self.message
    }
    fn get_mut_message(&mut self) -> &mut Packet {
        &mut self.message
    }
    fn get_header(&self) -> &Header {
        &self.message.header
    }
    fn get_mut_header(&mut self) -> &mut Header {
        &mut self.message.header
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use message::packet::{CoAPOption, Packet};
    use message::header::MessageType;
    use message::IsMessage;
    use std::net::SocketAddr;
    use std::str::FromStr;

    #[test]
    fn test_request_create() {

        let mut packet = Packet::new();
        let mut request1 = CoAPRequest::new();

        packet.set_token(vec![0x17, 0x38]);
        request1.set_token(vec![0x17, 0x38]);

        packet.add_option(CoAPOption::UriPath, b"test-interface".to_vec());
        request1.add_option(CoAPOption::UriPath, b"test-interface".to_vec());

        packet.header.set_message_id(42);
        request1.set_message_id(42);

        packet.header.set_version(2);
        request1.set_version(2);

        packet.header.set_type(MessageType::Confirmable);
        request1.set_type(MessageType::Confirmable);

        packet.header.set_code("0.04");
        request1.set_code("0.04");

        let request2 =
            CoAPRequest::from_packet(packet, &SocketAddr::from_str("127.0.0.1:1234").unwrap());

        assert_eq!(
            request1.message.to_bytes().unwrap(),
            request2.message.to_bytes().unwrap()
        );
    }

    #[test]
    fn test_method() {
        let mut request = CoAPRequest::new();

        request.set_code("0.01");
        assert_eq!(&Method::Get, request.get_method());

        request.set_code("0.02");
        assert_eq!(&Method::Post, request.get_method());

        request.set_code("0.03");
        assert_eq!(&Method::Put, request.get_method());

        request.set_code("0.04");
        assert_eq!(&Method::Delete, request.get_method());

        request.set_method(Method::Get);
        assert_eq!("0.01", request.get_code());

        request.set_method(Method::Post);
        assert_eq!("0.02", request.get_code());

        request.set_method(Method::Put);
        assert_eq!("0.03", request.get_code());

        request.set_method(Method::Delete);
        assert_eq!("0.04", request.get_code());
    }

    #[test]
    fn test_path() {
        let mut request = CoAPRequest::new();

        let path = "test-interface";
        request.add_option(CoAPOption::UriPath, path.as_bytes().to_vec());
        assert_eq!(path, request.get_path());

        let path2 = "test-interface2";
        request.set_path(path2);
        assert_eq!(
            path2.as_bytes().to_vec(),
            *request
                .get_option(CoAPOption::UriPath)
                .unwrap()
                .front()
                .unwrap()
        );

        request.set_path("/test-interface2");
        assert_eq!(
            path2.as_bytes().to_vec(),
            *request
                .get_option(CoAPOption::UriPath)
                .unwrap()
                .front()
                .unwrap()
        );
    }
}
