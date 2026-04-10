use bytes::Bytes;
use std::sync::atomic::{AtomicU32, Ordering};
use tokio::sync::Mutex;
use crate::error::Error;
use super::Transport;
use super::capture::{CaptureFile, CapturedPacket};

pub struct RecordingTransport<T: Transport> {
    inner: T,
    packets: Mutex<Vec<CapturedPacket>>,
    seq: AtomicU32,
}

impl<T: Transport> RecordingTransport<T> {
    pub fn new(inner: T) -> Self {
        Self {
            inner,
            packets: Mutex::new(Vec::new()),
            seq: AtomicU32::new(0),
        }
    }

    pub async fn flush(&self) -> CaptureFile {
        let packets = {
            let mut guard = self.packets.lock().await;
            std::mem::take(&mut *guard)
        };
        CaptureFile {
            description: String::new(),
            packets,
        }
    }
}

impl<T: Transport> Transport for RecordingTransport<T> {
    async fn send(&self, payload: &[u8]) -> Result<(), Error> {
        let seq = self.seq.fetch_add(1, Ordering::Relaxed);
        let packet = CapturedPacket::new(seq, payload);
        self.packets.lock().await.push(packet);
        self.inner.send(payload).await
    }

    async fn recv(&self) -> Result<Bytes, Error> {
        let data = self.inner.recv().await?;
        let seq = self.seq.fetch_add(1, Ordering::Relaxed);
        let packet = CapturedPacket::new(seq, &data);
        self.packets.lock().await.push(packet);
        Ok(data)
    }
}
