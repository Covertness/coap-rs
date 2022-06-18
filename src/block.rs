use coap_lite::{
    CoapRequest, Packet, MessageClass,
    error::{InvalidBlockValue, IncompatibleOptionValueFormat},
    option_value::{OptionValueU16, OptionValueType},
};

#[derive(Ord, PartialOrd, Eq, PartialEq, Clone)]
pub struct RequestCacheKey<Endpoint: Ord + Clone> {
    /// Request type as an integer to make it easy to derive Ord.
    request_type_ord: u8,
    path: Vec<String>,
    requester: Option<Endpoint>,
}

impl<Endpoint: Ord + Clone> From<&CoapRequest<Endpoint>>
    for RequestCacheKey<Endpoint>
{
    fn from(request: &CoapRequest<Endpoint>) -> Self {
        Self {
            request_type_ord: u8::from(MessageClass::Request(
                *request.get_method(),
            )),
            path: request.get_path_as_vec().unwrap_or_default(),
            requester: request.source.clone(),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct BlockState {
    /// Last client request's block2 value (if any), which can either mean the
    /// client's attempt to suggest a block size or a request that came in
    /// after we expired our cache.
    pub last_request_block2: Option<BlockValue>,

    /// Packet we need to serve from if any future block-wise transfer requests
    /// come in.
    pub cached_response: Option<Packet>,

    /// Payload we are building up from a series of client requests.  Note that
    /// there is a deliberate lack of symmetry between the cached response and
    /// request due to the fact that the client is responsible for issuing
    /// multiple requests as we build up the cached payload.  This means that
    /// the client is ultimately responsible for making sure the last submitted
    /// packet is the one containing the interesting options we will need to
    /// handle the request and that we simply need to copy the payload into it.
    pub cached_request_payload: Option<Vec<u8>>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct BlockValue {
    pub num: u16,
    pub more: bool,
    pub size_exponent: u8,
}

impl BlockValue {
    pub fn new(
        num: usize,
        more: bool,
        size: usize,
    ) -> Result<Self, InvalidBlockValue> {
        let true_size_exponent = Self::largest_power_of_2_not_in_excess(size)
            .ok_or(InvalidBlockValue::SizeExponentEncodingError(size))?;

        let size_exponent = u8::try_from(true_size_exponent.saturating_sub(4))
            .map_err(InvalidBlockValue::TypeBoundsError)?;
        if size_exponent > 0x7 {
            return Err(InvalidBlockValue::SizeExponentEncodingError(size));
        }
        let num =
            u16::try_from(num).map_err(InvalidBlockValue::TypeBoundsError)?;
        Ok(Self {
            num,
            more,
            size_exponent,
        })
    }

    /// Finds the largest power of 2 that does not exceed `target`.
    fn largest_power_of_2_not_in_excess(target: usize) -> Option<usize> {
        if target == 0 {
            return None;
        }

        let max_power = usize::try_from(usize::BITS).unwrap();
        let power_in_excess = (0..max_power).find(|i| (1 << i) > target);

        match power_in_excess {
            Some(size) => Some(size - 1),
            None => Some(max_power),
        }
    }

    pub fn size(&self) -> usize {
        1 << (self.size_exponent + 4)
    }
}

impl From<BlockValue> for Vec<u8> {
    fn from(block_value: BlockValue) -> Vec<u8> {
        let scalar = block_value.num << 4
            | u16::from(block_value.more) << 3
            | u16::from(block_value.size_exponent & 0x7);
        Vec::from(OptionValueU16(scalar))
    }
}

impl TryFrom<Vec<u8>> for BlockValue {
    type Error = IncompatibleOptionValueFormat;

    fn try_from(value: Vec<u8>) -> Result<Self, Self::Error> {
        let scalar = OptionValueU16::try_from(value)?.0;

        let num: u16 = scalar >> 4;
        let more = scalar >> 3 & 0x1 == 0x1;
        let size_exponent: u8 = (scalar & 0x7) as u8;
        Ok(Self {
            num,
            more,
            size_exponent,
        })
    }
}

impl OptionValueType for BlockValue {}