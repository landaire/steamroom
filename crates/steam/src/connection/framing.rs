use bytes::Bytes;
use crate::error::ParseError;

pub const MAGIC: [u8; 4] = *b"VT01";

#[derive(Clone, Debug)]
pub struct Frame {
    pub payload: Bytes,
}

pub fn frame_bytes(payload: &[u8]) -> Vec<u8> {
    todo!()
}

pub fn parse_frame(data: &[u8]) -> Result<Frame, ParseError> {
    todo!()
}

pub fn write_frame<W: std::io::Write>(writer: &mut W, payload: &[u8]) -> std::io::Result<()> {
    todo!()
}
