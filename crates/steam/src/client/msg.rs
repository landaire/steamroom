use crate::messages::EMsg;

pub struct ClientMsg<'a> {
    pub emsg: EMsg,
    pub header: Vec<u8>,
    pub body: &'a [u8],
}

impl<'a> ClientMsg<'a> {
    pub fn new(emsg: EMsg) -> Self {
        Self {
            emsg,
            header: Vec::new(),
            body: &[],
        }
    }

    pub fn with_body(emsg: EMsg, body: &'a [u8]) -> Self {
        Self {
            emsg,
            header: Vec::new(),
            body,
        }
    }

    pub fn serialized_len(&self) -> usize {
        todo!()
    }

    pub fn write_to<W: std::io::Write>(&self, writer: &mut W) -> std::io::Result<()> {
        todo!()
    }
}
