//! Library surface for `steamroom-cli`. The binary lives in `main.rs`;
//! this exposes the daemon protocol and server pieces for integration
//! tests in `tests/`.

pub mod cli;
pub mod commands;
pub mod daemon;
pub mod download;
pub mod errors;
pub mod sink;
