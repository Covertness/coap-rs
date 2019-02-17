use bincode;
use std::collections::BTreeMap;
use std::collections::LinkedList;

use num_derive::FromPrimitive;
use num_traits::FromPrimitive;

use super::header;

macro_rules! u8_to_unsigned_be {
    ($src:ident, $start:expr, $end:expr, $t:ty) => ({
        (0 .. $end - $start + 1).rev().fold(0, |acc, i| acc | $src[$start+i] as $t << i * 8)
    })
}

#[derive(PartialEq, Eq, Debug)]
pub enum CoAPOption {
    IfMatch,
    UriHost,
    ETag,
    IfNoneMatch,
    Observe,
    UriPort,
    LocationPath,
    UriPath,
    ContentFormat,
    MaxAge,
    UriQuery,
    Accept,
    LocationQuery,
    Block2,
    Block1,
    ProxyUri,
    ProxyScheme,
    Size1,
    Size2,
    NoResponse,
}

#[derive(PartialEq, Eq, Debug, FromPrimitive)]
pub enum ContentFormat {
    TextPlain = 0,
    ApplicationLinkFormat = 40,
    ApplicationXML = 41,
    ApplicationOctetStream = 42,
    ApplicationEXI = 47,
    ApplicationJSON = 50,
    ApplicationCBOR = 60,
    ApplicationSenmlJSON = 110,
    ApplicationSensmlJSON = 111,
    ApplicationSenmlCBOR = 112,
    ApplicationSensmlCBOR = 113,
    ApplicationSenmlExi = 114,
    ApplicationSensmlExi = 115,
    ApplicationSenmlXML = 310,
    ApplicationSensmlXML = 311,
}

#[derive(PartialEq, Eq, Debug, FromPrimitive)]
pub enum ObserveOption {
    Register = 0,
    Deregister = 1,
}

#[derive(Debug)]
pub enum PackageError {
    InvalidHeader,
    InvalidPacketLength,
}

#[derive(Debug)]
pub enum ParseError {
    InvalidHeader,
    InvalidTokenLength,
    InvalidOptionDelta,
    InvalidOptionLength,
}

#[derive(Clone, Debug)]
pub struct Packet {
    pub header: header::Header,
    token: Vec<u8>,
    options: BTreeMap<usize, LinkedList<Vec<u8>>>,
    pub payload: Vec<u8>,
}

impl Packet {
    pub fn new() -> Packet {
        Packet {
            header: header::Header::new(),
            token: Vec::new(),
            options: BTreeMap::new(),
            payload: Vec::new(),
        }
    }

    pub fn set_token(&mut self, token: Vec<u8>) {
        self.header.set_token_length(token.len() as u8);
        self.token = token;
    }

    pub fn get_token(&self) -> &Vec<u8> {
        return &self.token;
    }

    pub fn set_option(&mut self, tp: CoAPOption, value: LinkedList<Vec<u8>>) {
        let num = Self::get_option_number(tp);
        self.options.insert(num, value);
    }

    pub fn set_content_format(&mut self, cf: ContentFormat) {
        let content_format = cf as u16;
        let msb = (content_format >> 8) as u8;
        let lsb = (content_format & 0xFF) as u8;

        let content_format: Vec<u8> = vec![msb, lsb];
        self.add_option(CoAPOption::ContentFormat, content_format);
    }

    pub fn set_payload(&mut self, payload: Vec<u8>) {
        self.payload = payload;
    }

    pub fn add_option(&mut self, tp: CoAPOption, value: Vec<u8>) {
        let num = Self::get_option_number(tp);
        match self.options.get_mut(&num) {
            Some(list) => {
                list.push_back(value);
                return;
            }
            None => (),
        };

        let mut list = LinkedList::new();
        list.push_back(value);
        self.options.insert(num, list);
    }

    pub fn get_option(&self, tp: CoAPOption) -> Option<&LinkedList<Vec<u8>>> {
        let num = Self::get_option_number(tp);
        self.options.get(&num)
    }

    pub fn clear_option(&mut self, tp: CoAPOption) {
        let num = Self::get_option_number(tp);
        if let Some(list) = self.options.get_mut(&num) {
            list.clear()
        }
    }

    pub fn get_content_format(&self) -> Option<ContentFormat> {
        if let Some(list) = self.get_option(CoAPOption::ContentFormat) {
            if let Some(vector) = list.front() {
                let msb = vector[0] as u16;
                let lsb = vector[1] as u16;
                let number = (msb << 8) + lsb;

                return ContentFormat::from_u16(number);
            }
        }

        None
    }

