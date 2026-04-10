use std::path::Path;
use bytes::Bytes;
use tokio::sync::Mutex;
use crate::error::{ConnectionError, Error};
use super::Transport;
use super::capture::CaptureFile;

pub struct ReplayTransport {
    packets: Mutex<std::collections::VecDeque<Vec<u8>>>,
}

impl ReplayTransport {
    pub fn from_capture(capture: CaptureFile) -> Self {
        let packets: std::collections::VecDeque<Vec<u8>> = capture
            .packets
            .iter()
            .filter_map(|p| p.decode_payload().ok())
            .collect();
        Self {
            packets: Mutex::new(packets),
        }
    }

    pub fn from_file(path: &Path) -> Result<Self, Error> {
        let capture = CaptureFile::load(path).map_err(Error::Io)?;
        Ok(Self::from_capture(capture))
    }
}

impl Transport for ReplayTransport {
    async fn send(&self, _payload: &[u8]) -> Result<(), Error> {
        // In replay mode, sends are no-ops
        Ok(())
    }

    async fn recv(&self) -> Result<Bytes, Error> {
        let mut packets = self.packets.lock().await;
        packets
            .pop_front()
            .map(Bytes::from)
            .ok_or_else(|| ConnectionError::Disconnected.into())
    }
}
