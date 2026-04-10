use bytes::Bytes;
use std::pin::Pin;
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
        let packets = std::mem::take(&mut *self.packets.lock().await);
        CaptureFile {
            description: String::new(),
            packets,
        }
    }
}

impl<T: Transport> Transport for RecordingTransport<T> {
    fn send(&self, payload: &[u8]) -> Pin<Box<dyn std::future::Future<Output = Result<(), Error>> + Send + '_>> {
        let seq = self.seq.fetch_add(1, Ordering::Relaxed);
        let packet = CapturedPacket::new(seq, payload);
        Box::pin(async move {
            self.packets.lock().await.push(packet);
            self.inner.send(&[]).await // TODO: forward original payload
        })
    }

    fn recv(&self) -> Pin<Box<dyn std::future::Future<Output = Result<Bytes, Error>> + Send + '_>> {
        Box::pin(async move {
            let data = self.inner.recv().await?;
            let seq = self.seq.fetch_add(1, Ordering::Relaxed);
            let packet = CapturedPacket::new(seq, &data);
            self.packets.lock().await.push(packet);
            Ok(data)
        })
    }
}
