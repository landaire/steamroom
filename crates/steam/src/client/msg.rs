use prost::Message;
use crate::generated::CMsgProtoBufHeader;
use crate::messages::{EMsg, RawEMsg};

pub struct ClientMsg<'a> {
    pub emsg: EMsg,
    pub header: CMsgProtoBufHeader,
    pub body: &'a [u8],
}

impl<'a> ClientMsg<'a> {
    pub fn new(emsg: EMsg) -> Self {
        Self {
            emsg,
            header: CMsgProtoBufHeader::default(),
            body: &[],
        }
    }

    pub fn with_body(emsg: EMsg, body: &'a [u8]) -> Self {
        Self {
            emsg,
            header: CMsgProtoBufHeader::default(),
            body,
        }
    }

    pub fn serialized_len(&self) -> usize {
        let header_len = self.header.encoded_len();
        // u32 raw_emsg + u32 header_len + header + body
        4 + 4 + header_len + self.body.len()
    }

    pub fn write_to<W: std::io::Write>(&self, writer: &mut W) -> std::io::Result<()> {
        let raw = RawEMsg::with_proto(self.emsg);
        writer.write_all(&raw.0.to_le_bytes())?;

        let header_bytes = self.header.encode_to_vec();
        writer.write_all(&(header_bytes.len() as u32).to_le_bytes())?;
        writer.write_all(&header_bytes)?;
        writer.write_all(self.body)?;
        Ok(())
    }

    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(self.serialized_len());
        self.write_to(&mut buf).unwrap();
        buf
    }
}