    pub fn set_observe(&mut self, value: Vec<u8>) {
        self.clear_option(CoAPOption::Observe);
        self.add_option(CoAPOption::Observe, value);
    }

    pub fn get_observe(&self) -> Option<&Vec<u8>> {
        if let Some(list) = self.get_option(CoAPOption::Observe) {
            if let Some(flag) = list.front() {
                return Some(flag);
            }
        }

        None
    }

    /// Decodes a byte slice and construct the equivalent Packet.
    pub fn from_bytes(buf: &[u8]) -> Result<Packet, ParseError> {
        
        let header_result: bincode::Result<header::HeaderRaw> = bincode::config().big_endian().deserialize(buf);
        match header_result {
            Ok(raw_header) => {
                let header = header::Header::from_raw(&raw_header);
                let token_length = header.get_token_length();
                let options_start: usize = 4 + token_length as usize;

                if token_length > 8 {
                    return Err(ParseError::InvalidTokenLength);
                }

                if options_start > buf.len() {
                    return Err(ParseError::InvalidTokenLength);
                }

                let token = buf[4..options_start].to_vec();

                let mut idx = options_start;
                let mut options_number = 0;
                let mut options: BTreeMap<usize, LinkedList<Vec<u8>>> = BTreeMap::new();
                while idx < buf.len() {
                    let byte = buf[idx];

                    if byte == 255 || idx > buf.len() {
                        break;
                    }

                    let mut delta = (byte >> 4) as usize;
                    let mut length = (byte & 0xF) as usize;

                    idx += 1;

                    // Check for special delta characters
                    match delta {
                        13 => {
                            if idx >= buf.len() {
                                return Err(ParseError::InvalidOptionLength);
                            }
                            delta = buf[idx] as usize + 13;
                            idx += 1;
                        }
                        14 => {
                            if idx + 1 >= buf.len() {
                                return Err(ParseError::InvalidOptionLength);
                            }

                            delta = (u16::from_be(u8_to_unsigned_be!(buf, idx, idx + 1, u16)) +
                                     269) as usize;
                            idx += 2;
                        }
                        15 => {
                            return Err(ParseError::InvalidOptionDelta);
                        }
                        _ => {}
                    };

                    // Check for special length characters
                    match length {
                        13 => {
                            if idx >= buf.len() {
                                return Err(ParseError::InvalidOptionLength);
                            }

                            length = buf[idx] as usize + 13;
                            idx += 1;
                        }
                        14 => {
                            if idx + 1 >= buf.len() {
                                return Err(ParseError::InvalidOptionLength);
                            }

                            length = (u16::from_be(u8_to_unsigned_be!(buf, idx, idx + 1, u16)) +
                                      269) as usize;
                            idx += 2;
                        }
                        15 => {
                            return Err(ParseError::InvalidOptionLength);
                        }
                        _ => {}
                    };

                    options_number += delta;

                    let end = idx + length;
                    if end > buf.len() {
                        return Err(ParseError::InvalidOptionLength);
                    }
                    let options_value = buf[idx..end].to_vec();

                    if options.contains_key(&options_number) {
                        let options_list = options.get_mut(&options_number).unwrap();
                        options_list.push_back(options_value);
                    } else {
                        let mut list = LinkedList::new();
                        list.push_back(options_value);
                        options.insert(options_number, list);
                    }

                    idx += length;
                }

                let mut payload = Vec::new();
                if idx < buf.len() {
                    payload = buf[(idx + 1)..buf.len()].to_vec();
                }


                Ok(Packet {
                    header: header,
                    token: token,
                    options: options,
                    payload: payload,
                })
            }
            Err(_) => Err(ParseError::InvalidHeader),
        }
    }

