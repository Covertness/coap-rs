use num_derive::FromPrimitive;

#[derive(Debug)]
pub enum OptionCreateError {
    InvalidBlockNumber
}

#[derive(Debug)]
pub enum BlockSize {
    S1024 = 6,
    S515 = 5,
    S256 = 4,
    S128 = 3,
    S64 = 2,
    S32 = 1,
    S16 = 0
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
pub enum ObserveOption {
    Register = 0,
    Deregister = 1,
}

#[derive(Debug)]
pub struct BlockOption {
    num: u32,
    m: bool,
    block_size: BlockSize
}

impl BlockOption {
    pub fn new(num: u32, m: bool, block_size: BlockSize) -> Result<BlockOption, OptionCreateError> {
        if num > 2u32.pow(20) {
            return Err(OptionCreateError::InvalidBlockNumber)
        }
        
        Ok(BlockOption {
            num,
            m,
            block_size
        })
    }
}

impl From<BlockOption> for Vec<u8> {
    fn from(option: BlockOption) -> Self {
        let block_as_u32: u32;
        let more_blocks = u32::from(option.m);
        let block_size = option.block_size as u32;
        block_as_u32 = (option.num << 4) |
                 (more_blocks << 3) |
                 (block_size);

        let mut result: Vec<u8> = Vec::new();
        let mut next_byte = ((block_as_u32 >> 16) & 0xff) as u8;
        if next_byte != 0 {
            result.push(next_byte);    
        }
        next_byte = ((block_as_u32 >> 8) & 0xff) as u8;
        if next_byte != 0 {
            result.push(next_byte);
        }
        result.push((block_as_u32 & 0xff) as u8);
        
        result
    }
}

#[cfg(test)]
mod test {
    use super::{ BlockOption, BlockSize };

    #[test]
    fn create_one_byte_block_option() {
        let block_option = BlockOption::new(3, true, BlockSize::S1024).expect("Failed to create block option");
        let u8_vector: Vec<u8> = Vec::from(block_option);
        assert_eq!(u8_vector.len(), 1)
    }

    #[test]
    fn create_two_byte_block_option() {
        let block_option = BlockOption::new(20, true, BlockSize::S1024).expect("Failed to create block option");
        let u8_vector: Vec<u8> = Vec::from(block_option);
        assert_eq!(u8_vector.len(), 2)
    }

    #[test]
    fn create_three_byte_block_option() {
        let block_option = BlockOption::new(1000000, true, BlockSize::S1024).expect("Failed to create block option");
        let u8_vector: Vec<u8> = Vec::from(block_option);
        assert_eq!(u8_vector.len(), 3)
    }

    #[test]
    fn fail_on_to_large_num_for_block_option() {
        BlockOption::new(2000000, true, BlockSize::S1024).expect_err("Creating block with NUM so large should fail");
    }
}