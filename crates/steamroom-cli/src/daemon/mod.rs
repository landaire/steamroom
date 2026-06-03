//! Daemon mode for steamroom.

pub mod client;
pub mod framing;
pub mod ipc;
pub mod lifecycle;
pub mod proto;
pub mod server;
pub mod tracing_layer;

#[cfg(feature = "tui")]
pub mod tui;
