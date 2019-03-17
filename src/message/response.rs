use super::IsMessage;
use super::packet::Packet;
use super::header::{Header, MessageClass, MessageType};

pub use super::header::ResponseType as Status;

#[derive(Clone, Debug)]
pub struct CoAPResponse {
    pub message: Packet,
}

impl CoAPResponse {
    pub fn new(request: &Packet) -> Option<CoAPResponse> {
        let mut packet = Packet::new();

        packet.header.set_version(1);
        let response_type = match request.header.get_type() {
            MessageType::Confirmable => MessageType::Acknowledgement,
            MessageType::NonConfirmable => MessageType::NonConfirmable,
            _ => return None,
        };
        packet.header.set_type(response_type);
        packet.header.code = MessageClass::Response(Status::Content);
        packet
            .header
            .set_message_id(request.header.get_message_id());
        packet.set_token(request.get_token().clone());

        packet.payload = request.payload.clone();

        Some(CoAPResponse { message: packet })
    }

    pub fn set_status(&mut self, status: Status) {
        self.message.header.code = MessageClass::Response(status);
    }

    pub fn get_status(&self) -> &Status {
        if let MessageClass::Response(ref status) = &self.message.header.code {
            return status;
        }

        &Status::UnKnown
    }
}

impl IsMessage for CoAPResponse {
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
    use super::super::packet::Packet;
    use super::super::header::MessageType;

    #[test]
    fn test_new_response_valid() {
        for mtyp in vec![MessageType::Confirmable, MessageType::NonConfirmable] {
            let mut packet = Packet::new();
            packet.header.set_type(mtyp);
            let opt_resp = CoAPResponse::new(&packet);
            assert!(opt_resp.is_some());

            let response = opt_resp.unwrap();
            assert_eq!(packet.payload, response.message.payload);
        }
    }

    #[test]
    fn test_new_response_invalid() {
        let mut packet = Packet::new();
        packet.header.set_type(MessageType::Acknowledgement);
        assert!(CoAPResponse::new(&packet).is_none());
    }
}
