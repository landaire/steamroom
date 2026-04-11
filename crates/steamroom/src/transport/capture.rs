use serde::Deserialize;
use serde::Serialize;
use std::path::Path;

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
        let json = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(path, json)
    }

    pub fn load(path: &Path) -> Result<Self, std::io::Error> {
        let data = std::fs::read_to_string(path)?;
        serde_json::from_str(&data)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
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
        use base64::Engine;
        Self {
            seq,
            emsg: None,
            payload_b64: base64::engine::general_purpose::STANDARD.encode(payload),
        }
    }

    pub fn decode_payload(&self) -> Result<Vec<u8>, base64::DecodeError> {
        use base64::Engine;
        base64::engine::general_purpose::STANDARD.decode(&self.payload_b64)
    }
}
