use std::path::Path;
use bytes::Bytes;
use crate::error::Error;
use super::Transport;
use super::capture::CaptureFile;

pub struct ReplayTransport {
    // private
}

impl ReplayTransport {
    pub fn from_capture(capture: CaptureFile) -> Self {
        todo!()
    }

    pub fn from_file(path: &Path) -> Result<Self, Error> {
        todo!()
    }
}

impl Transport for ReplayTransport {
    async fn send(&self, _payload: &[u8]) -> Result<(), Error> {
        todo!()
    }

    async fn recv(&self) -> Result<Bytes, Error> {
        todo!()
    }
}
