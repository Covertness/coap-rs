use super::client::CoAPClient;
use super::message::options::{BlockOption, BlockSize, CoAPOption};
use super::message::request::{CoAPRequest, Method};
use super::message::response::Status;
use super::message::IsMessage;
use std::io::{Error, ErrorKind};
use url::Url;

#[derive(Debug)]
pub enum BlockTransferError {
    FailedToSendBlockTransfer,
    UnexpectedResponseCode(Status),
    IoError(Error),
}

impl From<Error> for BlockTransferError {
    fn from(error: Error) -> Self {
        BlockTransferError::IoError(error)
    }
}

#[derive(PartialEq)]
enum BlockTransferProgress {
    Progress(u32, u64, BlockSize),
    Finished(Status),
}

pub fn send(url: Url, payload: Vec<u8>, client: &CoAPClient) -> Result<Status, BlockTransferError> {
    let mut send_progress = BlockTransferProgress::Progress(0, 0, BlockSize::S1024);

    loop {
        send_progress = send_next_block(&url, &payload, client, send_progress)?;
        if let BlockTransferProgress::Finished(response) = send_progress {
            return Ok(response);
        }
    }
}

fn send_next_block(
    url: &Url,
    payload: &[u8],
    client: &CoAPClient,
    send_progress: BlockTransferProgress,
) -> Result<BlockTransferProgress, BlockTransferError> {
    let mut request = CoAPRequest::new();
    request.set_method(Method::Put);
    request.set_path(url.path());
    match send_progress {
        BlockTransferProgress::Progress(block_nr, bytes_sent, negotiated_block_size) => {
            let byte_index_of_block_end =
                std::cmp::min((bytes_sent + negotiated_block_size) as usize, payload.len());
            request.set_payload(payload[bytes_sent as usize..byte_index_of_block_end].to_vec());
            request.add_option(
                CoAPOption::Block1,
                BlockOption::new(
                    block_nr,
                    byte_index_of_block_end < payload.len(),
                    negotiated_block_size,
                )
                .expect("Could not create block option")
                .into(),
            );
        }
        BlockTransferProgress::Finished(_) => return Ok(send_progress),
    }

    client.send(&request)?;

    match client.receive() {
        Ok(response) => match response.get_status() {
            Status::Continue => match send_progress {
                BlockTransferProgress::Progress(block_nr, bytes_sent, block_size) => {
                    Ok(BlockTransferProgress::Progress(
                        block_nr + 1,
                        bytes_sent + block_size,
                        block_size,
                    ))
                }
                BlockTransferProgress::Finished(_) => Ok(send_progress),
            },
            Status::Changed => Ok(BlockTransferProgress::Finished(Status::Changed)), // We shouldn't get here, if we do, just re-iterate that this transfer has Finished.
            _ => Err(BlockTransferError::UnexpectedResponseCode(
                response.get_status().clone(),
            )),
        },
        Err(e) => {
            match e.kind() {
                ErrorKind::WouldBlock => Err(BlockTransferError::FailedToSendBlockTransfer), // Unix
                ErrorKind::TimedOut => Err(BlockTransferError::FailedToSendBlockTransfer), // Windows
                _ => Err(BlockTransferError::FailedToSendBlockTransfer),
            }
        }
    }
}
