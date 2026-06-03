//! Socket name resolution and bind. `interprocess` handles the
//! platform-specific bits; this module just adds the stale-socket probe.

use std::time::Duration;

use interprocess::local_socket::GenericNamespaced;
use interprocess::local_socket::ListenerOptions;
use interprocess::local_socket::Name;
use interprocess::local_socket::ToNsName;
use interprocess::local_socket::tokio::Listener;
use interprocess::local_socket::tokio::Stream;
use interprocess::local_socket::traits::tokio::Listener as _;
use interprocess::local_socket::traits::tokio::Stream as _;

use crate::daemon::framing::read_frame;
use crate::daemon::framing::write_frame;
use crate::daemon::proto::Frame;
use crate::daemon::proto::Request;
use crate::daemon::proto::Response;
use crate::errors::CliError;

/// Build the platform-appropriate name for the current user's daemon.
pub fn socket_name() -> Result<Name<'static>, CliError> {
    let raw = socket_name_string();
    raw.to_ns_name::<GenericNamespaced>().map_err(CliError::Io)
}

pub fn socket_name_string() -> String {
    #[cfg(unix)]
    {
        // getuid is infallible on supported targets.
        let uid = unsafe { libc::getuid() };
        format!("steamroom-{uid}.sock")
    }
    #[cfg(windows)]
    {
        let user = std::env::var("USERNAME").unwrap_or_else(|_| "user".into());
        format!("steamroom-{user}")
    }
}

/// Connect-and-probe: send a Status request with a short read timeout.
/// Returns Ok if a peer answered, Err otherwise. Used to differentiate
/// "stale socket file" from "daemon already running".
pub async fn probe_peer() -> Result<(), CliError> {
    let name = socket_name()?;
    let mut stream = Stream::connect(name).await.map_err(CliError::Io)?;
    write_frame(&mut stream, &Frame::Request(Request::Status)).await?;
    let fut = read_frame(&mut stream);
    let resp = tokio::time::timeout(Duration::from_millis(200), fut)
        .await
        .map_err(|_| {
            CliError::Io(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "probe timed out",
            ))
        })??;
    match resp {
        Frame::Response(Response::Status(_)) => Ok(()),
        other => Err(CliError::MalformedFrame(format!(
            "probe expected Status, got {other:?}"
        ))),
    }
}

/// Bind the daemon's listener. Returns `Err(CliError::DaemonAlreadyRunning)`
/// if a probe shows a live peer; otherwise overwrites stale sockets.
pub async fn bind_listener() -> Result<Listener, CliError> {
    if probe_peer().await.is_ok() {
        return Err(CliError::DaemonAlreadyRunning);
    }
    let name = socket_name()?;
    ListenerOptions::new()
        .name(name)
        // The probe above confirmed no live peer, so overwriting a stale
        // socket file (e.g. from a crashed daemon) is safe.
        .try_overwrite(true)
        .create_tokio()
        .map_err(CliError::Io)
}

pub async fn accept(listener: &Listener) -> Result<Stream, CliError> {
    listener.accept().await.map_err(CliError::Io)
}
