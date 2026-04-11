use bytes::Bytes;
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use crate::error::ParseError;

pub const MAGIC: [u8; 4] = *b"VT01";

#[derive(Clone, Debug)]
pub struct Frame {
    pub payload: Bytes,
}

impl Frame {
    pub fn encode(payload: &[u8]) -> Vec<u8> {
        let mut buf = Vec::with_capacity(8 + payload.len());
        buf.write_u32::<LittleEndian>(payload.len() as u32).unwrap();
        buf.extend_from_slice(&MAGIC);
        buf.extend_from_slice(payload);
        buf
    }

    pub fn decode(data: &[u8]) -> Result<Self, ParseError> {
        if data.len() < 8 {
            return Err(ParseError::UnexpectedEof);
        }
        let mut cursor = &data[..8];
        let length = cursor
            .read_u32::<LittleEndian>()
            .map_err(|_| ParseError::UnexpectedEof)?;
        if data[4..8] != MAGIC {
            return Err(ParseError::InvalidProtobufHeader);
        }
        let payload_end = 8 + length as usize;
        if data.len() < payload_end {
            return Err(ParseError::UnexpectedEof);
        }
        Ok(Self {
            payload: Bytes::copy_from_slice(&data[8..payload_end]),
        })
    }

    pub fn write_to<W: std::io::Write>(&self, writer: &mut W) -> std::io::Result<()> {
        writer.write_u32::<LittleEndian>(self.payload.len() as u32)?;
        writer.write_all(&MAGIC)?;
        writer.write_all(&self.payload)?;
        Ok(())
    }
}
