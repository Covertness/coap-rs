use message::IsMessage;
use message::packet::Packet;
use message::header::{Header, MessageClass, MessageType};

pub use message::header::ResponseType as Status;

#[derive(Debug)]
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
        match self.message.header.code {
            MessageClass::Response(Status::Created) => &Status::Created,
            MessageClass::Response(Status::Deleted) => &Status::Deleted,
            MessageClass::Response(Status::Valid) => &Status::Valid,
            MessageClass::Response(Status::Changed) => &Status::Changed,
            MessageClass::Response(Status::Content) => &Status::Content,

            MessageClass::Response(Status::BadRequest) => &Status::BadRequest,
            MessageClass::Response(Status::Unauthorized) => &Status::Unauthorized,
            MessageClass::Response(Status::BadOption) => &Status::BadOption,
            MessageClass::Response(Status::Forbidden) => &Status::Forbidden,
            MessageClass::Response(Status::NotFound) => &Status::NotFound,
            MessageClass::Response(Status::MethodNotAllowed) => &Status::MethodNotAllowed,
            MessageClass::Response(Status::NotAcceptable) => &Status::NotAcceptable,
            MessageClass::Response(Status::PreconditionFailed) => &Status::PreconditionFailed,
            MessageClass::Response(Status::RequestEntityTooLarge) => &Status::RequestEntityTooLarge,
            MessageClass::Response(Status::UnsupportedContentFormat) => &Status::UnsupportedContentFormat,

            MessageClass::Response(Status::InternalServerError) => &Status::InternalServerError,
            MessageClass::Response(Status::NotImplemented) => &Status::NotImplemented,
            MessageClass::Response(Status::BadGateway) => &Status::BadGateway,
            MessageClass::Response(Status::ServiceUnavailable) => &Status::ServiceUnavailable,
            MessageClass::Response(Status::GatewayTimeout) => &Status::GatewayTimeout,
            MessageClass::Response(Status::ProxyingNotSupported) => &Status::ProxyingNotSupported,
            _ => &Status::UnKnown,
        }
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
    use message::packet::Packet;
    use message::header::MessageType;

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
