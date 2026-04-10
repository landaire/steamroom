use std::path::Path;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CaptureFile {
    pub description: String,
    pub packets: Vec<CapturedPacket>,
}

impl CaptureFile {
    pub fn new(description: impl Into<String>) -> Self {
        Self {
            description: description.into(),
            packets: Vec::new(),
        }
    }

    pub fn save(&self, path: &Path) -> Result<(), std::io::Error> {
        todo!()
    }

    pub fn load(path: &Path) -> Result<Self, std::io::Error> {
        todo!()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CapturedPacket {
    pub seq: u32,
    pub emsg: Option<u32>,
    pub payload_b64: String,
}

impl CapturedPacket {
    pub fn new(seq: u32, payload: &[u8]) -> Self {
        todo!()
    }

    pub fn decode_payload(&self) -> Result<Vec<u8>, base64::DecodeError> {
        todo!()
    }
}
