//! Socket name resolution and bind. `interprocess` handles the
//! platform-specific bits; this module just adds the stale-socket probe.

use std::time::Duration;

use interprocess::local_socket::{
    GenericNamespaced, ListenerOptions, Name, ToNsName,
    tokio::{Listener, Stream},
    traits::tokio::{Listener as _, Stream as _},
};

use crate::daemon::framing::{read_frame, write_frame};
use crate::daemon::proto::{Frame, Request, Response};
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
        .create_tokio()
        .map_err(CliError::Io)
}

pub async fn accept(listener: &Listener) -> Result<Stream, CliError> {
    listener.accept().await.map_err(CliError::Io)
}
