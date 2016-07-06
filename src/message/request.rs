use message::IsMessage;
use message::response::CoAPResponse;
use message::packet::Packet;
use message::header::Header;
use std::net::SocketAddr;

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
    use message::packet::{Packet, CoAPOption};
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

        let request2 = CoAPRequest::from_packet(packet,
                                                &SocketAddr::from_str("127.0.0.1:1234").unwrap());

        assert_eq!(request1.message.to_bytes().unwrap(),
                   request2.message.to_bytes().unwrap());

    }
}
