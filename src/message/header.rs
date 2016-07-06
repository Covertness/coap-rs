#[derive(Default, Debug, RustcEncodable, RustcDecodable)]
pub struct HeaderRaw {
    ver_type_tkl: u8,
    code: u8,
    message_id: u16,
}

#[derive(Debug)]
pub struct Header {
    ver_type_tkl: u8,
    pub code: MessageClass,
    message_id: u16,
}

#[derive(Debug, PartialEq)]
pub enum MessageClass {
    Empty,
    RequestType(Requests),
    ResponseType(Responses),
    Reserved,
}

#[derive(Debug, PartialEq)]
pub enum Requests {
    Get,
    Post,
    Put,
    Delete,
}

#[derive(Debug, PartialEq)]
pub enum Responses {
    // 200 Codes
    Created,
    Deleted,
    Valid,
    Changed,
    Content,

    // 400 Codes
    BadRequest,
    Unauthorized,
    BadOption,
    Forbidden,
    NotFound,
    MethodNotAllowed,
    NotAcceptable,
    PreconditionFailed,
    RequestEntityTooLarge,
    UnsupportedContentFormat,

    // 500 Codes
    InternalServerError,
    NotImplemented,
    BadGateway,
    ServiceUnavailable,
    GatewayTimeout,
    ProxyingNotSupported,
}

#[derive(PartialEq, Eq, Debug)]
pub enum MessageType {
    Confirmable,
    NonConfirmable,
    Acknowledgement,
    Reset,
    Invalid,
}

impl Header {
    pub fn new() -> Header {
        return Header::from_raw(&HeaderRaw::default());
    }

    pub fn from_raw(raw: &HeaderRaw) -> Header {
        return Header {
            ver_type_tkl: raw.ver_type_tkl,
            code: code_to_class(&raw.code),
            message_id: raw.message_id,
        };
    }

    pub fn to_raw(&self) -> HeaderRaw {
        return HeaderRaw {
            ver_type_tkl: self.ver_type_tkl,
            code: class_to_code(&self.code),
            message_id: self.message_id,
        };
    }

    #[inline]
    pub fn set_version(&mut self, v: u8) {
        let type_tkl = 0x3F & self.ver_type_tkl;
        self.ver_type_tkl = v << 6 | type_tkl;
    }

    #[inline]
    pub fn get_version(&self) -> u8 {
        return self.ver_type_tkl >> 6;
    }

    #[inline]
    pub fn set_type(&mut self, t: MessageType) {
        let tn = match t {
            MessageType::Confirmable => 0,
            MessageType::NonConfirmable => 1,
            MessageType::Acknowledgement => 2,
            MessageType::Reset => 3,
            _ => unreachable!(),
        };

        let ver_tkl = 0xCF & self.ver_type_tkl;
        self.ver_type_tkl = tn << 4 | ver_tkl;
    }

    #[inline]
    pub fn get_type(&self) -> MessageType {
        let tn = (0x30 & self.ver_type_tkl) >> 4;
        match tn {
            0 => MessageType::Confirmable,
            1 => MessageType::NonConfirmable,
            2 => MessageType::Acknowledgement,
            3 => MessageType::Reset,
            _ => MessageType::Invalid,
        }
    }

    #[inline]
    pub fn set_token_length(&mut self, tkl: u8) {
        assert_eq!(0xF0 & tkl, 0);

        let ver_type = 0xF0 & self.ver_type_tkl;
        self.ver_type_tkl = tkl | ver_type;
    }

    #[inline]
    pub fn get_token_length(&self) -> u8 {
        return 0x0F & self.ver_type_tkl;
    }

    pub fn set_code(&mut self, code: &str) {
        let code_vec: Vec<&str> = code.split('.').collect();
        assert_eq!(code_vec.len(), 2);

        let class_code = code_vec[0].parse::<u8>().unwrap();
        let detail_code = code_vec[1].parse::<u8>().unwrap();
        assert_eq!(0xF8 & class_code, 0);
        assert_eq!(0xE0 & detail_code, 0);

        self.code = code_to_class(&(class_code << 5 | detail_code));
    }

    pub fn get_code(&self) -> String {
        class_to_str(&self.code)
    }

    #[inline]
    pub fn set_message_id(&mut self, message_id: u16) {
        self.message_id = message_id;
    }

    #[inline]
    pub fn get_message_id(&self) -> u16 {
        return self.message_id;
    }
}

