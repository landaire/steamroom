use bytes::Bytes;
use byteorder::{LittleEndian, ReadBytesExt};
use std::io::Cursor;
use prost::Message;
use super::{EMsg, RawEMsg};
use crate::error::ParseError;

#[derive(Clone, Debug)]
pub struct MsgHdr {
    pub emsg: EMsg,
    pub target_job_id: u64,
    pub source_job_id: u64,
}

#[derive(Clone, Debug)]
pub struct MsgHdrProtoBuf {
    pub emsg: EMsg,
    pub is_protobuf: bool,
    pub header_data: Bytes,
}

#[derive(Clone, Debug)]
pub struct ExtendedClientMsgHdr {
    pub emsg: EMsg,
    pub header_size: u8,
    pub header_version: u16,
    pub target_job_id: u64,
    pub source_job_id: u64,
    pub header_canary: u8,
    pub steam_id: u64,
    pub session_id: i32,
}

#[derive(Clone, Debug)]
pub enum PacketHeader {
    Simple {
        header: MsgHdr,
        body: Bytes,
    },
    Extended {
        header: ExtendedClientMsgHdr,
        body: Bytes,
    },
    Protobuf {
        header: MsgHdrProtoBuf,
        body: Bytes,
    },
}

impl PacketHeader {
    pub fn parse(data: &[u8]) -> Result<Self, ParseError> {
        if data.len() < 4 {
            return Err(ParseError::UnexpectedEof);
        }

        let raw = u32::from_le_bytes(data[..4].try_into().unwrap());
        let raw_emsg = RawEMsg(raw);

        if raw_emsg.is_protobuf() {
            parse_protobuf_header(data, raw_emsg)
        } else {
            parse_simple_header(data, raw_emsg)
        }
    }
}

fn parse_protobuf_header(data: &[u8], raw_emsg: RawEMsg) -> Result<PacketHeader, ParseError> {
    // Layout: u32 raw_emsg | u32 header_length | [header_length bytes of CMsgProtoBufHeader] | body
    if data.len() < 8 {
        return Err(ParseError::UnexpectedEof);
    }

    let header_length = u32::from_le_bytes(data[4..8].try_into().unwrap()) as usize;
    let header_end = 8 + header_length;

    if data.len() < header_end {
        return Err(ParseError::UnexpectedEof);
    }

    let header_data = Bytes::copy_from_slice(&data[8..header_end]);
    let body = Bytes::copy_from_slice(&data[header_end..]);

    Ok(PacketHeader::Protobuf {
        header: MsgHdrProtoBuf {
            emsg: raw_emsg.emsg(),
            is_protobuf: true,
            header_data,
        },
        body,
    })
}

fn parse_simple_header(data: &[u8], raw_emsg: RawEMsg) -> Result<PacketHeader, ParseError> {
    // Simple MsgHdr: u32 emsg | u64 target_job_id | u64 source_job_id = 20 bytes
    if data.len() < 20 {
        return Err(ParseError::UnexpectedEof);
    }

    let mut cursor = Cursor::new(&data[4..]);
    let target_job_id = cursor.read_u64::<LittleEndian>().map_err(|_| ParseError::UnexpectedEof)?;
    let source_job_id = cursor.read_u64::<LittleEndian>().map_err(|_| ParseError::UnexpectedEof)?;
    let body = Bytes::copy_from_slice(&data[20..]);

    Ok(PacketHeader::Simple {
        header: MsgHdr {
            emsg: raw_emsg.emsg(),
            target_job_id,
            source_job_id,
        },
        body,
    })
}

impl MsgHdrProtoBuf {
    pub fn decode_header(&self) -> Result<crate::generated::CMsgProtoBufHeader, prost::DecodeError> {
        crate::generated::CMsgProtoBufHeader::decode(&*self.header_data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_protobuf_header_roundtrip() {
        // Build a minimal protobuf packet
        let emsg = RawEMsg::with_proto(EMsg::CLIENT_LOG_ON_RESPONSE);
        let proto_header = crate::generated::CMsgProtoBufHeader {
            steamid: Some(12345),
            ..Default::default()
        };
        let header_bytes = proto_header.encode_to_vec();
        let body = b"test body";

        let mut packet = Vec::new();
        packet.extend_from_slice(&emsg.0.to_le_bytes());
        packet.extend_from_slice(&(header_bytes.len() as u32).to_le_bytes());
        packet.extend_from_slice(&header_bytes);
        packet.extend_from_slice(body);

        let parsed = PacketHeader::parse(&packet).unwrap();
        match parsed {
            PacketHeader::Protobuf { header, body: b } => {
                assert_eq!(header.emsg, EMsg::CLIENT_LOG_ON_RESPONSE);
                assert!(header.is_protobuf);
                let decoded = header.decode_header().unwrap();
                assert_eq!(decoded.steamid, Some(12345));
                assert_eq!(&*b, b"test body");
            }
            _ => panic!("expected protobuf header"),
        }
    }
}
