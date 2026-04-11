use crate::error::ParseError;
use bytes::Bytes;

pub const MAGIC: [u8; 4] = *b"VT01";

#[derive(Clone, Debug)]
pub struct Frame {
    pub payload: Bytes,
}

impl Frame {
    pub const HEADER_LEN: usize = 8;

    pub fn encoded_len(payload_len: usize) -> usize {
        Self::HEADER_LEN + payload_len
    }

    pub fn write_to<W: std::io::Write>(writer: &mut W, payload: &[u8]) -> std::io::Result<()> {
        writer.write_all(&(payload.len() as u32).to_le_bytes())?;
        writer.write_all(&MAGIC)?;
        writer.write_all(payload)?;
        Ok(())
    }

    pub fn encode(payload: &[u8]) -> Vec<u8> {
        let mut buf = Vec::with_capacity(Self::encoded_len(payload.len()));
        Self::write_to(&mut buf, payload).unwrap();
        buf
    }

    pub fn decode(data: &[u8]) -> Result<Self, ParseError> {
        if data.len() < Self::HEADER_LEN {
            return Err(ParseError::UnexpectedEof);
        }
        let length = u32::from_le_bytes(data[..4].try_into().unwrap()) as usize;
        if data[4..8] != MAGIC {
            return Err(ParseError::InvalidProtobufHeader);
        }
        let payload_end = Self::HEADER_LEN + length;
        if data.len() < payload_end {
            return Err(ParseError::UnexpectedEof);
        }
        Ok(Self {
            payload: Bytes::copy_from_slice(&data[Self::HEADER_LEN..payload_end]),
        })
    }
}
