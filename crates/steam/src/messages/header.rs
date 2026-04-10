use bytes::Bytes;
use super::EMsg;
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

pub fn parse_packet_header(data: &[u8]) -> Result<PacketHeader, ParseError> {
    todo!()
}
