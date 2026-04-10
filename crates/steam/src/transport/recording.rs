use bytes::Bytes;
use crate::error::Error;
use super::Transport;
use super::capture::CaptureFile;

pub struct RecordingTransport<T: Transport> {
    inner: T,
    // capture state
}

impl<T: Transport> RecordingTransport<T> {
    pub fn new(inner: T) -> Self {
        todo!()
    }

    pub fn flush(&self) -> CaptureFile {
        todo!()
    }
}

impl<T: Transport> Transport for RecordingTransport<T> {
    async fn send(&self, payload: &[u8]) -> Result<(), Error> {
        todo!()
    }

    async fn recv(&self) -> Result<Bytes, Error> {
        todo!()
    }
}