    /// Returns a vector of bytes representing the Packet.
    pub fn to_bytes(&self) -> Result<Vec<u8>, PackageError> {
        let mut options_delta_length = 0;
        let mut options_bytes: Vec<u8> = Vec::new();
        for (number, value_list) in self.options.iter() {
            for value in value_list.iter() {
                let mut header: Vec<u8> = Vec::with_capacity(1 + 2 + 2);
                let delta = number - options_delta_length;

                let mut byte: u8 = 0;
                if delta <= 12 {
                    byte |= (delta << 4) as u8;
                } else if delta < 269 {
                    byte |= 13 << 4;
                } else {
                    byte |= 14 << 4;
                }
                if value.len() <= 12 {
                    byte |= value.len() as u8;
                } else if value.len() < 269 {
                    byte |= 13;
                } else {
                    byte |= 14;
                }
                header.push(byte);

                if delta > 12 && delta < 269 {
                    header.push((delta - 13) as u8);
                } else if delta >= 269 {
                    let fix = (delta - 269) as u16;
                    header.push((fix >> 8) as u8);
                    header.push((fix & 0xFF) as u8);
                }

                if value.len() > 12 && value.len() < 269 {
                    header.push((value.len() - 13) as u8);
                } else if value.len() >= 269 {
                    let fix = (value.len() - 269) as u16;
                    header.push((fix >> 8) as u8);
                    header.push((fix & 0xFF) as u8);
                }

                options_delta_length += delta;

                options_bytes.reserve(header.len() + value.len());
                unsafe {
                    use std::ptr;
                    let buf_len = options_bytes.len();
                    ptr::copy(header.as_ptr(),
                              options_bytes.as_mut_ptr().offset(buf_len as isize),
                              header.len());
                    ptr::copy(value.as_ptr(),
                              options_bytes.as_mut_ptr().offset((buf_len + header.len()) as isize),
                              value.len());
                    options_bytes.set_len(buf_len + header.len() + value.len());
                }
            }
        }

        let mut buf_length = 4 + self.payload.len() + self.token.len();
        if self.header.code != header::MessageClass::Empty && self.payload.len() != 0 {
            buf_length += 1;
        }
        buf_length += options_bytes.len();

        if buf_length > 1280 {
            return Err(PackageError::InvalidPacketLength);
        }

        let mut buf: Vec<u8> = Vec::with_capacity(buf_length);
        let header_result: bincode::Result<()> =
            bincode::config().big_endian().serialize_into(&mut buf, &self.header.to_raw());


        match header_result {
            Ok(_) => {
                buf.reserve(self.token.len() + options_bytes.len());
                unsafe {
                    use std::ptr;
                    let buf_len = buf.len();
                    ptr::copy(self.token.as_ptr(),
                              buf.as_mut_ptr().offset(buf_len as isize),
                              self.token.len());
                    ptr::copy(options_bytes.as_ptr(),
                              buf.as_mut_ptr().offset((buf_len + self.token.len()) as isize),
                              options_bytes.len());
                    buf.set_len(buf_len + self.token.len() + options_bytes.len());
                }

                if self.header.code != header::MessageClass::Empty && self.payload.len() != 0 {
                    buf.push(0xFF);
                    buf.reserve(self.payload.len());
                    unsafe {
                        use std::ptr;
                        let buf_len = buf.len();
                        ptr::copy(self.payload.as_ptr(),
                                  buf.as_mut_ptr().offset(buf.len() as isize),
                                  self.payload.len());
                        buf.set_len(buf_len + self.payload.len());
                    }
                }
                Ok(buf)
            }
            Err(_) => Err(PackageError::InvalidHeader),
        }
    }

