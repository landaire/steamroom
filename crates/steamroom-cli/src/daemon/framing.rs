//! Async length-prefixed rkyv framing for daemon IPC.
//!
//! Wire format per frame:
//! ```text
//!   u16 LE   proto_version
//!   u32 LE   payload_length (<= MAX_FRAME_BYTES)
//!   [u8]     rkyv-archived Frame
//! ```
//! The version is checked before deserialization so mismatched daemons
//! and clients fail with a clear error rather than rkyv validation noise.

use rkyv::{rancor, util::AlignedVec};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::daemon::proto::{Frame, PROTO_VERSION};
use crate::errors::CliError;

pub const MAX_FRAME_BYTES: u32 = 16 * 1024 * 1024;

/// Map `UnexpectedEof` to `CliError::SocketClosed`; all other I/O errors
/// become `CliError::Io`.  Used by `read_frame` for all three reads so that
/// a peer disconnecting mid-frame always yields `SocketClosed` rather than a
/// raw I/O error.
async fn read_exact_or_closed<R>(r: &mut R, buf: &mut [u8]) -> Result<(), CliError>
where
    R: AsyncReadExt + Unpin,
{
    match r.read_exact(buf).await {
        Ok(_) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => Err(CliError::SocketClosed),
        Err(e) => Err(CliError::Io(e)),
    }
}

pub async fn write_frame<W>(w: &mut W, frame: &Frame) -> Result<(), CliError>
where
    W: AsyncWriteExt + Unpin,
{
    let bytes = rkyv::to_bytes::<rancor::Error>(frame)
        .map_err(|e| CliError::MalformedFrame(e.to_string()))?;
    let len_usize = bytes.len();
    if len_usize > MAX_FRAME_BYTES as usize {
        return Err(CliError::FrameTooLarge {
            len_bytes: len_usize as u64,
            limit_bytes: MAX_FRAME_BYTES as u64,
        });
    }
    // len_usize <= MAX_FRAME_BYTES (u32-sized), so the cast is lossless.
    let len: u32 = len_usize as u32;
    w.write_all(&PROTO_VERSION.to_le_bytes()).await.map_err(CliError::Io)?;
    w.write_all(&len.to_le_bytes()).await.map_err(CliError::Io)?;
    w.write_all(&bytes).await.map_err(CliError::Io)?;
    w.flush().await.map_err(CliError::Io)?;
    Ok(())
}

/// Read one length-prefixed rkyv-archived `Frame` from `r`.
///
/// # Cancel safety
///
/// This function is NOT cancel-safe. `AsyncReadExt::read_exact` may
/// partially consume bytes from the stream before the future is dropped,
/// leaving the connection in an undefined framing state. Callers that
/// race this against another future (e.g. via `tokio::select!` on a
/// shutdown signal) MUST abort the connection on cancellation rather
/// than re-entering `read_frame` on the same stream.
pub async fn read_frame<R>(r: &mut R) -> Result<Frame, CliError>
where
    R: AsyncReadExt + Unpin,
{
    let mut ver_buf = [0u8; 2];
    read_exact_or_closed(r, &mut ver_buf).await?;
    let peer = u16::from_le_bytes(ver_buf);
    if peer != PROTO_VERSION {
        return Err(CliError::ProtocolVersionMismatch { peer, ours: PROTO_VERSION });
    }

    let mut len_buf = [0u8; 4];
    read_exact_or_closed(r, &mut len_buf).await?;
    let len = u32::from_le_bytes(len_buf);
    if len > MAX_FRAME_BYTES {
        return Err(CliError::FrameTooLarge {
            len_bytes: len as u64,
            limit_bytes: MAX_FRAME_BYTES as u64,
        });
    }

    // rkyv's checked access demands 16-byte alignment.
    let mut buf = AlignedVec::<16>::with_capacity(len as usize);
    buf.resize(len as usize, 0);
    read_exact_or_closed(r, &mut buf).await?;
    rkyv::from_bytes::<Frame, rancor::Error>(&buf)
        .map_err(|e| CliError::MalformedFrame(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::proto::{Event, JobId, LogLevel, Response};
    use tokio::io::duplex;

    #[tokio::test]
    async fn round_trip_through_duplex() {
        let (mut a, mut b) = duplex(64 * 1024);
        let frame = Frame::Response(Response::Stopping);

        write_frame(&mut a, &frame).await.unwrap();
        let back = read_frame(&mut b).await.unwrap();
        assert!(matches!(back, Frame::Response(Response::Stopping)));
    }

    #[tokio::test]
    async fn rejects_mismatched_version() {
        let (mut a, mut b) = duplex(64);
        // Hand-craft a frame with version 999.
        a.write_all(&999u16.to_le_bytes()).await.unwrap();
        a.write_all(&0u32.to_le_bytes()).await.unwrap();
        a.flush().await.unwrap();
        let err = read_frame(&mut b).await.unwrap_err();
        match err {
            CliError::ProtocolVersionMismatch { peer, ours } => {
                assert_eq!(peer, 999);
                assert_eq!(ours, PROTO_VERSION);
            }
            other => panic!("wrong error: {other:?}"),
        }
    }

    #[tokio::test]
    async fn rejects_oversized_length() {
        let (mut a, mut b) = duplex(64);
        a.write_all(&PROTO_VERSION.to_le_bytes()).await.unwrap();
        a.write_all(&(MAX_FRAME_BYTES + 1).to_le_bytes()).await.unwrap();
        a.flush().await.unwrap();
        let err = read_frame(&mut b).await.unwrap_err();
        assert!(matches!(err, CliError::FrameTooLarge { .. }));
    }

    #[tokio::test]
    async fn event_with_log_round_trips() {
        let (mut a, mut b) = duplex(64 * 1024);
        let frame = Frame::Event(Event::Log {
            job_id: Some(JobId(1)),
            level: LogLevel::Info,
            target: "t".into(),
            message: "hello".into(),
        });
        write_frame(&mut a, &frame).await.unwrap();
        let back = read_frame(&mut b).await.unwrap();
        match back {
            Frame::Event(Event::Log { message, .. }) => assert_eq!(message, "hello"),
            other => panic!("wrong: {other:?}"),
        }
    }
}