pub fn class_to_code(class: &MessageClass) -> u8 {
    return match *class {
        MessageClass::Empty => 0x00,

        MessageClass::RequestType(Requests::Get) => 0x01,
        MessageClass::RequestType(Requests::Post) => 0x02,
        MessageClass::RequestType(Requests::Put) => 0x03,
        MessageClass::RequestType(Requests::Delete) => 0x04,

        MessageClass::ResponseType(Responses::Created) => 0x41,
        MessageClass::ResponseType(Responses::Deleted) => 0x42,
        MessageClass::ResponseType(Responses::Valid) => 0x43,
        MessageClass::ResponseType(Responses::Changed) => 0x44,
        MessageClass::ResponseType(Responses::Content) => 0x45,

        MessageClass::ResponseType(Responses::BadRequest) => 0x80,
        MessageClass::ResponseType(Responses::Unauthorized) => 0x81,
        MessageClass::ResponseType(Responses::BadOption) => 0x82,
        MessageClass::ResponseType(Responses::Forbidden) => 0x83,
        MessageClass::ResponseType(Responses::NotFound) => 0x84,
        MessageClass::ResponseType(Responses::MethodNotAllowed) => 0x85,
        MessageClass::ResponseType(Responses::NotAcceptable) => 0x86,
        MessageClass::ResponseType(Responses::PreconditionFailed) => 0x8C,
        MessageClass::ResponseType(Responses::RequestEntityTooLarge) => 0x8D,
        MessageClass::ResponseType(Responses::UnsupportedContentFormat) => 0x8F,

        MessageClass::ResponseType(Responses::InternalServerError) => 0x90,
        MessageClass::ResponseType(Responses::NotImplemented) => 0x91,
        MessageClass::ResponseType(Responses::BadGateway) => 0x92,
        MessageClass::ResponseType(Responses::ServiceUnavailable) => 0x93,
        MessageClass::ResponseType(Responses::GatewayTimeout) => 0x94,
        MessageClass::ResponseType(Responses::ProxyingNotSupported) => 0x95,

        _ => 0xFF,
    } as u8;
}

pub fn code_to_class(code: &u8) -> MessageClass {
    match *code {
        0x00 => MessageClass::Empty,

        0x01 => MessageClass::RequestType(Requests::Get),
        0x02 => MessageClass::RequestType(Requests::Post),
        0x03 => MessageClass::RequestType(Requests::Put),
        0x04 => MessageClass::RequestType(Requests::Delete),

        0x41 => MessageClass::ResponseType(Responses::Created),
        0x42 => MessageClass::ResponseType(Responses::Deleted),
        0x43 => MessageClass::ResponseType(Responses::Valid),
        0x44 => MessageClass::ResponseType(Responses::Changed),
        0x45 => MessageClass::ResponseType(Responses::Content),

        0x80 => MessageClass::ResponseType(Responses::BadRequest),
        0x81 => MessageClass::ResponseType(Responses::Unauthorized),
        0x82 => MessageClass::ResponseType(Responses::BadOption),
        0x83 => MessageClass::ResponseType(Responses::Forbidden),
        0x84 => MessageClass::ResponseType(Responses::NotFound),
        0x85 => MessageClass::ResponseType(Responses::MethodNotAllowed),
        0x86 => MessageClass::ResponseType(Responses::NotAcceptable),
        0x8C => MessageClass::ResponseType(Responses::PreconditionFailed),
        0x8D => MessageClass::ResponseType(Responses::RequestEntityTooLarge),
        0x8F => MessageClass::ResponseType(Responses::UnsupportedContentFormat),

        0x90 => MessageClass::ResponseType(Responses::InternalServerError),
        0x91 => MessageClass::ResponseType(Responses::NotImplemented),
        0x92 => MessageClass::ResponseType(Responses::BadGateway),
        0x93 => MessageClass::ResponseType(Responses::ServiceUnavailable),
        0x94 => MessageClass::ResponseType(Responses::GatewayTimeout),
        0x95 => MessageClass::ResponseType(Responses::ProxyingNotSupported),

        _ => MessageClass::Reserved,
    }
}

pub fn code_to_str(code: &u8) -> String {
    let class_code = (0xE0 & code) >> 5;
    let detail_code = 0x1F & code;

    return format!("{}.{:02}", class_code, detail_code);
}

pub fn class_to_str(class: &MessageClass) -> String {
    return code_to_str(&class_to_code(class));
}

#[cfg(test)]
mod test {
    use message::header::*;

    #[test]
    fn test_header_codes() {
        for code in 0..255 {
            let class = code_to_class(&code);
            let code_str = code_to_str(&code);
            let class_str = class_to_str(&class);

            // Reserved class could technically be many codes
            //   so only check valid items
            if class != MessageClass::Reserved {
                assert_eq!(class_to_code(&class), code);
                assert_eq!(code_str, class_str);
            }
        }
    }
}
