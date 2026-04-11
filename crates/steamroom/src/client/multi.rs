use bytes::Bytes;
use byteorder::{LittleEndian, ReadBytesExt};
use prost::Message;
use std::io::{Cursor, Read};
use crate::error::Error;
use crate::generated::CMsgMulti;

pub fn unpack_multi(data: &[u8]) -> Result<Vec<Bytes>, Error> {
    let multi = CMsgMulti::decode(data)?;

    let payload = match multi.message_body {
        Some(body) => {
            if let Some(size_unzipped) = multi.size_unzipped {
                if size_unzipped > 0 {
                    // Gzip-compressed payload
                    let mut decoder = flate2::read::GzDecoder::new(&body[..]);
                    let mut decompressed = Vec::with_capacity(size_unzipped as usize);
                    decoder.read_to_end(&mut decompressed)?;
                    decompressed
                } else {
                    body
                }
            } else {
                body
            }
        }
        None => return Ok(Vec::new()),
    };

    // The payload contains length-prefixed sub-messages
    let mut messages = Vec::new();
    let mut cursor = Cursor::new(&payload);

    while (cursor.position() as usize) < payload.len() {
        let len = match cursor.read_u32::<LittleEndian>() {
            Ok(len) => len as usize,
            Err(_) => break,
        };
        let pos = cursor.position() as usize;
        if pos + len > payload.len() {
            break;
        }
        messages.push(Bytes::copy_from_slice(&payload[pos..pos + len]));
        cursor.set_position((pos + len) as u64);
    }

    Ok(messages)
}
