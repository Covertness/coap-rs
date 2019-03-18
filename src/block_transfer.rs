use super::client::CoAPClient;
use super::message::options::{BlockOption, BlockSize, CoAPOption};
use super::message::request::{CoAPRequest, Method};
use super::message::response::{CoAPResponse, Status};
use super::message::IsMessage;
use std::io::{Error};
use rand::Rng;


#[derive(PartialEq)]
enum BlockTransferProgress {
    Progress(u32, usize, BlockSize, u16),
    Finished(CoAPResponse),
}

pub fn send(path: &str, payload: Vec<u8>, client: &CoAPClient) -> Result<CoAPResponse, Error> {
    let mut send_progress = BlockTransferProgress::Progress(0, 0, client.get_config().get_block_size(), rand::thread_rng().gen());

    loop {
        send_progress = send_next_block(&path, &payload, client, send_progress)?;
        if let BlockTransferProgress::Finished(response) = send_progress {
            return Ok(response);
        }
    }
}

fn send_next_block(
    path: &str,
    payload: &[u8],
    client: &CoAPClient,
    send_progress: BlockTransferProgress,
) -> Result<BlockTransferProgress, Error> {
    let mut request = CoAPRequest::new();
    request.set_method(Method::Put);
    request.set_path(path);
        
    match send_progress {
        BlockTransferProgress::Progress(block_nr, bytes_sent, negotiated_block_size, message_id) => {
            request.set_message_id(message_id);
            let byte_index_of_block_end =
                std::cmp::min(bytes_sent + negotiated_block_size, payload.len());
            request.set_payload(payload[bytes_sent..byte_index_of_block_end].to_vec());
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
    client.set_receive_timeout(Some(client.get_config().get_timeout())).expect("Set timeout to zero");

    match client.receive() {
        Ok(response) => match response.get_status() {
            Status::Continue => match send_progress {
                BlockTransferProgress::Progress(block_nr, bytes_sent, block_size, message_id) => {
                    Ok(BlockTransferProgress::Progress(
                        block_nr + 1,
                        bytes_sent + block_size,
                        block_size,
                        message_id + 1,
                    ))
                }
                BlockTransferProgress::Finished(_) => Ok(send_progress),
            },
            _ => Ok(BlockTransferProgress::Finished(response))
        },
        Err(e) => Err(e)
    }
}
