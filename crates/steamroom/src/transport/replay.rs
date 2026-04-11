use super::capture::CaptureFile;
use super::Transport;
use crate::error::ConnectionError;
use crate::error::Error;
use bytes::Bytes;
use futures_util::lock::Mutex;
use std::path::Path;
use std::pin::Pin;

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
    fn send(
        &self,
        _payload: &[u8],
    ) -> Pin<Box<dyn std::future::Future<Output = Result<(), Error>> + Send + '_>> {
        Box::pin(async { Ok(()) })
    }

    fn recv(&self) -> Pin<Box<dyn std::future::Future<Output = Result<Bytes, Error>> + Send + '_>> {
        Box::pin(async {
            self.packets
                .lock()
                .await
                .pop_front()
                .map(Bytes::from)
                .ok_or_else(|| ConnectionError::Disconnected.into())
        })
    }
}