    fn get_option_number(tp: CoAPOption) -> usize {
        match tp {
            CoAPOption::IfMatch => 1,
            CoAPOption::UriHost => 3,
            CoAPOption::ETag => 4,
            CoAPOption::IfNoneMatch => 5,
            CoAPOption::Observe => 6,
            CoAPOption::UriPort => 7,
            CoAPOption::LocationPath => 8,
            CoAPOption::UriPath => 11,
            CoAPOption::ContentFormat => 12,
            CoAPOption::MaxAge => 14,
            CoAPOption::UriQuery => 15,
            CoAPOption::Accept => 17,
            CoAPOption::LocationQuery => 20,
            CoAPOption::Block2 => 23,
            CoAPOption::Block1 => 27,
            CoAPOption::ProxyUri => 35,
            CoAPOption::ProxyScheme => 39,
            CoAPOption::Size1 => 60,
            CoAPOption::Size2 => 28,
            CoAPOption::NoResponse => 258
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use super::super::header;
    use std::collections::LinkedList;
    use log::*;

    #[test]
    fn test_decode_packet_with_options() {
        let buf = [0x44, 0x01, 0x84, 0x9e, 0x51, 0x55, 0x77, 0xe8, 0xb2, 0x48, 0x69, 0x04, 0x54,
                   0x65, 0x73, 0x74, 0x43, 0x61, 0x3d, 0x31];
        let packet = Packet::from_bytes(&buf);
        assert!(packet.is_ok());
        let packet = packet.unwrap();
        assert_eq!(packet.header.get_version(), 1);
        assert_eq!(packet.header.get_type(), header::MessageType::Confirmable);
        assert_eq!(packet.header.get_token_length(), 4);
        assert_eq!(packet.header.code,
                   header::MessageClass::Request(header::RequestType::Get));
        assert_eq!(packet.header.get_message_id(), 33950);
        assert_eq!(*packet.get_token(), vec![0x51, 0x55, 0x77, 0xE8]);
        assert_eq!(packet.options.len(), 2);

        let uri_path = packet.get_option(CoAPOption::UriPath);
        assert!(uri_path.is_some());
        let uri_path = uri_path.unwrap();
        let mut expected_uri_path = LinkedList::new();
        expected_uri_path.push_back("Hi".as_bytes().to_vec());
        expected_uri_path.push_back("Test".as_bytes().to_vec());
        assert_eq!(*uri_path, expected_uri_path);

        let uri_query = packet.get_option(CoAPOption::UriQuery);
        assert!(uri_query.is_some());
        let uri_query = uri_query.unwrap();
        let mut expected_uri_query = LinkedList::new();
        expected_uri_query.push_back("a=1".as_bytes().to_vec());
        assert_eq!(*uri_query, expected_uri_query);
    }

    #[test]
    fn test_decode_packet_with_payload() {
        let buf = [0x64, 0x45, 0x13, 0xFD, 0xD0, 0xE2, 0x4D, 0xAC, 0xFF, 0x48, 0x65, 0x6C, 0x6C,
                   0x6F];
        let packet = Packet::from_bytes(&buf);
        assert!(packet.is_ok());
        let packet = packet.unwrap();
        assert_eq!(packet.header.get_version(), 1);
        assert_eq!(packet.header.get_type(),
                   header::MessageType::Acknowledgement);
        assert_eq!(packet.header.get_token_length(), 4);
        assert_eq!(packet.header.code,
                   header::MessageClass::Response(header::ResponseType::Content));
        warn!("{}", packet.header.get_message_id());
        assert_eq!(packet.header.get_message_id(), 5117);
        assert_eq!(*packet.get_token(), vec![0xD0, 0xE2, 0x4D, 0xAC]);
        assert_eq!(packet.payload, "Hello".as_bytes().to_vec());
    }

    #[test]
    fn test_encode_packet_with_options() {
        let mut packet = Packet::new();
        packet.header.set_version(1);
        packet.header.set_type(header::MessageType::Confirmable);
        packet.header.code = header::MessageClass::Request(header::RequestType::Get);
        packet.header.set_message_id(33950);
        packet.set_token(vec![0x51, 0x55, 0x77, 0xE8]);
        packet.add_option(CoAPOption::UriPath, b"Hi".to_vec());
        packet.add_option(CoAPOption::UriPath, b"Test".to_vec());
        packet.add_option(CoAPOption::UriQuery, b"a=1".to_vec());
        assert_eq!(packet.to_bytes().unwrap(),
                   vec![0x44, 0x01, 0x84, 0x9e, 0x51, 0x55, 0x77, 0xe8, 0xb2, 0x48, 0x69, 0x04,
                        0x54, 0x65, 0x73, 0x74, 0x43, 0x61, 0x3d, 0x31]);
    }

    #[test]
    fn test_encode_packet_with_payload() {
        let mut packet = Packet::new();
        packet.header.set_version(1);
        packet.header.set_type(header::MessageType::Acknowledgement);
        packet.header.code = header::MessageClass::Response(header::ResponseType::Content);
        packet.header.set_message_id(5117);
        packet.set_token(vec![0xD0, 0xE2, 0x4D, 0xAC]);
        packet.payload = "Hello".as_bytes().to_vec();
        assert_eq!(packet.to_bytes().unwrap(),
                   vec![0x64, 0x45, 0x13, 0xFD, 0xD0, 0xE2, 0x4D, 0xAC, 0xFF, 0x48, 0x65, 0x6C,
                        0x6C, 0x6F]);
    }

    #[test]
    fn test_encode_decode_content_format() {
        let mut packet = Packet::new();
        packet.set_content_format(ContentFormat::ApplicationJSON);
        assert_eq!(ContentFormat::ApplicationJSON, packet.get_content_format().unwrap())
    }

    #[test]
    fn test_decode_empty_content_format() {
        let packet = Packet::new();
        assert!(packet.get_content_format().is_none());
    }

    #[test]
    fn test_malicious_packet() {
        use rand;
        use quickcheck::{QuickCheck, StdGen, TestResult};

        fn run(x: Vec<u8>) -> TestResult {
            match Packet::from_bytes(&x[..]) {
                Ok(packet) => {
                    TestResult::from_bool(packet.get_token().len() ==
                                          packet.header.get_token_length() as usize)
                }
                Err(_) => TestResult::passed(),
            }
        }
        QuickCheck::new()
            .tests(10000)
            .gen(StdGen::new(rand::thread_rng(), 1500))
            .quickcheck(run as fn(Vec<u8>) -> TestResult)
    }
}
