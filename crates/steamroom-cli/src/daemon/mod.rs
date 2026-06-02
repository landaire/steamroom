//! Daemon mode for steamroom. See
//! `docs/superpowers/specs/2026-06-02-daemon-mode-design.md`.

pub mod client;
pub mod framing;
pub mod ipc;
pub mod lifecycle;
pub mod proto;
pub mod server;
pub mod tracing_layer;
pub mod tui;
