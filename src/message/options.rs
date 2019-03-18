use num_derive::FromPrimitive;
use std::ops::Add;

#[derive(Debug)]
pub enum OptionCreateError {
    InvalidBlockNumber
}

#[derive(PartialEq, Eq, Clone, Copy, Debug)]
pub enum BlockSize {
    S1024 = 6,
    S512 = 5,
    S256 = 4,
    S128 = 3,
    S64 = 2,
    S32 = 1,
    S16 = 0
}

impl Add<BlockSize> for usize {
    type Output = usize;
    fn add(self, other: BlockSize) -> usize {
        let nr_of_bytes = match other {
            BlockSize::S1024 => 1024,
            BlockSize::S512 => 512,
            BlockSize::S256 => 256,
            BlockSize::S128 => 128,
            BlockSize::S64 => 64,
            BlockSize::S32 => 32,
            BlockSize::S16 => 16
        };
        self + nr_of_bytes
    }
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

    #[test]
    fn test_to_add_usize_to_block_option() {
        assert_eq!(1 + BlockSize::S1024, 1025);
        assert_eq!(1 + BlockSize::S512, 513);
        assert_eq!(1 + BlockSize::S256, 257);
        assert_eq!(1 + BlockSize::S128, 129);
        assert_eq!(1 + BlockSize::S64, 65);
        assert_eq!(1 + BlockSize::S32, 33);
        assert_eq!(1 + BlockSize::S16, 17);
    }
}