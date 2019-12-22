pub mod header;
pub mod request;
pub mod response;
pub mod packet;

use std::collections::LinkedList;
use bytes::BytesMut;
use tokio::{io};

use tokio_util::codec::{Decoder, Encoder};

use self::packet::Packet;
use self::header::Header;

pub trait IsMessage {
    fn get_message(&self) -> &Packet;
    fn get_mut_message(&mut self) -> &mut Packet;
    fn get_header(&self) -> &Header;
    fn get_mut_header(&mut self) -> &mut Header;

    fn set_token(&mut self, token: Vec<u8>) {
        self.get_mut_message().set_token(token);
    }
    fn get_token(&self) -> &Vec<u8> {
        return self.get_message().get_token();
    }
    fn set_option(&mut self, tp: packet::CoAPOption, value: LinkedList<Vec<u8>>) {
        self.get_mut_message().set_option(tp, value);
    }
    fn set_payload(&mut self, payload: Vec<u8>) {
        self.get_mut_message().set_payload(payload);
    }
    fn add_option(&mut self, tp: packet::CoAPOption, value: Vec<u8>) {
        self.get_mut_message().add_option(tp, value);
    }
    fn get_option(&self, tp: packet::CoAPOption) -> Option<&LinkedList<Vec<u8>>> {
        return self.get_message().get_option(tp);
    }
    fn clear_option(&mut self, tp: packet::CoAPOption) {
        self.get_mut_message().clear_option(tp);
    }
    fn set_observe(&mut self, value: Vec<u8>) {
        self.get_mut_message().set_observe(value);
    }
    fn get_observe(&self) -> Option<&Vec<u8>> {
        return self.get_message().get_observe();
    }

    fn get_message_id(&self) -> u16 {
        return self.get_message().header.get_message_id();
    }
    fn set_message_id(&mut self, message_id: u16) {
        self.get_mut_message().header.set_message_id(message_id);
    }
    fn set_version(&mut self, v: u8) {
        self.get_mut_message().header.set_version(v);
    }
    fn get_version(&self) -> u8 {
        return self.get_message().header.get_version();
    }
    fn set_type(&mut self, t: header::MessageType) {
        self.get_mut_message().header.set_type(t);
    }
    fn get_type(&self) -> header::MessageType {
        return self.get_message().header.get_type();
    }
    fn get_code(&self) -> String {
        return self.get_message().header.get_code();
    }
    fn set_code(&mut self, code: &str) {
        self.get_mut_message().header.set_code(code);
    }
}

pub struct Codec {}

impl Codec {
    pub fn new() -> Codec {
        Codec{}
    }
}

impl Decoder for Codec {
    type Item = Packet;
    type Error = io::Error;

    fn decode(&mut self, buf: &mut BytesMut) -> Result<Option<Packet>, io::Error> {
        Ok(Some(Packet::from_bytes(buf)
        .map_err(|cause| io::Error::new(io::ErrorKind::InvalidData, cause.to_string()))?))
    }
}

impl Encoder for Codec {
    type Item = Packet;
    type Error = io::Error;

    fn encode(&mut self, my_packet: Packet, buf: &mut BytesMut) -> Result<(), io::Error> {
        buf.extend_from_slice(&my_packet.to_bytes()
        .map_err(|cause| io::Error::new(io::ErrorKind::InvalidData, cause.to_string()))?[..]);
        Ok(())
    }
}
