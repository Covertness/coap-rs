use bytes::BytesMut;
use tokio::{io};

use tokio_util::codec::{Decoder, Encoder};

use coap_lite::{Packet};

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
