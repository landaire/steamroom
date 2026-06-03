use super::Transport;
use super::capture::CaptureFile;
use super::capture::CapturedPacket;
use crate::error::Error;
use bytes::Bytes;
use futures_util::lock::Mutex;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::AtomicU32;
use std::sync::atomic::Ordering;

/// Clonable handle to a recording's packet buffer. Cloning shares the
/// same underlying buffer, so a `Recorder` taken before the transport is
/// moved into a `SteamClient` can still flush the capture afterward.
#[derive(Clone, Default)]
pub struct Recorder {
    packets: Arc<Mutex<Vec<CapturedPacket>>>,
}

impl Recorder {
    /// Drain the recorded packets into a `CaptureFile`.
    pub async fn flush(&self) -> CaptureFile {
        let packets = std::mem::take(&mut *self.packets.lock().await);
        CaptureFile {
            description: String::new(),
            packets,
        }
    }
}

/// Wraps a transport and records every received packet into a shared
/// `Recorder`. Only the server-to-client direction is captured, matching
/// what `ReplayTransport` consumes: replay returns these packets in order
/// from `recv` and treats `send` as a no-op.
pub struct RecordingTransport<T: Transport> {
    inner: T,
    recorder: Recorder,
    seq: AtomicU32,
}

impl<T: Transport> RecordingTransport<T> {
    pub fn new(inner: T) -> Self {
        Self::with_recorder(inner, Recorder::default())
    }

    /// Record into a caller-supplied `Recorder`. The caller keeps a clone
    /// of `recorder` to flush the capture after this transport has been
    /// moved into a `SteamClient`.
    pub fn with_recorder(inner: T, recorder: Recorder) -> Self {
        Self {
            inner,
            recorder,
            seq: AtomicU32::new(0),
        }
    }

    /// Clonable handle to the recorded packets. Take this before moving
    /// the transport into a client so the capture can be flushed later.
    pub fn recorder(&self) -> Recorder {
        self.recorder.clone()
    }
}

impl<T: Transport> Transport for RecordingTransport<T> {
    fn send(
        &self,
        payload: &[u8],
    ) -> Pin<Box<dyn std::future::Future<Output = Result<(), Error>> + Send + '_>> {
        // Sent packets are not recorded: a capture is the recv stream that
        // ReplayTransport plays back. Forward the real payload unchanged.
        let payload = payload.to_vec();
        Box::pin(async move { self.inner.send(&payload).await })
    }

    fn recv(&self) -> Pin<Box<dyn std::future::Future<Output = Result<Bytes, Error>> + Send + '_>> {
        Box::pin(async move {
            let data = self.inner.recv().await?;
            let seq = self.seq.fetch_add(1, Ordering::Relaxed);
            let packet = CapturedPacket::new(seq, &data);
            self.recorder.packets.lock().await.push(packet);
            Ok(data)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::replay::ReplayTransport;
    use super::*;

    /// Inner transport that hands back canned recv payloads and records
    /// what was sent, so the test can assert payloads are forwarded.
    struct FakeInner {
        to_recv: Mutex<std::collections::VecDeque<Vec<u8>>>,
        sent: Mutex<Vec<Vec<u8>>>,
    }

    impl Transport for FakeInner {
        fn send(
            &self,
            payload: &[u8],
        ) -> Pin<Box<dyn std::future::Future<Output = Result<(), Error>> + Send + '_>> {
            let payload = payload.to_vec();
            Box::pin(async move {
                self.sent.lock().await.push(payload);
                Ok(())
            })
        }

        fn recv(
            &self,
        ) -> Pin<Box<dyn std::future::Future<Output = Result<Bytes, Error>> + Send + '_>> {
            Box::pin(async move {
                let next = self.to_recv.lock().await.pop_front();
                match next {
                    Some(p) => Ok(Bytes::from(p)),
                    None => Err(crate::error::ConnectionError::Disconnected.into()),
                }
            })
        }
    }

    #[tokio::test]
    async fn forwards_payload_and_records_only_recv() {
        let inner = FakeInner {
            to_recv: Mutex::new(vec![vec![1, 2, 3], vec![4, 5]].into()),
            sent: Mutex::new(Vec::new()),
        };
        let rt = RecordingTransport::new(inner);
        let recorder = rt.recorder();

        rt.send(b"hello").await.unwrap();
        let a = rt.recv().await.unwrap();
        let b = rt.recv().await.unwrap();
        assert_eq!(&a[..], &[1, 2, 3]);
        assert_eq!(&b[..], &[4, 5]);

        // send must forward the real payload, not an empty slice.
        assert_eq!(rt.inner.sent.lock().await.as_slice(), &[b"hello".to_vec()]);

        // Only the two received packets are captured, in order.
        let capture = recorder.flush().await;
        assert_eq!(capture.packets.len(), 2);
        assert_eq!(capture.packets[0].decode_payload().unwrap(), vec![1, 2, 3]);
        assert_eq!(capture.packets[1].decode_payload().unwrap(), vec![4, 5]);

        // The capture replays back through ReplayTransport in the same order.
        let replay = ReplayTransport::from_capture(capture);
        assert_eq!(&replay.recv().await.unwrap()[..], &[1, 2, 3]);
        assert_eq!(&replay.recv().await.unwrap()[..], &[4, 5]);
    }
}
