//! Per-subcommand handlers. Each `run_*` takes an injected `JobSink`,
//! an externally-supplied authenticated `SteamClient<LoggedIn>`, and a
//! `CancellationToken`. The daemon worker reuses these across requests
//! without changing the direct-mode CLI's behavior.
//!
//! Signature uniformity exceptions:
//! - `run_local_info` takes no `SteamClient` (operates entirely on local
//!   Steam config files).
//! - `run_files` takes `Option<SteamClient<LoggedIn>>` because the
//!   `--manifest-file` path needs no Steam connection.

pub mod diff;
pub mod download;
pub mod files;
pub mod info;
pub mod local_info;
pub mod manifests;
pub mod packages;
pub mod save_manifest;
pub mod shared;
pub mod workshop;
