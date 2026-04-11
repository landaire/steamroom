/// Serializable packet capture format for recording and replaying sessions.
pub mod capture;
/// Wrap a transport to record all packets to a capture file.
pub mod recording;
/// Replay a previously captured session for deterministic testing.
pub mod replay;
/// TCP transport with VT01 framing and session cipher.
pub mod tcp;
/// WebSocket transport over TLS.
pub mod websocket;

use crate::error::Error;
use bytes::Bytes;

pub trait Transport: Send + Sync + 'static {
    fn send(
        &self,
        payload: &[u8],
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), Error>> + Send + '_>>;

    fn recv(
        &self,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Bytes, Error>> + Send + '_>>;
}
