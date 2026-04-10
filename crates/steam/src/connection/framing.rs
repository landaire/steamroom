use bytes::Bytes;
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use crate::error::ParseError;

pub const MAGIC: [u8; 4] = *b"VT01";

#[derive(Clone, Debug)]
pub struct Frame {
    pub payload: Bytes,
}

pub fn frame_bytes(payload: &[u8]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(8 + payload.len());
    buf.write_u32::<LittleEndian>(payload.len() as u32).unwrap();
    buf.extend_from_slice(&MAGIC);
    buf.extend_from_slice(payload);
    buf
}

pub fn parse_frame(data: &[u8]) -> Result<Frame, ParseError> {
    if data.len() < 8 {
        return Err(ParseError::UnexpectedEof);
    }
    let mut cursor = &data[..8];
    let length = cursor.read_u32::<LittleEndian>().map_err(|_| ParseError::UnexpectedEof)?;
    let mut magic = [0u8; 4];
    magic.copy_from_slice(&data[4..8]);
    if magic != MAGIC {
        return Err(ParseError::InvalidProtobufHeader);
    }
    let payload_end = 8 + length as usize;
    if data.len() < payload_end {
        return Err(ParseError::UnexpectedEof);
    }
    Ok(Frame {
        payload: Bytes::copy_from_slice(&data[8..payload_end]),
    })
}

pub fn write_frame<W: std::io::Write>(writer: &mut W, payload: &[u8]) -> std::io::Result<()> {
    writer.write_u32::<LittleEndian>(payload.len() as u32)?;
    writer.write_all(&MAGIC)?;
    writer.write_all(payload)?;
    Ok(())
}
