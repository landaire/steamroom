# Daemon Mode Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a `--daemon` mode that authenticates once and services subsequent CLI invocations over a local socket, with a queued + priority-aware RPC, attach-by-default client behavior, and a ratatui status TUI.

**Architecture:** A new `daemon/` module under `steamroom-cli/src/` hosts the IPC, protocol, server, client, lifecycle, and TUI. The existing `run_*` command handlers are refactored to take an injected `JobSink` so both direct-mode CLI and the daemon worker share one code path. The wire format is length-prefixed rkyv `Frame`s over `interprocess::local_socket::GenericNamespaced` (same pattern as `hxy`). The daemon launches in the foreground for interactive auth, then re-execs a detached child that reuses the just-saved refresh token to come up without further prompts.

**Tech Stack:** Rust 2024 edition, tokio (existing), `interprocess` (new), `rkyv` v0.8 (new), `ratatui` + `crossterm` (new), `tokio-util` (CancellationToken; new), `nix` (Unix-only, for `setsid`; new). All daemon code lives in `steamroom-cli`.

**Reference spec:** `docs/superpowers/specs/2026-06-02-daemon-mode-design.md`

**Repo conventions:** See `AGENTS.md` at the repo root.
- Use `jj` (not git). Conventional-commit subjects (`feat(daemon):`, `refactor(cli):`, `docs(daemon):`, etc.). Never add `Co-Authored-By:` trailers.
- No LLM-style unicode in code or comments (emdashes, endashes, arrows). ASCII only.
- Newtypes for any value with semantic meaning. Typed errors via `thiserror`. No silent `unwrap_or*` fallbacks.
- Comment only the non-obvious (the why, an invariant, a workaround). Do not restate code.
- After each phase milestone, dispatch a fresh subagent for adversarial code review of the phase's diff before proceeding.

---

## File Structure

```
crates/steamroom-cli/
├── Cargo.toml                   add deps: interprocess, rkyv, ratatui, crossterm, tokio-util, nix
├── src/
│   ├── main.rs                  thin: parse Cli, dispatch direct/daemon-launch/use-daemon
│   ├── cli.rs                   add --daemon, --use-daemon, --priority; Daemon subcommand
│   ├── errors.rs                add daemon-related CliError variants
│   ├── commands/                NEW: run_* handlers extracted from main.rs, refactored to take JobSink
│   │   ├── mod.rs
│   │   ├── info.rs
│   │   ├── manifests.rs
│   │   ├── files.rs
│   │   ├── diff.rs
│   │   ├── packages.rs
│   │   ├── save_manifest.rs
│   │   ├── local_info.rs
│   │   ├── download.rs
│   │   ├── workshop.rs
│   │   └── shared.rs            shared helpers (fetch_manifest, parse_app_kv, fmt_*, etc.)
│   ├── sink.rs                  JobSink trait + StdoutSink impl
│   └── daemon/
│       ├── mod.rs               re-exports + module documentation
│       ├── proto.rs             Request, Response, Event, Frame, *Params, JobId, JobKind, ProgressUpdate
│       ├── framing.rs           async read_frame / write_frame; version handshake; size cap
│       ├── ipc.rs               socket name resolution; bind with liveness probe
│       ├── server.rs            DaemonState, queue, worker, broadcast, connection task, BroadcastSink
│       ├── tracing_layer.rs     job-id-scoped tracing layer that emits Event::Log
│       ├── lifecycle.rs         PID file, launch (fork+setsid+exec), stop
│       ├── client.rs            RPC client: connect, send, attach loop, Ctrl-C handling
│       └── tui.rs               ratatui dashboard
└── tests/
    └── daemon.rs                integration tests with mock SteamClient
```

Tests are inline `#[cfg(test)] mod tests` at the bottom of each `daemon/*` source file, except cross-module integration tests which live under `tests/daemon.rs`.

The `commands/` extraction is part of this work because `main.rs` is already 2075 lines; splitting it is the kind of "improving code you're working in" that the brainstorming skill calls for.

---

## Milestone Review Protocol

After each of phases 1, 2, 3, 4, 5, and 6 completes, dispatch a fresh subagent for an adversarial code review:

```
Agent({
  description: "Adversarial review of phase N",
  subagent_type: "code-reviewer",
  prompt: "<paste the diff for this phase + the spec section it implements>.
           Be skeptical: look for missed edge cases, unsound abstractions,
           swallowed errors, type-system holes, silent unwrap_or defaults,
           drift from the spec. Treat the report as a punch list for me."
})
```

Address punch-list items before starting the next phase.

---

## Phase 1: Wire protocol and framing

Goal: protocol module is fully testable in isolation. No I/O, no daemon. Roundtrip tests cover every variant.

### Task 1: Workspace deps and module scaffolding

**Files:**
- Modify: `Cargo.toml` (workspace root)
- Modify: `crates/steamroom-cli/Cargo.toml`
- Create: `crates/steamroom-cli/src/daemon/mod.rs`
- Create: `crates/steamroom-cli/src/daemon/proto.rs` (stub)
- Create: `crates/steamroom-cli/src/daemon/framing.rs` (stub)
- Create: `crates/steamroom-cli/src/daemon/ipc.rs` (stub)
- Create: `crates/steamroom-cli/src/daemon/server.rs` (stub)
- Create: `crates/steamroom-cli/src/daemon/lifecycle.rs` (stub)
- Create: `crates/steamroom-cli/src/daemon/client.rs` (stub)
- Create: `crates/steamroom-cli/src/daemon/tui.rs` (stub)
- Create: `crates/steamroom-cli/src/daemon/tracing_layer.rs` (stub)
- Modify: `crates/steamroom-cli/src/main.rs` (add `mod daemon;`)

- [ ] **Step 1: Add workspace deps**

Edit the workspace root `Cargo.toml`. Under `[workspace.dependencies]`, add:

```toml
interprocess = { version = "2", features = ["tokio"] }
rkyv = { version = "0.8", features = ["bytecheck"] }
ratatui = "0.29"
crossterm = "0.28"
tokio-util = "0.7"
```

On Unix targets only, also add (this is the workspace declaration; the per-crate dep is below):

```toml
nix = { version = "0.29", features = ["process", "fs"] }
```

- [ ] **Step 2: Add per-crate deps**

Edit `crates/steamroom-cli/Cargo.toml`. Under `[dependencies]`, add:

```toml
interprocess = { workspace = true }
rkyv = { workspace = true }
ratatui = { workspace = true }
crossterm = { workspace = true }
tokio-util = { workspace = true }
```

Then add a target-specific deps section at the bottom of the file:

```toml
[target.'cfg(unix)'.dependencies]
nix = { workspace = true }
```

- [ ] **Step 3: Create stub module files**

Create `crates/steamroom-cli/src/daemon/proto.rs`:

```rust
// Wire types defined in tasks 2-5.
```

Create `crates/steamroom-cli/src/daemon/framing.rs`:

```rust
// Async length-prefixed rkyv framing defined in task 6.
```

Create the same one-line placeholder in `ipc.rs`, `server.rs`, `lifecycle.rs`, `client.rs`, `tui.rs`, `tracing_layer.rs`. Each notes which task defines its contents.

Create `crates/steamroom-cli/src/daemon/mod.rs`:

```rust
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
```

- [ ] **Step 4: Wire `daemon` into the binary**

Edit `crates/steamroom-cli/src/main.rs`. Below the existing `mod cli;` / `mod download;` / `mod errors;` lines, add:

```rust
mod daemon;
```

- [ ] **Step 5: Verify it builds**

Run: `cargo build -p steamroom-cli`
Expected: clean build, no warnings about the new module.

- [ ] **Step 6: Commit**

```bash
jj describe -m "feat(daemon): scaffold daemon module and add workspace deps"
jj new
```

---

### Task 2: Core newtypes and small enums

**Files:**
- Modify: `crates/steamroom-cli/src/daemon/proto.rs`

- [ ] **Step 1: Write failing tests**

Replace the stub in `crates/steamroom-cli/src/daemon/proto.rs` with:

```rust
//! Wire types for the daemon RPC. Owned, rkyv-archivable; never contain
//! `PathBuf`, `Regex`, or other types that rkyv cannot archive directly.

use rkyv::{Archive, Deserialize, Serialize};

/// Monotonically increasing identifier minted by the daemon. Stable for
/// the daemon's lifetime; zero is reserved as "not a job".
#[derive(Archive, Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[rkyv(derive(Debug))]
pub struct JobId(pub u64);

impl std::fmt::Display for JobId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "#{}", self.0)
    }
}

/// Discriminator for what kind of work a job represents. Used in
/// `StatusSnapshot` rendering and the TUI's queue/active panes.
#[derive(Archive, Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[rkyv(derive(Debug))]
pub enum JobKind {
    Download,
    Info,
    Files,
    Manifests,
    Diff,
    Packages,
    SaveManifest,
    Workshop,
    LocalInfo,
}

/// Output format selector. The clap-derived variant in `cli.rs` is the
/// CLI's source of truth; this is the wire-format mirror. Convert with
/// `From<crate::cli::OutputFormat>` (defined here below).
#[derive(Archive, Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[rkyv(derive(Debug))]
pub enum OutputFormat {
    Table,
    Json,
    Plain,
}

impl From<crate::cli::OutputFormat> for OutputFormat {
    fn from(v: crate::cli::OutputFormat) -> Self {
        match v {
            crate::cli::OutputFormat::Table => Self::Table,
            crate::cli::OutputFormat::Json => Self::Json,
            crate::cli::OutputFormat::Plain => Self::Plain,
        }
    }
}

impl From<OutputFormat> for crate::cli::OutputFormat {
    fn from(v: OutputFormat) -> Self {
        match v {
            OutputFormat::Table => Self::Table,
            OutputFormat::Json => Self::Json,
            OutputFormat::Plain => Self::Plain,
        }
    }
}

/// Mirror of `tracing::Level`. Used so attached clients can run the
/// daemon's tracing events through their own subscriber.
#[derive(Archive, Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[rkyv(derive(Debug))]
pub enum LogLevel { Error, Warn, Info, Debug, Trace }

impl From<tracing::Level> for LogLevel {
    fn from(l: tracing::Level) -> Self {
        match l {
            tracing::Level::ERROR => Self::Error,
            tracing::Level::WARN => Self::Warn,
            tracing::Level::INFO => Self::Info,
            tracing::Level::DEBUG => Self::Debug,
            tracing::Level::TRACE => Self::Trace,
        }
    }
}

/// Per-job progress snapshot, emitted by the worker as `Event::Progress`.
#[derive(Archive, Serialize, Deserialize, Debug, Clone)]
#[rkyv(derive(Debug))]
pub struct ProgressUpdate {
    pub bytes_done: u64,
    pub bytes_total: u64,
    pub files_done: u32,
    pub files_total: u32,
    pub rate_bytes_per_sec: u64,
    pub eta_seconds: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn job_id_displays_with_hash_prefix() {
        assert_eq!(format!("{}", JobId(42)), "#42");
    }

    #[test]
    fn output_format_round_trips_through_cli_enum() {
        for w in [OutputFormat::Table, OutputFormat::Json, OutputFormat::Plain] {
            let cli: crate::cli::OutputFormat = w.into();
            let back: OutputFormat = cli.into();
            assert_eq!(w, back);
        }
    }

    #[test]
    fn log_level_maps_from_tracing() {
        assert_eq!(LogLevel::from(tracing::Level::ERROR), LogLevel::Error);
        assert_eq!(LogLevel::from(tracing::Level::TRACE), LogLevel::Trace);
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p steamroom-cli --lib daemon::proto`
Expected: 3 passing tests.

- [ ] **Step 3: Commit**

```bash
jj describe -m "feat(daemon): proto newtypes (JobId, JobKind, OutputFormat, LogLevel, ProgressUpdate)"
jj new
```

---

### Task 3: Per-command `*Params` types

**Files:**
- Create: `crates/steamroom-cli/src/daemon/proto/params.rs`
- Modify: `crates/steamroom-cli/src/daemon/proto.rs` (turn into a `mod.rs`-style re-exporter, or add a submodule).

Note: prefer the submodule approach. Rename `proto.rs` to `proto/mod.rs` and create `proto/params.rs` alongside.

- [ ] **Step 1: Restructure `proto` as a directory module**

Move the existing `crates/steamroom-cli/src/daemon/proto.rs` to `crates/steamroom-cli/src/daemon/proto/mod.rs`. At the top of `mod.rs`, after the existing module doc, add:

```rust
mod params;
pub use params::*;
```

- [ ] **Step 2: Write failing tests**

Create `crates/steamroom-cli/src/daemon/proto/params.rs`:

```rust
//! Per-command argument shadows of the clap-derived structs in `cli.rs`.
//! These are rkyv-archivable: `PathBuf` becomes `String`, `Regex` becomes
//! a raw pattern string, clap enums become their wire mirrors.

use rkyv::{Archive, Deserialize, Serialize};
use super::OutputFormat;

#[derive(Archive, Serialize, Deserialize, Debug, Clone)]
#[rkyv(derive(Debug))]
pub struct DownloadParams {
    pub app: u32,
    pub depot: Option<u32>,
    pub manifest: Option<u64>,
    pub filelist: Option<String>,
    pub file_regex: Option<String>,
    pub output: Option<String>,
    pub verify: bool,
    pub os: Option<String>,
    pub arch: Option<String>,
    pub language: Option<String>,
    pub login_id: Option<u32>,
    pub all_platforms: bool,
    pub all_architectures: bool,
    pub all_languages: bool,
    pub lancache: bool,
    pub max_downloads: Option<usize>,
    pub branch: Option<String>,
    pub branch_password: Option<String>,
    pub local_keys: bool,
    pub non_atomic: bool,
    pub save_manifests: bool,
    pub bytes: bool,
}

#[derive(Archive, Serialize, Deserialize, Debug, Clone)]
#[rkyv(derive(Debug))]
pub struct InfoParams {
    pub app: u32,
    pub format: Option<OutputFormat>,
    pub os: Option<String>,
    pub show_all: bool,
}

#[derive(Archive, Serialize, Deserialize, Debug, Clone)]
#[rkyv(derive(Debug))]
pub struct FilesParams {
    pub app: Option<u32>,
    pub depot: Option<u32>,
    pub manifest: Option<u64>,
    pub manifest_file: Option<String>,
    pub depot_key: Option<String>,
    pub branch: Option<String>,
    pub branch_password: Option<String>,
    pub os: Option<String>,
    pub format: Option<OutputFormat>,
    pub raw: bool,
    pub bytes: bool,
}

#[derive(Archive, Serialize, Deserialize, Debug, Clone)]
#[rkyv(derive(Debug))]
pub struct ManifestsParams {
    pub app: u32,
    pub branch: Option<String>,
    pub branch_password: Option<String>,
    pub format: Option<OutputFormat>,
}

#[derive(Archive, Serialize, Deserialize, Debug, Clone)]
#[rkyv(derive(Debug))]
pub struct DiffParams {
    pub app: u32,
    pub depot: u32,
    pub from: u64,
    pub to: u64,
    pub branch: Option<String>,
    pub format: Option<OutputFormat>,
}

#[derive(Archive, Serialize, Deserialize, Debug, Clone)]
#[rkyv(derive(Debug))]
pub struct PackagesParams {
    pub packages: Vec<u32>,
    pub format: Option<OutputFormat>,
}

#[derive(Archive, Serialize, Deserialize, Debug, Clone)]
#[rkyv(derive(Debug))]
pub struct SaveManifestParams {
    pub app: u32,
    pub depot: u32,
    pub manifest: Option<u64>,
    pub branch: Option<String>,
    pub output: String,
}

#[derive(Archive, Serialize, Deserialize, Debug, Clone)]
#[rkyv(derive(Debug))]
pub struct WorkshopParams {
    pub app: u32,
    pub item: u64,
    pub output: Option<String>,
}

#[derive(Archive, Serialize, Deserialize, Debug, Clone)]
#[rkyv(derive(Debug))]
pub struct LocalInfoParams {
    pub format: Option<OutputFormat>,
    pub user: Option<String>,
    pub users: bool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use rkyv::rancor;

    fn round_trip<T>(value: &T) -> T
    where
        T: rkyv::Archive
            + for<'a> rkyv::Serialize<rkyv::api::high::HighSerializer<rkyv::util::AlignedVec, rkyv::ser::allocator::ArenaHandle<'a>, rancor::Error>>,
        T::Archived: rkyv::Deserialize<T, rkyv::api::high::HighDeserializer<rancor::Error>>,
    {
        let bytes = rkyv::to_bytes::<rancor::Error>(value).unwrap();
        rkyv::from_bytes::<T, rancor::Error>(&bytes).unwrap()
    }

    #[test]
    fn download_params_round_trip() {
        let p = DownloadParams {
            app: 730,
            depot: Some(731),
            manifest: Some(1234),
            filelist: Some("files.txt".into()),
            file_regex: Some(r"\.dll$".into()),
            output: Some("out".into()),
            verify: true,
            os: None,
            arch: None,
            language: None,
            login_id: None,
            all_platforms: false,
            all_architectures: false,
            all_languages: false,
            lancache: false,
            max_downloads: Some(16),
            branch: Some("public".into()),
            branch_password: None,
            local_keys: false,
            non_atomic: false,
            save_manifests: false,
            bytes: false,
        };
        let back: DownloadParams = round_trip(&p);
        assert_eq!(back.app, 730);
        assert_eq!(back.depot, Some(731));
        assert_eq!(back.file_regex.as_deref(), Some(r"\.dll$"));
        assert_eq!(back.max_downloads, Some(16));
    }

    #[test]
    fn packages_params_round_trip_with_vec() {
        let p = PackagesParams { packages: vec![1, 2, 3], format: Some(OutputFormat::Json) };
        let back: PackagesParams = round_trip(&p);
        assert_eq!(back.packages, vec![1, 2, 3]);
        assert!(matches!(back.format, Some(OutputFormat::Json)));
    }
}
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p steamroom-cli --lib daemon::proto::params`
Expected: 2 passing tests.

- [ ] **Step 4: Commit**

```bash
jj describe -m "feat(daemon): per-command *Params wire types with rkyv round-trip tests"
jj new
```

---

### Task 4: `Request` enum and `Cli::into_rpc_request`

**Files:**
- Modify: `crates/steamroom-cli/src/daemon/proto/mod.rs`
- Create: `crates/steamroom-cli/src/daemon/proto/request.rs`
- Modify: `crates/steamroom-cli/src/errors.rs`

- [ ] **Step 1: Add new CliError variants**

Edit `crates/steamroom-cli/src/errors.rs`. Add to the `CliError` enum (anywhere before the closing brace):

```rust
    #[error("--use-daemon: {0} are not supported via the daemon; pass them to --daemon at launch instead")]
    DaemonRejectedFlag(&'static str),

    #[error("--priority is only valid with --use-daemon")]
    PriorityWithoutDaemon,

    #[error("--daemon and --use-daemon are mutually exclusive")]
    DaemonModeConflict,
```

- [ ] **Step 2: Write failing tests**

Create `crates/steamroom-cli/src/daemon/proto/request.rs`:

```rust
use rkyv::{Archive, Deserialize, Serialize};
use super::params::*;
use super::JobId;

#[derive(Archive, Serialize, Deserialize, Debug, Clone)]
#[rkyv(derive(Debug))]
pub enum Request {
    Download { args: DownloadParams, priority: bool },
    Info { args: InfoParams, priority: bool },
    Files { args: FilesParams, priority: bool },
    Manifests { args: ManifestsParams, priority: bool },
    Diff { args: DiffParams, priority: bool },
    Packages { args: PackagesParams, priority: bool },
    SaveManifest { args: SaveManifestParams, priority: bool },
    Workshop { args: WorkshopParams, priority: bool },
    LocalInfo { args: LocalInfoParams, priority: bool },

    Status,
    Subscribe,
    Attach { job_id: JobId },
    Cancel { job_id: JobId },
    TogglePriority { job_id: JobId },
    Stop { force: bool },
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::OutputFormat;
    use rkyv::rancor;

    #[test]
    fn info_request_round_trips() {
        let req = Request::Info {
            args: InfoParams { app: 480, format: Some(OutputFormat::Json), os: None, show_all: false },
            priority: true,
        };
        let bytes = rkyv::to_bytes::<rancor::Error>(&req).unwrap();
        let back = rkyv::from_bytes::<Request, rancor::Error>(&bytes).unwrap();
        match back {
            Request::Info { args, priority } => {
                assert_eq!(args.app, 480);
                assert!(priority);
            }
            other => panic!("wrong variant: {other:?}"),
        }
    }

    #[test]
    fn control_requests_round_trip() {
        for req in [Request::Status, Request::Subscribe, Request::Stop { force: true }] {
            let bytes = rkyv::to_bytes::<rancor::Error>(&req).unwrap();
            let _back = rkyv::from_bytes::<Request, rancor::Error>(&bytes).unwrap();
        }
    }
}
```

Add to `crates/steamroom-cli/src/daemon/proto/mod.rs`:

```rust
mod request;
pub use request::Request;
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p steamroom-cli --lib daemon::proto::request`
Expected: 2 passing tests.

- [ ] **Step 4: Add `Cli::into_rpc_request`**

Append to `crates/steamroom-cli/src/cli.rs` (at the very bottom):

```rust
use crate::daemon::proto::{
    DiffParams, DownloadParams, FilesParams, InfoParams, LocalInfoParams,
    ManifestsParams, PackagesParams, Request, SaveManifestParams,
    WorkshopParams,
};
use crate::errors::CliError;

impl Cli {
    /// Lower a parsed `Cli` into the wire-typed `Request`. Validates that
    /// daemon-mode constraints hold (per-request auth flags rejected,
    /// `--priority` only with `--use-daemon`, etc.).
    pub fn into_rpc_request(self) -> Result<Request, CliError> {
        // Per-request auth flags are bound to the daemon at launch time;
        // any non-default value here is a user error.
        let auth = &self.auth;
        let has_auth = auth.username.is_some()
            || auth.password.is_some()
            || auth.qr
            || auth.use_steam_token
            || auth.remember_password
            || auth.device_name.is_some();
        if has_auth {
            return Err(CliError::DaemonRejectedFlag("auth flags"));
        }
        if self.capture.is_some() {
            return Err(CliError::DaemonRejectedFlag("--capture"));
        }

        let priority = self.priority;
        match self.command {
            Command::Download(a) => Ok(Request::Download { args: DownloadParams::from(a), priority }),
            Command::Info(a) => Ok(Request::Info { args: InfoParams::from(a), priority }),
            Command::Files(a) => Ok(Request::Files { args: FilesParams::from(a), priority }),
            Command::Manifests(a) => Ok(Request::Manifests { args: ManifestsParams::from(a), priority }),
            Command::Diff(a) => Ok(Request::Diff { args: DiffParams::from(a), priority }),
            Command::Packages(a) => Ok(Request::Packages { args: PackagesParams::from(a), priority }),
            Command::SaveManifest(a) => Ok(Request::SaveManifest { args: SaveManifestParams::from(a), priority }),
            Command::Workshop(a) => Ok(Request::Workshop { args: WorkshopParams::from(a), priority }),
            Command::LocalInfo(a) => Ok(Request::LocalInfo { args: LocalInfoParams::from(a), priority }),
            Command::Daemon(_) => Err(CliError::DaemonRejectedFlag("daemon subcommand")),
        }
    }
}
```

(`Command::Daemon` and `Cli::priority` will be added in task 18; until then the match needs to be `match self.command { ... }` without `Daemon`. The `has_auth` / `--capture` validation can be left as is; the `priority` field comes from `Cli` once added.)

For now, since `Cli::priority` and `Command::Daemon` don't exist yet, **stub this**: leave the impl body as `todo!("Cli::into_rpc_request: completed in task 18")` and finish it once the CLI flags exist. Still add the deps for it to compile. The tests in this task only exercise `Request` round-tripping.

- [ ] **Step 5: Add `From<DownloadArgs> for DownloadParams` etc.**

Append to `crates/steamroom-cli/src/daemon/proto/params.rs`, after the type defs:

```rust
fn pathbuf_to_string(p: std::path::PathBuf) -> String {
    p.to_string_lossy().into_owned()
}

impl From<crate::cli::DownloadArgs> for DownloadParams {
    fn from(a: crate::cli::DownloadArgs) -> Self {
        Self {
            app: a.app,
            depot: a.depot,
            manifest: a.manifest,
            filelist: a.filelist.map(pathbuf_to_string),
            file_regex: a.file_regex,
            output: a.output.map(pathbuf_to_string),
            verify: a.verify,
            os: a.os,
            arch: a.arch,
            language: a.language,
            login_id: a.login_id,
            all_platforms: a.all_platforms,
            all_architectures: a.all_architectures,
            all_languages: a.all_languages,
            lancache: a.lancache,
            max_downloads: a.max_downloads,
            branch: a.branch,
            branch_password: a.branch_password,
            local_keys: a.local_keys,
            non_atomic: a.non_atomic,
            save_manifests: a.save_manifests,
            bytes: a.bytes,
        }
    }
}

impl From<crate::cli::InfoArgs> for InfoParams {
    fn from(a: crate::cli::InfoArgs) -> Self {
        Self { app: a.app, format: a.format.map(Into::into), os: a.os, show_all: a.show_all }
    }
}

impl From<crate::cli::FilesArgs> for FilesParams {
    fn from(a: crate::cli::FilesArgs) -> Self {
        Self {
            app: a.app,
            depot: a.depot,
            manifest: a.manifest,
            manifest_file: a.manifest_file.map(pathbuf_to_string),
            depot_key: a.depot_key,
            branch: a.branch,
            branch_password: a.branch_password,
            os: a.os,
            format: a.format.map(Into::into),
            raw: a.raw,
            bytes: a.bytes,
        }
    }
}

impl From<crate::cli::ManifestsArgs> for ManifestsParams {
    fn from(a: crate::cli::ManifestsArgs) -> Self {
        Self {
            app: a.app,
            branch: a.branch,
            branch_password: a.branch_password,
            format: a.format.map(Into::into),
        }
    }
}

impl From<crate::cli::DiffArgs> for DiffParams {
    fn from(a: crate::cli::DiffArgs) -> Self {
        Self {
            app: a.app,
            depot: a.depot,
            from: a.from,
            to: a.to,
            branch: a.branch,
            format: a.format.map(Into::into),
        }
    }
}

impl From<crate::cli::PackagesArgs> for PackagesParams {
    fn from(a: crate::cli::PackagesArgs) -> Self {
        Self { packages: a.packages, format: a.format.map(Into::into) }
    }
}

impl From<crate::cli::SaveManifestArgs> for SaveManifestParams {
    fn from(a: crate::cli::SaveManifestArgs) -> Self {
        Self {
            app: a.app,
            depot: a.depot,
            manifest: a.manifest,
            branch: a.branch,
            output: pathbuf_to_string(a.output),
        }
    }
}

impl From<crate::cli::WorkshopArgs> for WorkshopParams {
    fn from(a: crate::cli::WorkshopArgs) -> Self {
        Self { app: a.app, item: a.item, output: a.output.map(pathbuf_to_string) }
    }
}

impl From<crate::cli::LocalInfoArgs> for LocalInfoParams {
    fn from(a: crate::cli::LocalInfoArgs) -> Self {
        Self { format: a.format.map(Into::into), user: a.user, users: a.users }
    }
}
```

- [ ] **Step 6: Build**

Run: `cargo build -p steamroom-cli`
Expected: clean. (The `into_rpc_request` body is `todo!()` so won't be exercised at runtime.)

- [ ] **Step 7: Commit**

```bash
jj describe -m "feat(daemon): Request enum and From<ClapArgs> for *Params"
jj new
```

---

### Task 5: `Response`, `Event`, `StatusSnapshot`, `Frame`

**Files:**
- Create: `crates/steamroom-cli/src/daemon/proto/response.rs`
- Create: `crates/steamroom-cli/src/daemon/proto/event.rs`
- Create: `crates/steamroom-cli/src/daemon/proto/status.rs`
- Create: `crates/steamroom-cli/src/daemon/proto/frame.rs`
- Modify: `crates/steamroom-cli/src/daemon/proto/mod.rs`

- [ ] **Step 1: Write `Response`**

Create `crates/steamroom-cli/src/daemon/proto/response.rs`:

```rust
use rkyv::{Archive, Deserialize, Serialize};
use super::status::StatusSnapshot;
use super::JobId;

#[derive(Archive, Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
#[rkyv(derive(Debug))]
pub enum ErrorKind {
    ProtocolMismatch,
    InvalidRequest,
    DaemonBusy,
    JobNotFound,
    InternalError,
}

#[derive(Archive, Serialize, Deserialize, Debug, Clone)]
#[rkyv(derive(Debug))]
pub enum Response {
    /// Job submission accepted. `position` is its queue index.
    JobAccepted { job_id: JobId, position: u32 },
    /// One-shot snapshot in reply to `Request::Status`.
    Status(StatusSnapshot),
    /// Reply to `Request::Stop`; daemon is shutting down.
    Stopping,
    /// Control RPC succeeded but produces no payload (`Cancel`, `TogglePriority`).
    Ack,
    /// Typed error reply. `kind` is for programmatic handling; `message` is for display.
    Error { kind: ErrorKind, message: String },
}
```

- [ ] **Step 2: Write `Event`**

Create `crates/steamroom-cli/src/daemon/proto/event.rs`:

```rust
use rkyv::{Archive, Deserialize, Serialize};
use super::status::StatusSnapshot;
use super::{JobId, JobKind, LogLevel, ProgressUpdate};

#[derive(Archive, Serialize, Deserialize, Debug, Clone)]
#[rkyv(derive(Debug))]
pub enum Event {
    JobStarted { job_id: JobId, kind: JobKind, args_summary: String },
    JobFinished { job_id: JobId, exit_code: i32 },
    Log { job_id: Option<JobId>, level: LogLevel, target: String, message: String },
    Progress { job_id: JobId, update: ProgressUpdate },
    Stdout { job_id: JobId, line: String },
    QueueChanged { snapshot: StatusSnapshot },
}

impl Event {
    /// `Some(job_id)` for events scoped to a specific job, `None` for
    /// daemon-wide events (`Log { job_id: None, .. }`, `QueueChanged`).
    pub fn job_id(&self) -> Option<JobId> {
        match self {
            Event::JobStarted { job_id, .. }
            | Event::JobFinished { job_id, .. }
            | Event::Progress { job_id, .. }
            | Event::Stdout { job_id, .. } => Some(*job_id),
            Event::Log { job_id, .. } => *job_id,
            Event::QueueChanged { .. } => None,
        }
    }
}
```

- [ ] **Step 3: Write `StatusSnapshot` and `JobRecord`**

Create `crates/steamroom-cli/src/daemon/proto/status.rs`:

```rust
use rkyv::{Archive, Deserialize, Serialize};
use super::{JobId, JobKind, ProgressUpdate};

#[derive(Archive, Serialize, Deserialize, Debug, Clone)]
#[rkyv(derive(Debug))]
pub struct JobRecord {
    pub job_id: JobId,
    pub kind: JobKind,
    pub args_summary: String,
    pub priority: bool,
    pub submitted_at: u64,
    pub started_at: Option<u64>,
    pub finished_at: Option<u64>,
    pub exit_code: Option<i32>,
    pub progress: Option<ProgressUpdate>,
}

#[derive(Archive, Serialize, Deserialize, Debug, Clone)]
#[rkyv(derive(Debug))]
pub struct StatusSnapshot {
    pub daemon_pid: u32,
    pub daemon_started_at: u64,
    pub account: Option<String>,
    pub active: Option<JobRecord>,
    pub queue: Vec<JobRecord>,
    pub recent: Vec<JobRecord>,
}
```

- [ ] **Step 4: Write `Frame`**

Create `crates/steamroom-cli/src/daemon/proto/frame.rs`:

```rust
use rkyv::{Archive, Deserialize, Serialize};
use super::{Event, Request, Response};

/// The single rkyv-archived top-level type that flows over the socket.
/// Length-prefixed framing wraps these (see `daemon::framing`).
#[derive(Archive, Serialize, Deserialize, Debug, Clone)]
#[rkyv(derive(Debug))]
pub enum Frame {
    Request(Request),
    Response(Response),
    Event(Event),
    /// Closes a streaming reply. `exit_code` is the process-style exit
    /// code the attached CLI should propagate to its own caller.
    EndOfStream { exit_code: i32 },
}

/// Wire-format version. Bump on any breaking change to a `*Params`,
/// `Request`, `Response`, or `Event` variant layout. Receivers check this
/// before deserializing and close the connection on mismatch.
pub const PROTO_VERSION: u16 = 1;
```

- [ ] **Step 5: Wire into `proto/mod.rs`**

Update `crates/steamroom-cli/src/daemon/proto/mod.rs` so the full module list is:

```rust
mod event;
mod frame;
mod params;
mod request;
mod response;
mod status;

pub use event::Event;
pub use frame::{Frame, PROTO_VERSION};
pub use params::*;
pub use request::Request;
pub use response::{ErrorKind, Response};
pub use status::{JobRecord, StatusSnapshot};

// (keep existing JobId/JobKind/OutputFormat/LogLevel/ProgressUpdate
//  definitions from task 2 in this file)
```

- [ ] **Step 6: Write round-trip tests for Frame**

Append to the existing `#[cfg(test)] mod tests` block in `crates/steamroom-cli/src/daemon/proto/mod.rs`:

```rust
    #[test]
    fn frame_round_trips_response_jobaccepted() {
        let f = Frame::Response(Response::JobAccepted { job_id: JobId(7), position: 0 });
        let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&f).unwrap();
        let back = rkyv::from_bytes::<Frame, rkyv::rancor::Error>(&bytes).unwrap();
        match back {
            Frame::Response(Response::JobAccepted { job_id, position }) => {
                assert_eq!(job_id, JobId(7));
                assert_eq!(position, 0);
            }
            other => panic!("wrong frame: {other:?}"),
        }
    }

    #[test]
    fn frame_round_trips_event_log() {
        let f = Frame::Event(Event::Log {
            job_id: Some(JobId(3)),
            level: LogLevel::Warn,
            target: "steamroom_cli".into(),
            message: "stale".into(),
        });
        let bytes = rkyv::to_bytes::<rkyv::rancor::Error>(&f).unwrap();
        let _back = rkyv::from_bytes::<Frame, rkyv::rancor::Error>(&bytes).unwrap();
    }

    #[test]
    fn event_job_id_routes_correctly() {
        let scoped = Event::Stdout { job_id: JobId(9), line: "x".into() };
        assert_eq!(scoped.job_id(), Some(JobId(9)));
        let qc = Event::QueueChanged { snapshot: StatusSnapshot {
            daemon_pid: 1, daemon_started_at: 0, account: None,
            active: None, queue: vec![], recent: vec![],
        }};
        assert_eq!(qc.job_id(), None);
    }
```

Add the missing imports at the top of the test block: `use super::*;` already covers it.

- [ ] **Step 7: Run tests**

Run: `cargo test -p steamroom-cli --lib daemon::proto`
Expected: all previous tests still pass, plus 3 new Frame/Event tests.

- [ ] **Step 8: Commit**

```bash
jj describe -m "feat(daemon): Response/Event/StatusSnapshot/Frame wire types"
jj new
```

---

### Task 6: Async framing with version handshake

**Files:**
- Modify: `crates/steamroom-cli/src/daemon/framing.rs`
- Modify: `crates/steamroom-cli/src/errors.rs`

- [ ] **Step 1: Add framing-specific errors**

Add to `CliError` in `crates/steamroom-cli/src/errors.rs`:

```rust
    #[error("daemon RPC: incompatible wire-protocol version (peer={peer}, ours={ours}); restart the daemon")]
    ProtocolVersionMismatch { peer: u16, ours: u16 },

    #[error("daemon RPC: frame exceeds {limit_bytes} byte cap (got {len_bytes})")]
    FrameTooLarge { len_bytes: u32, limit_bytes: u32 },

    #[error("daemon RPC: malformed frame: {0}")]
    MalformedFrame(String),

    #[error("daemon RPC: socket closed before frame complete")]
    SocketClosed,
```

- [ ] **Step 2: Write the framing module**

Replace `crates/steamroom-cli/src/daemon/framing.rs` with:

```rust
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

pub async fn write_frame<W>(w: &mut W, frame: &Frame) -> Result<(), CliError>
where
    W: AsyncWriteExt + Unpin,
{
    let bytes = rkyv::to_bytes::<rancor::Error>(frame)
        .map_err(|e| CliError::MalformedFrame(e.to_string()))?;
    let len: u32 = u32::try_from(bytes.len())
        .map_err(|_| CliError::FrameTooLarge {
            len_bytes: u32::MAX,
            limit_bytes: MAX_FRAME_BYTES,
        })?;
    if len > MAX_FRAME_BYTES {
        return Err(CliError::FrameTooLarge { len_bytes: len, limit_bytes: MAX_FRAME_BYTES });
    }
    w.write_all(&PROTO_VERSION.to_le_bytes()).await.map_err(CliError::Io)?;
    w.write_all(&len.to_le_bytes()).await.map_err(CliError::Io)?;
    w.write_all(&bytes).await.map_err(CliError::Io)?;
    w.flush().await.map_err(CliError::Io)?;
    Ok(())
}

pub async fn read_frame<R>(r: &mut R) -> Result<Frame, CliError>
where
    R: AsyncReadExt + Unpin,
{
    let mut ver_buf = [0u8; 2];
    match r.read_exact(&mut ver_buf).await {
        Ok(_) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Err(CliError::SocketClosed),
        Err(e) => return Err(CliError::Io(e)),
    }
    let peer = u16::from_le_bytes(ver_buf);
    if peer != PROTO_VERSION {
        return Err(CliError::ProtocolVersionMismatch { peer, ours: PROTO_VERSION });
    }

    let mut len_buf = [0u8; 4];
    r.read_exact(&mut len_buf).await.map_err(CliError::Io)?;
    let len = u32::from_le_bytes(len_buf);
    if len > MAX_FRAME_BYTES {
        return Err(CliError::FrameTooLarge { len_bytes: len, limit_bytes: MAX_FRAME_BYTES });
    }

    // rkyv's checked access demands 16-byte alignment.
    let mut buf = AlignedVec::<16>::with_capacity(len as usize);
    buf.resize(len as usize, 0);
    r.read_exact(&mut buf).await.map_err(CliError::Io)?;
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
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p steamroom-cli --lib daemon::framing`
Expected: 4 passing tests.

- [ ] **Step 4: Commit**

```bash
jj describe -m "feat(daemon): async length-prefixed rkyv framing with version handshake"
jj new
```

---

### Phase 1 milestone

Run the adversarial code-review subagent on the Phase 1 diff (commits since main) using the protocol described in "Milestone Review Protocol" above. Address any findings before starting Phase 2.

---

## Phase 2: `JobSink` and `run_*` refactor

Goal: extract every `run_*` into its own file under `commands/`; route all output through a `JobSink`; direct-mode CLI still behaves identically.

### Task 7: `JobSink` trait and `StdoutSink`

**Files:**
- Create: `crates/steamroom-cli/src/sink.rs`
- Modify: `crates/steamroom-cli/src/main.rs`

- [ ] **Step 1: Define the trait**

Create `crates/steamroom-cli/src/sink.rs`:

```rust
//! Output sink abstraction shared by direct-mode CLI and the daemon
//! worker. Every `run_*` command in `commands/` writes results through a
//! `&dyn JobSink` so the same code paths produce stdout text in direct
//! mode and broadcast events in daemon mode.

use crate::daemon::proto::{LogLevel, ProgressUpdate};

pub trait JobSink: Send + Sync {
    fn stdout_line(&self, line: &str);
    fn progress(&self, update: ProgressUpdate);
    fn log(&self, level: LogLevel, target: &str, message: &str);
}

/// Direct-mode sink: writes to the inherited stdout and to `tracing`.
/// Progress updates are absorbed (direct mode wires the progress bar
/// separately through `download::spawn_progress_renderer`).
pub struct StdoutSink {
    show_progress: bool,
}

impl StdoutSink {
    pub fn new(show_progress: bool) -> Self {
        Self { show_progress }
    }
    pub fn show_progress(&self) -> bool { self.show_progress }
}

impl JobSink for StdoutSink {
    fn stdout_line(&self, line: &str) {
        println!("{line}");
    }
    fn progress(&self, _update: ProgressUpdate) {
        // Direct mode renders progress via the existing event channel;
        // sink-level progress events are a daemon-only concern.
    }
    fn log(&self, level: LogLevel, target: &str, message: &str) {
        match level {
            LogLevel::Error => tracing::error!(target: "from_sink", "{target}: {message}"),
            LogLevel::Warn => tracing::warn!(target: "from_sink", "{target}: {message}"),
            LogLevel::Info => tracing::info!(target: "from_sink", "{target}: {message}"),
            LogLevel::Debug => tracing::debug!(target: "from_sink", "{target}: {message}"),
            LogLevel::Trace => tracing::trace!(target: "from_sink", "{target}: {message}"),
        }
    }
}
```

- [ ] **Step 2: Wire into the binary**

Edit `crates/steamroom-cli/src/main.rs`. Add near the other `mod` declarations:

```rust
mod sink;
```

- [ ] **Step 3: Build**

Run: `cargo build -p steamroom-cli`
Expected: clean.

- [ ] **Step 4: Commit**

```bash
jj describe -m "feat(cli): JobSink trait and StdoutSink for direct-mode output"
jj new
```

---

### Task 8: Extract `commands/` module skeleton and move read-only handlers

**Files:**
- Create: `crates/steamroom-cli/src/commands/mod.rs`
- Create: `crates/steamroom-cli/src/commands/shared.rs`
- Create: `crates/steamroom-cli/src/commands/info.rs`
- Create: `crates/steamroom-cli/src/commands/manifests.rs`
- Create: `crates/steamroom-cli/src/commands/files.rs`
- Create: `crates/steamroom-cli/src/commands/diff.rs`
- Create: `crates/steamroom-cli/src/commands/packages.rs`
- Create: `crates/steamroom-cli/src/commands/save_manifest.rs`
- Create: `crates/steamroom-cli/src/commands/local_info.rs`
- Modify: `crates/steamroom-cli/src/main.rs`

- [ ] **Step 1: Establish the module**

Create `crates/steamroom-cli/src/commands/mod.rs`:

```rust
//! Per-subcommand handlers. Each `run_*` takes an injected `JobSink` and
//! an externally-supplied authenticated `SteamClient<LoggedIn>` so the
//! daemon worker can reuse them across requests without changing the
//! direct-mode CLI's behavior.

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
```

Add `mod commands;` to `crates/steamroom-cli/src/main.rs` near the other `mod` lines.

- [ ] **Step 2: Move shared helpers**

Create `crates/steamroom-cli/src/commands/shared.rs`. Move into it from `main.rs`:
- `parse_app_kv`
- `parse_package_kv`
- `find_first_depot`
- `find_manifest_for_depot`
- `resolve_depot_key`
- `decompress_manifest`
- `fmt_size`
- `fmt_timestamp`
- `fmt_relative`
- `fetch_app_kv`
- `fetch_manifest`
- `kv_to_json`
- `connect_and_login`
- `detect_steam_user`
- `tokens_path`
- `load_saved_token`
- `save_token`
- `forget_saved_token`
- `drive_credentials_flow`
- `drive_qr_flow`
- `guard_prompt`
- `preferred_kind`

Each must be `pub` so the per-command files can call them. Update their internal type references (e.g. `CliError`, `AuthOptions`, `Cli` types) to use `crate::errors::CliError` and `crate::cli::AuthOptions`.

The `INTERACTIVE` `OnceLock` and `is_interactive()` should also move here (made `pub`); `main.rs` will call `commands::shared::is_interactive()`.

After moving, `main.rs` should no longer contain these definitions. Reduce it to: parse `Cli`, init runtime, dispatch to a `commands::run_*` function.

- [ ] **Step 3: Refactor `run_info` into its own file**

Create `crates/steamroom-cli/src/commands/info.rs`. Move the body of the existing `run_info` here, but change the signature:

```rust
use steamroom::apps::AccessToken;
use steamroom::client::{LoggedIn, SteamClient};
use steamroom::depot::AppId;
use steamroom::types::key_value::{KeyValue, KvValue};
use tokio_util::sync::CancellationToken;

use crate::cli::{InfoArgs, OutputFormat};
use crate::commands::shared::*;
use crate::errors::CliError;
use crate::sink::JobSink;

pub async fn run_info(
    args: InfoArgs,
    client: SteamClient<LoggedIn>,
    sink: &dyn JobSink,
    _cancel: CancellationToken,
) -> Result<(), CliError> {
    let app_id = AppId(args.app);
    // ...existing logic, with every `println!(...)` replaced by
    //    sink.stdout_line(&format!(...))
    // and `info!`/`debug!`/`warn!` left as tracing macros (the daemon's
    // tracing layer hooks those up to Event::Log on its end).
}
```

Concretely, every `println!("...")` in `run_info` becomes `sink.stdout_line("...")` (no trailing newline; `stdout_line` writes one). Every `println!()` (empty) becomes `sink.stdout_line("")`. Multi-line table output becomes a loop over `table.lines()` calling `sink.stdout_line(line)`.

Remove the call to `connect_and_login` and `fetch_app_kv` that creates the client; the client is now a parameter. Instead, use:

```rust
let tokens = client.pics_get_access_tokens(&[app_id]).await?;
let token = tokens.into_iter().next().unwrap_or(AccessToken { app_id, token: 0 });
let infos = client.pics_get_product_info(&[token]).await?;
let app_info = infos.into_iter().next().ok_or(CliError::NoProductInfo(app_id.0))?;
let kv_data = app_info.kv_data.ok_or(CliError::NoKvData(app_id.0))?;
let kv = parse_app_kv(&kv_data)?;
```

Note: `unwrap_or` here propagates the existing behavior. Per `AGENTS.md` rule, audit this in a follow-up task; leave a `TODO(no-silent-defaults): tokens.into_iter().next().ok_or(...)` comment so it shows up in review.

- [ ] **Step 4: Repeat for `run_manifests`, `run_files`, `run_diff`, `run_packages`, `run_save_manifest`, `run_local_info`**

Same pattern. Each lives in its own file under `commands/`. Each takes `(args, client: SteamClient<LoggedIn>, sink: &dyn JobSink, cancel: CancellationToken)` and returns `Result<(), CliError>`.

For `run_local_info`, which does not take a Steam client today, change the signature to `(args, sink: &dyn JobSink, _cancel: CancellationToken)` (no client param). Note this special-case in `commands/mod.rs` doc.

For `run_files`, the `--manifest-file` path does not need a client; if `args.manifest_file.is_some()`, the client is unused. To keep the signature uniform we still pass the client; calling code in direct mode authenticates only when needed (handled in task 9).

- [ ] **Step 5: Update `main.rs::async_main` to dispatch**

Replace the body of `async_main` in `main.rs` with:

```rust
async fn async_main(cli: Cli) -> Result<(), CliError> {
    use crate::sink::StdoutSink;
    use tokio_util::sync::CancellationToken;

    let sink = StdoutSink::new(!cli.no_progress);
    let cancel = CancellationToken::new();

    match cli.command {
        Command::LocalInfo(args) => {
            commands::local_info::run_local_info(args, &sink, cancel).await
        }
        Command::Info(args) => {
            let client = commands::shared::connect_and_login(&cli.auth).await?;
            commands::info::run_info(args, client, &sink, cancel).await
        }
        Command::Manifests(args) => {
            let client = commands::shared::connect_and_login(&cli.auth).await?;
            commands::manifests::run_manifests(args, client, &sink, cancel).await
        }
        Command::Files(args) => {
            // Local-only path: no client.
            let client = if args.manifest_file.is_some() {
                None
            } else {
                Some(commands::shared::connect_and_login(&cli.auth).await?)
            };
            commands::files::run_files(args, client, &sink, cancel).await
        }
        Command::Diff(args) => {
            let client = commands::shared::connect_and_login(&cli.auth).await?;
            commands::diff::run_diff(args, client, &sink, cancel).await
        }
        Command::Packages(args) => {
            let client = commands::shared::connect_and_login(&cli.auth).await?;
            commands::packages::run_packages(args, client, &sink, cancel).await
        }
        Command::SaveManifest(args) => {
            let client = commands::shared::connect_and_login(&cli.auth).await?;
            commands::save_manifest::run_save_manifest(args, client, &sink, cancel).await
        }
        Command::Download(args) => {
            let client = commands::shared::connect_and_login(&cli.auth).await?;
            commands::download::run_download(args, client, &sink, cancel).await
        }
        Command::Workshop(args) => {
            let client = commands::shared::connect_and_login(&cli.auth).await?;
            commands::workshop::run_workshop(args, client, &sink, cancel).await
        }
    }
}
```

For `run_files`, change its signature to take `client: Option<SteamClient<LoggedIn>>` (uniformity break, but documented in `commands/mod.rs`).

- [ ] **Step 6: Build and smoke-test against a free app**

Run: `cargo build -p steamroom-cli`
Expected: clean.

Run: `cargo run -p steamroom-cli -- info --app 480 --format plain`
Expected: same output as before the refactor (Spacewar app info).

- [ ] **Step 7: Commit**

```bash
jj describe -m "refactor(cli): extract per-command run_* into commands/ with JobSink"
jj new
```

---

### Task 9: Refactor `run_download` with `JobSink::progress` and `CancellationToken`

**Files:**
- Create: `crates/steamroom-cli/src/commands/download.rs`
- Modify: `crates/steamroom-cli/src/commands/mod.rs` (already done in task 8)
- Modify: `crates/steamroom-cli/src/download.rs` (rename or remove)

- [ ] **Step 1: Move `run_download` and the existing event-renderer**

The existing `crates/steamroom-cli/src/download.rs` (the progress renderer) stays put; it's reused by direct mode. Reference it from `commands/download.rs` as `crate::download`.

Create `crates/steamroom-cli/src/commands/download.rs`. Move the body of `run_download` here. New signature:

```rust
use std::path::PathBuf;
use std::sync::Arc;
use steamroom::client::{LoggedIn, SteamClient};
use steamroom::depot::*;
use tokio_util::sync::CancellationToken;
use tracing::{debug, info, warn};

use crate::cli::DownloadArgs;
use crate::commands::shared::*;
use crate::download as progress_renderer;
use crate::errors::CliError;
use crate::sink::JobSink;

pub async fn run_download(
    args: DownloadArgs,
    client: SteamClient<LoggedIn>,
    sink: &dyn JobSink,
    cancel: CancellationToken,
) -> Result<(), CliError> {
    // ...body of existing run_download, with the following changes:
    //   * Replace info!/warn!/debug! with the same calls; the daemon's
    //     tracing layer routes them to the sink. Direct mode prints via
    //     the standard tracing subscriber.
    //   * The final summary line that was `info!(summary)` stays as
    //     tracing.
    //   * The progress renderer in direct mode reads from the existing
    //     `mpsc::UnboundedReceiver<DownloadEvent>` exactly as today
    //     (sink is not involved). The daemon worker will subscribe to
    //     the same channel in task 11 and translate events.
    //   * Wrap the `job.download(...)` await with a select! on cancel:
}
```

Inside `run_download`, locate the `let stats = job.download(...).await?;` line. Replace it with:

```rust
let download_fut = job.download(&manifest, std::sync::Arc::new(fetcher));
tokio::pin!(download_fut);
let stats = tokio::select! {
    res = &mut download_fut => res.map_err(|e| CliError::Io(std::io::Error::other(e)))?,
    _ = cancel.cancelled() => {
        // Aborting the future drops in-flight chunk tasks. Any spawned
        // tasks are reaped on drop; partial writes are left in place
        // for resume next run (the existing `set_installing` marker
        // signals incomplete state).
        return Err(CliError::Cancelled);
    }
};
```

- [ ] **Step 2: Add `Cancelled` to `CliError`**

In `crates/steamroom-cli/src/errors.rs`, add:

```rust
    #[error("operation cancelled")]
    Cancelled,
```

- [ ] **Step 3: Build and smoke-test**

Run: `cargo build -p steamroom-cli`
Expected: clean.

Run (against a free app to verify direct-mode download still works):

```bash
cargo run -p steamroom-cli -- download --app 480 --depot 481 -o /tmp/spacewar-test
```

Expected: progress bar appears, download completes, no behavior change vs. main.

- [ ] **Step 4: Commit**

```bash
jj describe -m "refactor(cli): run_download accepts CancellationToken; relocate to commands/"
jj new
```

---

### Task 10: Refactor `run_workshop` similarly

**Files:**
- Create: `crates/steamroom-cli/src/commands/workshop.rs`

- [ ] **Step 1: Move the body**

Same pattern as task 9. Wrap the final `.await?` on `job.download(...)` in a `tokio::select!` against `cancel`. Replace `println!`/format usage with `sink.stdout_line` where applicable (workshop mostly uses `info!` already; few direct prints).

- [ ] **Step 2: Build and verify smoke**

Run: `cargo build -p steamroom-cli`
Expected: clean.

(Smoke testing workshop requires a real workshop item; skip unless you have one.)

- [ ] **Step 3: Commit**

```bash
jj describe -m "refactor(cli): relocate run_workshop with CancellationToken"
jj new
```

---

### Phase 2 milestone

Run the adversarial code-review subagent on the Phase 2 diff. Pay particular attention to:
- Any `println!`/`eprintln!` that escaped sink replacement.
- `unwrap_or` / `unwrap_or_else` / `unwrap_or_default` introduced by the move (the existing code has several; flag them but only fix if review consensus says they are wrong).
- Whether `connect_and_login` is still callable from the new locations.

Address findings before Phase 3.

---

## Phase 3: Daemon server core

Goal: an in-memory daemon harness that accepts jobs through a method call (not a socket), runs them through the worker, broadcasts events, and is fully unit-testable.

### Task 11: `DaemonState` skeleton with queue and priority

**Files:**
- Modify: `crates/steamroom-cli/src/daemon/server.rs`

- [ ] **Step 1: Write failing tests**

Replace the stub in `crates/steamroom-cli/src/daemon/server.rs` with:

```rust
//! Daemon-side state, worker loop, and connection task. Decoupled from
//! socket I/O so the queue and dispatch logic can be unit-tested with
//! plain method calls.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::{Mutex, broadcast};
use tokio_util::sync::CancellationToken;

use crate::daemon::proto::{
    Event, JobId, JobKind, JobRecord, ProgressUpdate, Request, StatusSnapshot,
};

/// One unit of work waiting to run. The `request` carries the parameters;
/// `priority` is also reflected here so the queue can rebalance on
/// `TogglePriority` without re-reading the Request.
pub struct QueuedJob {
    pub job_id: JobId,
    pub kind: JobKind,
    pub request: Request,
    pub priority: bool,
    pub submitted_at: u64,
    pub cancel: CancellationToken,
    pub args_summary: String,
}

pub struct RunningJob {
    pub record: JobRecord,
    pub cancel: CancellationToken,
}

pub struct DaemonState {
    pub queue: Mutex<VecDeque<QueuedJob>>,
    pub active: Mutex<Option<RunningJob>>,
    pub recent: Mutex<RingBuffer<JobRecord>>,
    pub events: broadcast::Sender<Event>,
    pub next_job_id: AtomicU64,
    pub shutdown: CancellationToken,
    pub started_at: u64,
    pub daemon_pid: u32,
    pub account: Option<String>,
    pub queue_notify: tokio::sync::Notify,
}

impl DaemonState {
    pub fn new(account: Option<String>, daemon_pid: u32, started_at: u64) -> Arc<Self> {
        // Channel depth: enough to buffer one chunky download's worth of
        // progress events between worker emit and subscriber drain. Lag
        // is tolerated; clients reading slower than the worker writes
        // see `broadcast::error::RecvError::Lagged` and skip ahead.
        let (events, _) = broadcast::channel(512);
        Arc::new(Self {
            queue: Mutex::new(VecDeque::new()),
            active: Mutex::new(None),
            recent: Mutex::new(RingBuffer::new(32)),
            events,
            next_job_id: AtomicU64::new(1),
            shutdown: CancellationToken::new(),
            started_at,
            daemon_pid,
            account,
            queue_notify: tokio::sync::Notify::new(),
        })
    }

    pub fn allocate_job_id(&self) -> JobId {
        JobId(self.next_job_id.fetch_add(1, Ordering::Relaxed))
    }

    /// Insert a job. Priority items sit at the boundary between existing
    /// priority items and non-priority items; non-priority items append.
    /// Returns the queue position the caller should report: 0 if it will
    /// run next (queue empty and no active), 1+ otherwise. The "active"
    /// job is NOT counted because it is already running.
    pub async fn enqueue(&self, job: QueuedJob) -> u32 {
        let mut q = self.queue.lock().await;
        let position = if job.priority {
            let boundary = q.iter().take_while(|j| j.priority).count();
            q.insert(boundary, job);
            boundary as u32
        } else {
            let pos = q.len() as u32;
            q.push_back(job);
            pos
        };
        self.queue_notify.notify_one();
        let _ = self.events.send(Event::QueueChanged { snapshot: self.snapshot_inner(&q, None).await });
        position
    }

    pub async fn toggle_priority(&self, job_id: JobId) -> Result<(), JobNotFound> {
        let mut q = self.queue.lock().await;
        let Some(idx) = q.iter().position(|j| j.job_id == job_id) else {
            return Err(JobNotFound);
        };
        let mut job = q.remove(idx).expect("index just found");
        job.priority = !job.priority;
        if job.priority {
            let boundary = q.iter().take_while(|j| j.priority).count();
            q.insert(boundary, job);
        } else {
            q.push_back(job);
        }
        let _ = self.events.send(Event::QueueChanged { snapshot: self.snapshot_inner(&q, None).await });
        Ok(())
    }

    pub async fn cancel(&self, job_id: JobId) -> Result<(), JobNotFound> {
        // Active first.
        if let Some(running) = self.active.lock().await.as_ref() {
            if running.record.job_id == job_id {
                running.cancel.cancel();
                return Ok(());
            }
        }
        // Queued: remove and emit JobFinished with exit_code 130 (SIGINT-ish).
        let mut q = self.queue.lock().await;
        let Some(idx) = q.iter().position(|j| j.job_id == job_id) else {
            return Err(JobNotFound);
        };
        let removed = q.remove(idx).expect("index just found");
        let _ = self.events.send(Event::JobFinished { job_id: removed.job_id, exit_code: 130 });
        let _ = self.events.send(Event::QueueChanged { snapshot: self.snapshot_inner(&q, None).await });
        Ok(())
    }

    async fn snapshot_inner(
        &self,
        queue: &VecDeque<QueuedJob>,
        active_override: Option<&RunningJob>,
    ) -> StatusSnapshot {
        let active = match active_override {
            Some(r) => Some(r.record.clone()),
            None => self.active.lock().await.as_ref().map(|r| r.record.clone()),
        };
        StatusSnapshot {
            daemon_pid: self.daemon_pid,
            daemon_started_at: self.started_at,
            account: self.account.clone(),
            active,
            queue: queue.iter().map(|j| job_record_for_queued(j)).collect(),
            recent: self.recent.lock().await.iter().cloned().collect(),
        }
    }

    pub async fn snapshot(&self) -> StatusSnapshot {
        let q = self.queue.lock().await;
        self.snapshot_inner(&q, None).await
    }
}

fn job_record_for_queued(j: &QueuedJob) -> JobRecord {
    JobRecord {
        job_id: j.job_id,
        kind: j.kind,
        args_summary: j.args_summary.clone(),
        priority: j.priority,
        submitted_at: j.submitted_at,
        started_at: None,
        finished_at: None,
        exit_code: None,
        progress: None,
    }
}

#[derive(Debug)]
pub struct JobNotFound;

pub struct RingBuffer<T> {
    cap: usize,
    items: VecDeque<T>,
}

impl<T> RingBuffer<T> {
    pub fn new(cap: usize) -> Self { Self { cap, items: VecDeque::with_capacity(cap) } }
    pub fn push(&mut self, v: T) {
        if self.items.len() == self.cap { self.items.pop_front(); }
        self.items.push_back(v);
    }
    pub fn iter(&self) -> impl Iterator<Item = &T> { self.items.iter() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::proto::{InfoParams, OutputFormat};

    fn fake_queued(state: &DaemonState, priority: bool) -> QueuedJob {
        QueuedJob {
            job_id: state.allocate_job_id(),
            kind: JobKind::Info,
            request: Request::Info {
                args: InfoParams { app: 1, format: Some(OutputFormat::Plain), os: None, show_all: false },
                priority,
            },
            priority,
            submitted_at: 0,
            cancel: CancellationToken::new(),
            args_summary: "fake".into(),
        }
    }

    #[tokio::test]
    async fn enqueue_returns_position_zero_for_empty_queue() {
        let s = DaemonState::new(None, 1, 0);
        let pos = s.enqueue(fake_queued(&s, false)).await;
        assert_eq!(pos, 0);
    }

    #[tokio::test]
    async fn priority_jumps_non_priority() {
        let s = DaemonState::new(None, 1, 0);
        let _ = s.enqueue(fake_queued(&s, false)).await; // pos 0
        let _ = s.enqueue(fake_queued(&s, false)).await; // pos 1
        let prio_pos = s.enqueue(fake_queued(&s, true)).await;
        assert_eq!(prio_pos, 0, "first priority should land at the head");

        // Toggle the first non-priority off; verify ordering snapshot.
        let snap = s.snapshot().await;
        let kinds: Vec<bool> = snap.queue.iter().map(|j| j.priority).collect();
        assert_eq!(kinds, vec![true, false, false]);
    }

    #[tokio::test]
    async fn cancel_queued_removes_and_emits_finished() {
        let s = DaemonState::new(None, 1, 0);
        let mut rx = s.events.subscribe();
        let _ = s.enqueue(fake_queued(&s, false)).await;
        let snap = s.snapshot().await;
        let target = snap.queue[0].job_id;
        s.cancel(target).await.expect("ok");
        // QueueChanged from enqueue, then JobFinished, then QueueChanged
        // from cancel. Drain until we see JobFinished.
        let mut saw_finished = false;
        while let Ok(ev) = rx.try_recv() {
            if let Event::JobFinished { job_id, exit_code } = ev {
                assert_eq!(job_id, target);
                assert_eq!(exit_code, 130);
                saw_finished = true;
            }
        }
        assert!(saw_finished, "expected JobFinished after cancel");
    }

    #[tokio::test]
    async fn toggle_priority_moves_across_boundary() {
        let s = DaemonState::new(None, 1, 0);
        let _ = s.enqueue(fake_queued(&s, true)).await;
        let _ = s.enqueue(fake_queued(&s, false)).await;
        let target = s.snapshot().await.queue[1].job_id; // the non-prio one
        s.toggle_priority(target).await.expect("ok");
        let kinds: Vec<bool> = s.snapshot().await.queue.iter().map(|j| j.priority).collect();
        assert_eq!(kinds, vec![true, true]);
    }
}
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p steamroom-cli --lib daemon::server`
Expected: 4 passing tests.

- [ ] **Step 3: Commit**

```bash
jj describe -m "feat(daemon): DaemonState with priority-aware queue and unit tests"
jj new
```

---

### Task 12: `BroadcastSink` and the worker loop

**Files:**
- Modify: `crates/steamroom-cli/src/daemon/server.rs`

- [ ] **Step 1: Append `BroadcastSink` and `worker_loop`**

Append to `crates/steamroom-cli/src/daemon/server.rs`:

```rust
use tokio::sync::broadcast::Sender;
use crate::daemon::proto::LogLevel;
use crate::sink::JobSink;

/// Daemon-side JobSink that translates every call into an `Event` and
/// broadcasts it. Cheap to construct per job.
pub struct BroadcastSink {
    pub job_id: JobId,
    pub events: Sender<Event>,
}

impl JobSink for BroadcastSink {
    fn stdout_line(&self, line: &str) {
        let _ = self.events.send(Event::Stdout { job_id: self.job_id, line: line.to_string() });
    }
    fn progress(&self, update: ProgressUpdate) {
        let _ = self.events.send(Event::Progress { job_id: self.job_id, update });
    }
    fn log(&self, level: LogLevel, target: &str, message: &str) {
        let _ = self.events.send(Event::Log {
            job_id: Some(self.job_id),
            level,
            target: target.to_string(),
            message: message.to_string(),
        });
    }
}

/// Pop the next job (priority first), respecting the shutdown signal.
async fn wait_for_next_job(state: &DaemonState) -> Option<QueuedJob> {
    loop {
        if state.shutdown.is_cancelled() {
            return None;
        }
        {
            let mut q = state.queue.lock().await;
            if let Some(job) = q.pop_front() {
                return Some(job);
            }
        }
        tokio::select! {
            _ = state.queue_notify.notified() => {}
            _ = state.shutdown.cancelled() => return None,
        }
    }
}

/// Single-job worker loop. Owns the authenticated SteamClient and runs
/// it through every `run_*` dispatch.
pub async fn worker_loop(
    state: Arc<DaemonState>,
    client: steamroom::client::SteamClient<steamroom::client::LoggedIn>,
) {
    while let Some(job) = wait_for_next_job(&state).await {
        let started_at = unix_now();
        let sink = BroadcastSink { job_id: job.job_id, events: state.events.clone() };
        let record = JobRecord {
            job_id: job.job_id,
            kind: job.kind,
            args_summary: job.args_summary.clone(),
            priority: job.priority,
            submitted_at: job.submitted_at,
            started_at: Some(started_at),
            finished_at: None,
            exit_code: None,
            progress: None,
        };
        {
            let mut active = state.active.lock().await;
            *active = Some(RunningJob { record: record.clone(), cancel: job.cancel.clone() });
        }
        let _ = state.events.send(Event::JobStarted {
            job_id: job.job_id,
            kind: job.kind,
            args_summary: job.args_summary.clone(),
        });

        let exit_code = dispatch(job.request, client.clone(), &sink, job.cancel.clone()).await;

        {
            let mut active = state.active.lock().await;
            *active = None;
        }
        let mut finished = record;
        finished.finished_at = Some(unix_now());
        finished.exit_code = Some(exit_code);
        state.recent.lock().await.push(finished);
        let _ = state.events.send(Event::JobFinished { job_id: job.job_id, exit_code });
    }
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0) // Pre-1970 system clocks are out of scope.
}

async fn dispatch(
    req: Request,
    client: steamroom::client::SteamClient<steamroom::client::LoggedIn>,
    sink: &dyn JobSink,
    cancel: CancellationToken,
) -> i32 {
    use crate::commands;
    let result = match req {
        Request::Info { args, .. } => {
            let args = crate::cli::InfoArgs {
                app: args.app,
                format: args.format.map(Into::into),
                os: args.os,
                show_all: args.show_all,
            };
            commands::info::run_info(args, client, sink, cancel).await
        }
        Request::Manifests { args, .. } => {
            let args = crate::cli::ManifestsArgs {
                app: args.app,
                branch: args.branch,
                branch_password: args.branch_password,
                format: args.format.map(Into::into),
            };
            commands::manifests::run_manifests(args, client, sink, cancel).await
        }
        // ... similar arms for every Request variant, converting *Params
        // back into the matching clap struct. These are the dual of the
        // From impls added in task 4.
        _ => Ok(()), // placeholder; fully expanded in step 2 below
    };
    match result {
        Ok(()) => 0,
        Err(crate::errors::CliError::Cancelled) => 130,
        Err(_) => 1,
    }
}
```

- [ ] **Step 2: Fill in every `Request` arm in `dispatch`**

Expand every branch. For each variant, build the matching `cli::*Args` from `*Params` (the reverse of task 4's `From` impls; implement these as `impl From<DownloadParams> for crate::cli::DownloadArgs` etc. and keep them in `daemon/proto/params.rs`). Use the `Cancellation`-aware run_* signature from Phase 2.

Special cases:
- `Request::Files` calls `commands::files::run_files(args, Some(client), sink, cancel)` because that handler's signature takes `Option<SteamClient<LoggedIn>>`.
- `Request::LocalInfo` calls `commands::local_info::run_local_info(args, sink, cancel)` without a client.
- Control variants (`Status`, `Subscribe`, `Attach`, `Cancel`, `TogglePriority`, `Stop`) are not dispatched by the worker; they are handled in the connection task (task 14). `dispatch` should `unreachable!()` for those, with a comment.

- [ ] **Step 3: Build**

Run: `cargo build -p steamroom-cli`
Expected: clean.

- [ ] **Step 4: Commit**

```bash
jj describe -m "feat(daemon): BroadcastSink and single-worker dispatch loop"
jj new
```

---

### Task 13: Tracing layer for job-id-scoped log events

**Files:**
- Modify: `crates/steamroom-cli/src/daemon/tracing_layer.rs`

- [ ] **Step 1: Write the layer**

Replace stub with:

```rust
//! `tracing_subscriber::Layer` that intercepts events emitted inside a
//! `job_id`-tagged span and republishes them as `Event::Log` for that
//! job. Off-span events have `job_id: None` and only land in the daemon
//! log file (handled by the wrapping fmt layer).

use std::sync::Arc;
use tokio::sync::broadcast::Sender;
use tracing::{Event as TracingEvent, Subscriber};
use tracing_subscriber::layer::Context;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::Layer;

use crate::daemon::proto::{Event, JobId, LogLevel};

/// Span field name carrying the job id. The worker sets this when
/// entering each job's span: `tracing::info_span!("job", job_id = %job.0)`.
pub const JOB_ID_FIELD: &str = "job_id";

pub struct JobScopedLogLayer {
    pub events: Sender<Event>,
}

impl JobScopedLogLayer {
    pub fn new(events: Sender<Event>) -> Self { Self { events } }
}

impl<S> Layer<S> for JobScopedLogLayer
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &TracingEvent<'_>, ctx: Context<'_, S>) {
        let job_id = find_job_id_in_scope(&ctx);
        let mut visitor = MessageVisitor::default();
        event.record(&mut visitor);
        let message = visitor.message.unwrap_or_default();

        let level: LogLevel = (*event.metadata().level()).into();
        let target = event.metadata().target().to_string();

        let _ = self.events.send(Event::Log {
            job_id: job_id.map(JobId),
            level,
            target,
            message,
        });
    }
}

fn find_job_id_in_scope<S>(ctx: &Context<'_, S>) -> Option<u64>
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    let mut scope = ctx.event_scope()?;
    for span in scope.from_root() {
        let ext = span.extensions();
        if let Some(id) = ext.get::<JobIdAttachment>() {
            return Some(id.0);
        }
    }
    None
}

/// Attached to a span by `on_new_span` when the span's recorded fields
/// include `job_id`. Pure data; no Arc.
struct JobIdAttachment(u64);

impl<S> Layer<S> for JobIdAttachmentInstaller
where
    S: Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(&self, attrs: &tracing::span::Attributes<'_>, id: &tracing::span::Id, ctx: Context<'_, S>) {
        let mut v = JobIdFieldVisitor::default();
        attrs.record(&mut v);
        if let Some(jid) = v.job_id {
            if let Some(span) = ctx.span(id) {
                span.extensions_mut().insert(JobIdAttachment(jid));
            }
        }
    }
}

pub struct JobIdAttachmentInstaller;

#[derive(Default)]
struct JobIdFieldVisitor { job_id: Option<u64> }

impl tracing::field::Visit for JobIdFieldVisitor {
    fn record_debug(&mut self, _: &tracing::field::Field, _: &dyn std::fmt::Debug) {}
    fn record_u64(&mut self, field: &tracing::field::Field, value: u64) {
        if field.name() == JOB_ID_FIELD { self.job_id = Some(value); }
    }
    fn record_i64(&mut self, field: &tracing::field::Field, value: i64) {
        if field.name() == JOB_ID_FIELD && value >= 0 {
            self.job_id = Some(value as u64);
        }
    }
}

#[derive(Default)]
struct MessageVisitor { message: Option<String> }

impl tracing::field::Visit for MessageVisitor {
    fn record_debug(&mut self, field: &tracing::field::Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.message = Some(format!("{value:?}").trim_matches('"').to_string());
        }
    }
    fn record_str(&mut self, field: &tracing::field::Field, value: &str) {
        if field.name() == "message" { self.message = Some(value.to_string()); }
    }
}
```

- [ ] **Step 2: Build**

Run: `cargo build -p steamroom-cli`
Expected: clean.

- [ ] **Step 3: Commit**

```bash
jj describe -m "feat(daemon): tracing layer that scopes log events to job_id"
jj new
```

---

### Task 14: Connection task (in-process; no socket yet)

**Files:**
- Modify: `crates/steamroom-cli/src/daemon/server.rs`

- [ ] **Step 1: Add `handle_connection`**

Append to `server.rs`:

```rust
use tokio::io::{AsyncRead, AsyncWrite};

use crate::daemon::framing::{read_frame, write_frame};
use crate::daemon::proto::{ErrorKind, Frame, Response};

/// Handle a single client connection. Reads exactly one Request, then
/// either replies with a single Response and closes (control RPCs), or
/// streams Events filtered by job id (job submissions, Subscribe, Attach)
/// until terminated by JobFinished or a shutdown signal.
pub async fn handle_connection<S>(state: Arc<DaemonState>, mut stream: S)
where
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    let req = match read_frame(&mut stream).await {
        Ok(Frame::Request(r)) => r,
        Ok(other) => {
            let _ = write_frame(&mut stream, &Frame::Response(Response::Error {
                kind: ErrorKind::InvalidRequest,
                message: format!("expected Request, got {other:?}"),
            })).await;
            return;
        }
        Err(e) => {
            let _ = write_frame(&mut stream, &Frame::Response(Response::Error {
                kind: ErrorKind::InvalidRequest,
                message: e.to_string(),
            })).await;
            return;
        }
    };

    match req {
        Request::Status => {
            let snap = state.snapshot().await;
            let _ = write_frame(&mut stream, &Frame::Response(Response::Status(snap))).await;
        }
        Request::Stop { force } => {
            if force {
                if let Some(running) = state.active.lock().await.as_ref() {
                    running.cancel.cancel();
                }
            }
            state.shutdown.cancel();
            state.queue_notify.notify_one();
            let _ = write_frame(&mut stream, &Frame::Response(Response::Stopping)).await;
        }
        Request::Cancel { job_id } => {
            let resp = match state.cancel(job_id).await {
                Ok(()) => Response::Ack,
                Err(_) => Response::Error { kind: ErrorKind::JobNotFound, message: format!("{job_id}") },
            };
            let _ = write_frame(&mut stream, &Frame::Response(resp)).await;
        }
        Request::TogglePriority { job_id } => {
            let resp = match state.toggle_priority(job_id).await {
                Ok(()) => Response::Ack,
                Err(_) => Response::Error { kind: ErrorKind::JobNotFound, message: format!("{job_id}") },
            };
            let _ = write_frame(&mut stream, &Frame::Response(resp)).await;
        }
        Request::Subscribe => {
            stream_events(state.clone(), &mut stream, None).await;
        }
        Request::Attach { job_id } => {
            // TODO: replay buffer (task 16). For now, stream live until
            // JobFinished for this id.
            stream_events(state.clone(), &mut stream, Some(job_id)).await;
        }
        // Job submissions.
        other => {
            let priority = matches!(&other,
                Request::Download { priority: true, .. }
                | Request::Info { priority: true, .. }
                | Request::Files { priority: true, .. }
                | Request::Manifests { priority: true, .. }
                | Request::Diff { priority: true, .. }
                | Request::Packages { priority: true, .. }
                | Request::SaveManifest { priority: true, .. }
                | Request::Workshop { priority: true, .. }
                | Request::LocalInfo { priority: true, .. });
            let kind = job_kind_of(&other);
            let args_summary = summarize(&other);
            let job_id = state.allocate_job_id();
            let cancel = CancellationToken::new();
            let job = QueuedJob {
                job_id,
                kind,
                request: other,
                priority,
                submitted_at: unix_now(),
                cancel,
                args_summary,
            };
            let position = state.enqueue(job).await;
            let _ = write_frame(&mut stream, &Frame::Response(Response::JobAccepted { job_id, position })).await;
            stream_events(state.clone(), &mut stream, Some(job_id)).await;
        }
    }
}

async fn stream_events<S>(state: Arc<DaemonState>, stream: &mut S, filter: Option<JobId>)
where
    S: AsyncWrite + Unpin,
{
    let mut rx = state.events.subscribe();
    loop {
        tokio::select! {
            _ = state.shutdown.cancelled() => {
                let _ = write_frame(stream, &Frame::EndOfStream { exit_code: 130 }).await;
                return;
            }
            ev = rx.recv() => match ev {
                Ok(ev) => {
                    if let Some(want) = filter {
                        if ev.job_id() != Some(want) { continue; }
                    }
                    let is_terminal = matches!(&ev, Event::JobFinished { .. });
                    let exit_code = if let Event::JobFinished { exit_code, .. } = &ev { *exit_code } else { 0 };
                    if write_frame(stream, &Frame::Event(ev)).await.is_err() {
                        return; // Client dropped; that is fine.
                    }
                    if is_terminal {
                        let _ = write_frame(stream, &Frame::EndOfStream { exit_code }).await;
                        return;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
            }
        }
    }
}

fn job_kind_of(r: &Request) -> JobKind {
    match r {
        Request::Download { .. } => JobKind::Download,
        Request::Info { .. } => JobKind::Info,
        Request::Files { .. } => JobKind::Files,
        Request::Manifests { .. } => JobKind::Manifests,
        Request::Diff { .. } => JobKind::Diff,
        Request::Packages { .. } => JobKind::Packages,
        Request::SaveManifest { .. } => JobKind::SaveManifest,
        Request::Workshop { .. } => JobKind::Workshop,
        Request::LocalInfo { .. } => JobKind::LocalInfo,
        _ => unreachable!("control variants do not produce jobs"),
    }
}

fn summarize(r: &Request) -> String {
    match r {
        Request::Download { args, .. } => format!("download app={} depot={:?}", args.app, args.depot),
        Request::Info { args, .. } => format!("info app={}", args.app),
        Request::Files { args, .. } => format!("files app={:?}", args.app),
        Request::Manifests { args, .. } => format!("manifests app={}", args.app),
        Request::Diff { args, .. } => format!("diff depot={} from={} to={}", args.depot, args.from, args.to),
        Request::Packages { args, .. } => format!("packages count={}", args.packages.len()),
        Request::SaveManifest { args, .. } => format!("save-manifest app={} depot={}", args.app, args.depot),
        Request::Workshop { args, .. } => format!("workshop item={}", args.item),
        Request::LocalInfo { .. } => "local-info".to_string(),
        _ => "(control)".to_string(),
    }
}
```

- [ ] **Step 2: Add an in-process integration test**

Append to the tests module in `server.rs`:

```rust
    use tokio::io::duplex;

    #[tokio::test]
    async fn status_request_round_trips() {
        let s = DaemonState::new(Some("acct".into()), 42, 1000);
        let (mut client, server) = duplex(64 * 1024);
        let server_state = s.clone();
        let server_task = tokio::spawn(async move {
            handle_connection(server_state, server).await;
        });
        write_frame(&mut client, &Frame::Request(Request::Status)).await.unwrap();
        let resp = read_frame(&mut client).await.unwrap();
        match resp {
            Frame::Response(Response::Status(snap)) => {
                assert_eq!(snap.daemon_pid, 42);
                assert_eq!(snap.account.as_deref(), Some("acct"));
            }
            other => panic!("wrong: {other:?}"),
        }
        server_task.await.unwrap();
    }
```

- [ ] **Step 3: Run tests**

Run: `cargo test -p steamroom-cli --lib daemon::server`
Expected: 5 passing tests.

- [ ] **Step 4: Commit**

```bash
jj describe -m "feat(daemon): handle_connection dispatch over async streams"
jj new
```

---

### Phase 3 milestone

Run the adversarial review subagent on Phase 3. Address findings before Phase 4.

---

## Phase 4: IPC, lifecycle, daemon launch

Goal: a real socket-bound daemon you can start, kill, and observe. End-to-end: start daemon, submit an Info job over the wire, see output, stop daemon.

### Task 15: Socket name resolution and bind

**Files:**
- Modify: `crates/steamroom-cli/src/daemon/ipc.rs`

- [ ] **Step 1: Implement the bind helper**

Replace stub with:

```rust
//! Socket name resolution and bind. `interprocess` handles the
//! platform-specific bits; this module just adds the stale-socket probe.

use interprocess::local_socket::{
    GenericNamespaced, ListenerOptions, Name, ToNsName,
    tokio::Stream, tokio::Listener,
    traits::tokio::{Listener as _, Stream as _},
};
use std::time::Duration;

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
        // SAFETY: getuid is always-succeeds on supported targets.
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
        .map_err(|_| CliError::Io(std::io::Error::new(std::io::ErrorKind::TimedOut, "probe timed out")))??;
    match resp {
        Frame::Response(Response::Status(_)) => Ok(()),
        other => Err(CliError::MalformedFrame(format!("probe expected Status, got {other:?}"))),
    }
}

/// Bind the daemon's listener. Returns `Err(CliError::DaemonAlreadyRunning)`
/// if a probe shows a live peer; otherwise overwrites stale sockets.
pub async fn bind_listener() -> Result<Listener, CliError> {
    match probe_peer().await {
        Ok(()) => return Err(CliError::DaemonAlreadyRunning),
        Err(_) => {} // No live peer; safe to overwrite.
    }
    let name = socket_name()?;
    ListenerOptions::new()
        .name(name)
        .reclaim_name(true)
        .create_tokio()
        .map_err(CliError::Io)
}

pub async fn accept(listener: &Listener) -> Result<Stream, CliError> {
    listener.accept().await.map_err(CliError::Io)
}
```

- [ ] **Step 2: Add `DaemonAlreadyRunning` to `CliError`**

In `errors.rs`:

```rust
    #[error("a steamroom daemon is already running on this socket")]
    DaemonAlreadyRunning,
```

- [ ] **Step 3: Add `libc` dep on Unix**

Edit `crates/steamroom-cli/Cargo.toml`. Under `[target.'cfg(unix)'.dependencies]`:

```toml
libc = "0.2"
```

- [ ] **Step 4: Build**

Run: `cargo build -p steamroom-cli`
Expected: clean.

- [ ] **Step 5: Commit**

```bash
jj describe -m "feat(daemon): socket name resolution, liveness probe, listener bind"
jj new
```

---

### Task 16: PID file utilities and `daemon info`

**Files:**
- Modify: `crates/steamroom-cli/src/daemon/lifecycle.rs`

- [ ] **Step 1: Implement PID file helpers**

Replace stub with:

```rust
//! Daemon lifecycle: PID file, launch (Unix double-fork+exec), stop.
//!
//! On Unix the launch sequence is: parse CLI, authenticate in the
//! foreground (so Steam Guard works), save the refresh token via the
//! existing `save_token` path, fork once to escape the controlling
//! terminal, `setsid`, fork again, then `exec` the same binary with a
//! `--daemon-resume` flag. The resumed child rebuilds a fresh tokio
//! runtime, re-authenticates using the saved token (fast, no prompts),
//! binds the socket, and enters the accept loop. The original parent
//! waits on a pipe for the resumed child to report its PID, prints the
//! info block, and exits 0.

use std::path::PathBuf;
use crate::errors::CliError;
use crate::daemon::ipc::socket_name_string;

pub fn pid_file_path() -> PathBuf {
    if let Ok(dir) = std::env::var("XDG_RUNTIME_DIR") {
        return PathBuf::from(dir).join("steamroom.pid");
    }
    let tmp = std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".to_string());
    let uid = unix_uid();
    PathBuf::from(tmp).join(format!("steamroom-{uid}.pid"))
}

#[cfg(unix)]
fn unix_uid() -> u32 { unsafe { libc::getuid() } }
#[cfg(not(unix))]
fn unix_uid() -> u32 { 0 }

pub fn write_pid_file(pid: u32) -> Result<(), CliError> {
    let path = pid_file_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(CliError::Io)?;
    }
    std::fs::write(&path, format!("{pid}\n")).map_err(CliError::Io)
}

pub fn read_pid_file() -> Result<u32, CliError> {
    let data = std::fs::read_to_string(pid_file_path()).map_err(CliError::Io)?;
    data.trim().parse::<u32>().map_err(|e| CliError::MalformedFrame(format!("pid file: {e}")))
}

pub fn remove_pid_file() {
    let _ = std::fs::remove_file(pid_file_path());
}

/// Render the `daemon info` block to stdout. Does NOT contact the
/// daemon. Useful for diagnosing a wedged daemon.
pub fn render_daemon_info() {
    let path = pid_file_path();
    println!("pid file: {}", path.display());
    match read_pid_file() {
        Ok(pid) => println!("pid     : {pid}"),
        Err(_) => println!("pid     : (none; no daemon recorded)"),
    }
    println!("socket  : {}", socket_name_string());
    println!("stop    : steamroom daemon stop");
}
```

- [ ] **Step 2: Build**

Run: `cargo build -p steamroom-cli`
Expected: clean.

- [ ] **Step 3: Commit**

```bash
jj describe -m "feat(daemon): PID file utilities and daemon info renderer"
jj new
```

---

### Task 17: Daemon launch (Unix double-fork + exec)

**Files:**
- Modify: `crates/steamroom-cli/src/daemon/lifecycle.rs`

- [ ] **Step 1: Append the launch logic**

Append to `lifecycle.rs`:

```rust
/// Foreground-then-detach launch on Unix.
///
/// Caller must have already:
///  * Authenticated and saved a refresh token via `save_token`.
///  * Validated CLI args (auth flags, --daemon-only flags, etc.).
///
/// Steps:
///  1. fork() -- parent waits on a pipe for the grandchild's PID, prints
///     the info block, and exits 0.
///  2. Child setsid()s, fork()s again, then the intermediate exits 0.
///  3. Grandchild dup2's stdout/stderr to the log file, then exec's the
///     same binary with `--daemon-resume <username>`. The resumed process
///     rebuilds tokio, re-authenticates with the cached token, binds the
///     socket, writes the PID file, and enters the accept loop.
#[cfg(unix)]
pub fn detach_and_exec_resume(username: &str, log_path: &std::path::Path) -> Result<(), CliError> {
    use nix::unistd::{fork, setsid, ForkResult, dup2, pipe, close, execv, write};
    use std::os::fd::AsRawFd;
    use std::ffi::CString;

    let (read_end, write_end) = pipe().map_err(|e| CliError::Io(std::io::Error::other(e)))?;

    match unsafe { fork().map_err(|e| CliError::Io(std::io::Error::other(e)))? } {
        ForkResult::Parent { child: _ } => {
            close(write_end.as_raw_fd()).ok();
            let mut buf = [0u8; 16];
            let n = nix::unistd::read(read_end.as_raw_fd(), &mut buf)
                .map_err(|e| CliError::Io(std::io::Error::other(e)))?;
            let s = std::str::from_utf8(&buf[..n])
                .map_err(|e| CliError::MalformedFrame(e.to_string()))?;
            let pid: u32 = s.trim().parse()
                .map_err(|e: std::num::ParseIntError| CliError::MalformedFrame(e.to_string()))?;
            println!("steamroom daemon started");
            println!("  pid    : {pid}");
            println!("  socket : {}", socket_name_string());
            println!("  stop   : steamroom daemon stop    (or: kill {pid})");
            println!("  logs   : {}", log_path.display());
            std::process::exit(0);
        }
        ForkResult::Child => {
            close(read_end.as_raw_fd()).ok();
            setsid().map_err(|e| CliError::Io(std::io::Error::other(e)))?;
            match unsafe { fork().map_err(|e| CliError::Io(std::io::Error::other(e)))? } {
                ForkResult::Parent { child: grandchild } => {
                    // Report grandchild PID to the original parent.
                    let pid_str = format!("{}", grandchild.as_raw());
                    write(write_end, pid_str.as_bytes())
                        .map_err(|e| CliError::Io(std::io::Error::other(e)))?;
                    std::process::exit(0);
                }
                ForkResult::Child => {
                    drop(write_end); // close
                    let log = std::fs::OpenOptions::new().create(true).append(true)
                        .open(log_path).map_err(CliError::Io)?;
                    dup2(log.as_raw_fd(), 1).ok();
                    dup2(log.as_raw_fd(), 2).ok();
                    // exec self with resume flag.
                    let exe = std::env::current_exe().map_err(CliError::Io)?;
                    let arg0 = CString::new(exe.as_os_str().as_encoded_bytes()).unwrap();
                    let arg1 = CString::new("--daemon-resume").unwrap();
                    let arg2 = CString::new(username).unwrap();
                    execv(&arg0, &[&arg0, &arg1, &arg2])
                        .map_err(|e| CliError::Io(std::io::Error::other(e)))?;
                    unreachable!("execv either succeeds or fails");
                }
            }
        }
    }
}

#[cfg(not(unix))]
pub fn detach_and_exec_resume(_username: &str, _log_path: &std::path::Path) -> Result<(), CliError> {
    Err(CliError::Io(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "background daemon mode is not yet supported on Windows; run --daemon in the foreground",
    )))
}

pub fn log_path() -> std::path::PathBuf {
    let tmp = std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".to_string());
    std::path::PathBuf::from(tmp).join(format!("steamroom-{}.log", unix_uid()))
}
```

- [ ] **Step 2: Add the `--daemon-resume` resume path**

This task only defines the helpers. The actual `--daemon` and `--daemon-resume` CLI flags + the `serve_daemon` entrypoint that ties everything together are in tasks 18 and 19.

- [ ] **Step 3: Build**

Run: `cargo build -p steamroom-cli`
Expected: clean.

- [ ] **Step 4: Commit**

```bash
jj describe -m "feat(daemon): Unix fork+setsid+exec launch helper"
jj new
```

---

### Task 18: `--daemon` and `--daemon-resume` flags; `serve_daemon`

**Files:**
- Modify: `crates/steamroom-cli/src/cli.rs`
- Modify: `crates/steamroom-cli/src/main.rs`
- Modify: `crates/steamroom-cli/src/daemon/lifecycle.rs`

- [ ] **Step 1: Add CLI flags**

Edit `crates/steamroom-cli/src/cli.rs`. Add to the `Cli` struct:

```rust
    /// Launch a daemon: authenticate, fork, detach, and serve RPC.
    #[arg(long, conflicts_with = "use_daemon")]
    pub daemon: bool,

    /// Send this command to the running daemon instead of executing
    /// directly.
    #[arg(long = "use-daemon")]
    pub use_daemon: bool,

    /// (internal) Resume daemon execution after fork+exec. Not a public
    /// flag; users invoke --daemon, which detaches and re-execs with this.
    #[arg(long, hide = true)]
    pub daemon_resume: Option<String>,

    /// Push this request to the front of the daemon queue. Only valid
    /// with --use-daemon.
    #[arg(long)]
    pub priority: bool,
```

Also add a new variant to the `Command` enum:

```rust
    /// Daemon control commands (stop, status, info, attach).
    Daemon(DaemonArgs),
```

And the args struct:

```rust
#[derive(Parser, Debug)]
pub struct DaemonArgs {
    #[command(subcommand)]
    pub command: DaemonSub,
}

#[derive(Subcommand, Debug)]
pub enum DaemonSub {
    /// Stop the running daemon.
    Stop {
        /// Cancel the active job immediately instead of waiting for it.
        #[arg(long)]
        force: bool,
    },
    /// Print queue, active job, and recent history. Default: TUI dashboard.
    Status {
        /// One-shot text snapshot instead of the TUI.
        #[arg(long)]
        once: bool,
        /// Output format (implies --once when set to json).
        #[arg(long, value_enum)]
        format: Option<OutputFormat>,
    },
    /// Print PID, socket path, and stop command. Does not contact the daemon.
    Info,
    /// Attach to an existing job by id.
    Attach {
        job_id: u64,
    },
}
```

- [ ] **Step 2: Finish `Cli::into_rpc_request`**

Replace the `todo!()` body from task 4 with the implementation. The `priority` is `self.priority`; the `Command::Daemon` arm errors with `CliError::DaemonRejectedFlag("daemon subcommand")`.

Also add the cross-flag validation: `Cli::validate()` that returns `Err(CliError::PriorityWithoutDaemon)` when `priority && !use_daemon`, and `Err(CliError::DaemonModeConflict)` when both `daemon` and `use_daemon` are set (clap's `conflicts_with` handles the latter, so it is belt-and-suspenders).

- [ ] **Step 3: Wire main dispatcher**

In `crates/steamroom-cli/src/main.rs::main()`, after parsing the CLI but before building the tokio runtime, branch:

```rust
if let Some(ref user) = cli.daemon_resume {
    // Re-execed daemon child. Re-authenticate with cached token, bind
    // socket, run accept loop.
    rt.block_on(async { daemon::lifecycle::serve_resumed(user.clone(), cli).await })?;
    return;
}
if cli.daemon {
    // Foreground auth, then fork+exec into resume path.
    rt.block_on(async { daemon::lifecycle::launch_daemon(cli).await })?;
    return;
}
if cli.use_daemon {
    rt.block_on(async { daemon::client::dispatch_use_daemon(cli).await })?;
    return;
}
// otherwise: direct mode.
```

(`daemon::client::dispatch_use_daemon` is defined in Phase 5.)

- [ ] **Step 4: Implement `launch_daemon` and `serve_resumed`**

Append to `lifecycle.rs`:

```rust
use std::sync::Arc;

use crate::cli::Cli;
use crate::commands::shared;
use crate::daemon::ipc;
use crate::daemon::proto::Event;
use crate::daemon::server::{handle_connection, worker_loop, DaemonState};
use crate::daemon::tracing_layer::{JobIdAttachmentInstaller, JobScopedLogLayer};

/// Run before the fork. Authenticates in the foreground, then re-execs
/// the daemon child.
pub async fn launch_daemon(cli: Cli) -> Result<(), CliError> {
    // 1. Authenticate, saving the refresh token via the existing path.
    let client = shared::connect_and_login(&cli.auth).await?;
    let username = cli.auth.username.clone()
        .or_else(|| shared::detect_steam_user().map(|(u, _)| u))
        .ok_or(CliError::InteractiveAuthRequired)?;
    // Close the client so the child can re-establish a fresh connection.
    drop(client);

    // 2. Hand off to the fork+exec helper.
    detach_and_exec_resume(&username, &log_path())
}

/// The actual long-lived daemon process, post-exec. Builds a fresh
/// tokio runtime above this; this just runs the accept loop.
pub async fn serve_resumed(username: String, _cli: Cli) -> Result<(), CliError> {
    // Re-authenticate using the cached token saved by `launch_daemon`.
    let token = shared::load_saved_token(&username)
        .ok_or(CliError::InteractiveAuthRequired)?;
    let client = steamroom_client::login::LoginBuilder::new()
        .device_name("steamroom")
        .with_refresh_token(&username, &token)
        .login().await?;

    let pid = std::process::id();
    write_pid_file(pid)?;
    let state = DaemonState::new(Some(username.clone()), pid, unix_now());

    // Install the tracing layer. The fmt layer continues writing to the
    // log file (already redirected via dup2 in the resume parent).
    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;
    let subscriber = tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(JobIdAttachmentInstaller)
        .with(JobScopedLogLayer::new(state.events.clone()));
    subscriber.try_init().ok();

    // Bind socket.
    let listener = ipc::bind_listener().await?;

    // Spawn the worker.
    let worker_state = state.clone();
    let worker_task = tokio::spawn(async move {
        worker_loop(worker_state, client).await;
    });

    // Accept loop, exits when shutdown is signalled.
    loop {
        tokio::select! {
            _ = state.shutdown.cancelled() => break,
            res = ipc::accept(&listener) => {
                match res {
                    Ok(stream) => {
                        let st = state.clone();
                        tokio::spawn(handle_connection(st, stream));
                    }
                    Err(e) => {
                        tracing::warn!("accept failed: {e}");
                    }
                }
            }
        }
    }

    // Graceful shutdown.
    let _ = state.events.send(Event::Log {
        job_id: None,
        level: crate::daemon::proto::LogLevel::Info,
        target: "daemon".into(),
        message: "shutting down".into(),
    });
    worker_task.abort();
    let _ = worker_task.await;
    remove_pid_file();
    Ok(())
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
```

- [ ] **Step 5: Build**

Run: `cargo build -p steamroom-cli`
Expected: clean. (Connection-side dispatch in `daemon::client` is a stub for now; just check it compiles.)

- [ ] **Step 6: Commit**

```bash
jj describe -m "feat(daemon): --daemon launch, --daemon-resume worker, serve_resumed runtime"
jj new
```

---

### Phase 4 milestone

Run the adversarial review subagent. Particular attention to:
- fork-safety (no tokio threads should be live across the `fork()` call).
- File descriptor leaks (the pipe ends, the log file).
- PID file lifecycle (race between two `--daemon` invocations).
- Error path if `serve_resumed` fails before binding the socket.

Address findings before Phase 5.

---

## Phase 5: Client dispatch and control subcommands

Goal: `--use-daemon` works for every command; `daemon stop`/`info`/`attach`/`status --once`/`--format json` all work over the wire.

### Task 19: `dispatch_use_daemon` and the attach loop

**Files:**
- Modify: `crates/steamroom-cli/src/daemon/client.rs`

- [ ] **Step 1: Implement**

Replace stub with:

```rust
//! Client side of `--use-daemon`: connect, submit, attach to the event
//! stream, render events to stdout via the same formatting the direct
//! CLI uses.

use interprocess::local_socket::tokio::Stream;
use interprocess::local_socket::traits::tokio::Stream as _;

use crate::cli::Cli;
use crate::daemon::framing::{read_frame, write_frame};
use crate::daemon::ipc::socket_name;
use crate::daemon::proto::{Event, Frame, JobId, LogLevel, Request, Response};
use crate::errors::CliError;

pub async fn connect() -> Result<Stream, CliError> {
    let name = socket_name()?;
    Stream::connect(name).await.map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused => CliError::NoDaemonRunning,
        _ => CliError::Io(e),
    })
}

pub async fn dispatch_use_daemon(cli: Cli) -> Result<(), CliError> {
    let detach = cli.detach;
    let request = cli.into_rpc_request()?;
    let mut stream = connect().await?;
    write_frame(&mut stream, &Frame::Request(request)).await?;

    let resp = read_frame(&mut stream).await?;
    let (job_id, position) = match resp {
        Frame::Response(Response::JobAccepted { job_id, position }) => (job_id, position),
        Frame::Response(Response::Error { kind, message }) => {
            return Err(CliError::DaemonError(format!("{kind:?}: {message}")));
        }
        other => return Err(CliError::MalformedFrame(format!("expected JobAccepted, got {other:?}"))),
    };

    if detach {
        println!("job {} queued (position {})", job_id, position);
        return Ok(());
    }

    attach_loop(&mut stream, job_id).await
}

/// Stream events from a connection and render them as the direct CLI
/// would. Returns when EndOfStream arrives (with the carried exit code
/// propagated via std::process::exit) or when Ctrl-C detaches.
pub async fn attach_loop(stream: &mut Stream, job_id: JobId) -> Result<(), CliError> {
    let ctrl_c = tokio::signal::ctrl_c();
    tokio::pin!(ctrl_c);

    loop {
        let frame_fut = read_frame(stream);
        tokio::pin!(frame_fut);
        tokio::select! {
            _ = &mut ctrl_c => {
                eprintln!("detached -- reattach with `steamroom daemon attach {}`", job_id);
                return Ok(());
            }
            r = &mut frame_fut => match r {
                Ok(Frame::Event(ev)) => render_event(ev),
                Ok(Frame::EndOfStream { exit_code }) => {
                    if exit_code != 0 {
                        std::process::exit(exit_code);
                    }
                    return Ok(());
                }
                Ok(other) => {
                    return Err(CliError::MalformedFrame(format!("unexpected frame: {other:?}")));
                }
                Err(CliError::SocketClosed) => return Ok(()),
                Err(e) => return Err(e),
            }
        }
    }
}

fn render_event(ev: Event) {
    match ev {
        Event::Stdout { line, .. } => println!("{line}"),
        Event::Log { level, target, message, .. } => emit_log(level, &target, &message),
        Event::Progress { .. } => {
            // Direct rendering of progress requires an indicatif bar; for
            // now we forward to the existing tracing renderer in
            // `crate::download` once a daemon-friendly version exists.
            // Until then, the user sees Log/Stdout entries reporting
            // completion. This is wired up properly in task 23.
        }
        Event::JobStarted { .. } | Event::JobFinished { .. } | Event::QueueChanged { .. } => {}
    }
}

fn emit_log(level: LogLevel, target: &str, message: &str) {
    // Run through tracing so the user's existing filters (--debug, etc.)
    // decide what shows. The target field becomes the event's target.
    match level {
        LogLevel::Error => tracing::error!(target: "daemon", "{target}: {message}"),
        LogLevel::Warn => tracing::warn!(target: "daemon", "{target}: {message}"),
        LogLevel::Info => tracing::info!(target: "daemon", "{target}: {message}"),
        LogLevel::Debug => tracing::debug!(target: "daemon", "{target}: {message}"),
        LogLevel::Trace => tracing::trace!(target: "daemon", "{target}: {message}"),
    }
}
```

- [ ] **Step 2: Add CliError variants**

In `errors.rs`:

```rust
    #[error("no daemon running on this socket; start one with `steamroom --daemon`")]
    NoDaemonRunning,

    #[error("daemon returned error: {0}")]
    DaemonError(String),
```

- [ ] **Step 3: Add `--detach` flag**

In `cli.rs`, on the `Cli` struct:

```rust
    /// Return immediately after the daemon accepts the job, instead of
    /// streaming progress to this terminal. Only valid with --use-daemon.
    #[arg(long)]
    pub detach: bool,
```

- [ ] **Step 4: Build**

Run: `cargo build -p steamroom-cli`
Expected: clean.

- [ ] **Step 5: Smoke test**

Manual: start a daemon (`cargo run -p steamroom-cli -- --daemon --username FOO`), then in another terminal:

```bash
cargo run -p steamroom-cli -- --use-daemon info --app 480 --format plain
```

Expected: the daemon serves the request, output streams back over the socket. Direct execution of `info --app 480 --format plain` (no `--use-daemon`) still works.

- [ ] **Step 6: Commit**

```bash
jj describe -m "feat(daemon): client dispatcher and attach loop with Ctrl-C detach"
jj new
```

---

### Task 20: `daemon stop`, `daemon info`, `daemon attach`, `daemon status --once`

**Files:**
- Modify: `crates/steamroom-cli/src/main.rs`
- Modify: `crates/steamroom-cli/src/daemon/client.rs`

- [ ] **Step 1: Add subcommand handlers**

Append to `daemon/client.rs`:

```rust
use crate::cli::{DaemonSub, OutputFormat as CliOutputFormat};
use crate::daemon::proto::StatusSnapshot;

pub async fn run_daemon_subcommand(sub: DaemonSub) -> Result<(), CliError> {
    match sub {
        DaemonSub::Info => {
            crate::daemon::lifecycle::render_daemon_info();
            Ok(())
        }
        DaemonSub::Stop { force } => stop_daemon(force).await,
        DaemonSub::Attach { job_id } => attach_existing(JobId(job_id)).await,
        DaemonSub::Status { once, format } => {
            if once || format == Some(CliOutputFormat::Json) {
                print_status_once(format).await
            } else {
                crate::daemon::tui::run_tui().await
            }
        }
    }
}

async fn stop_daemon(force: bool) -> Result<(), CliError> {
    let mut stream = connect().await?;
    write_frame(&mut stream, &Frame::Request(Request::Stop { force })).await?;
    let resp = read_frame(&mut stream).await?;
    match resp {
        Frame::Response(Response::Stopping) => {
            if force {
                println!("stopping daemon (cancelling active job)");
            } else {
                println!("stopping daemon (active job will finish)");
            }
            Ok(())
        }
        Frame::Response(Response::Error { kind, message }) => {
            Err(CliError::DaemonError(format!("{kind:?}: {message}")))
        }
        other => Err(CliError::MalformedFrame(format!("expected Stopping, got {other:?}"))),
    }
}

async fn attach_existing(job_id: JobId) -> Result<(), CliError> {
    let mut stream = connect().await?;
    write_frame(&mut stream, &Frame::Request(Request::Attach { job_id })).await?;
    attach_loop(&mut stream, job_id).await
}

async fn print_status_once(format: Option<CliOutputFormat>) -> Result<(), CliError> {
    let mut stream = connect().await?;
    write_frame(&mut stream, &Frame::Request(Request::Status)).await?;
    let resp = read_frame(&mut stream).await?;
    let snap = match resp {
        Frame::Response(Response::Status(s)) => s,
        Frame::Response(Response::Error { kind, message }) => {
            return Err(CliError::DaemonError(format!("{kind:?}: {message}")));
        }
        other => return Err(CliError::MalformedFrame(format!("expected Status, got {other:?}"))),
    };
    match format {
        Some(CliOutputFormat::Json) => print_status_json(&snap),
        _ => print_status_table(&snap),
    }
    Ok(())
}

fn print_status_json(snap: &StatusSnapshot) {
    // Reuse the existing serde_json formatting style from the CLI.
    let json = serde_json::json!({
        "daemon_pid": snap.daemon_pid,
        "daemon_started_at": snap.daemon_started_at,
        "account": snap.account,
        "active": snap.active.as_ref().map(record_to_json),
        "queue": snap.queue.iter().map(record_to_json).collect::<Vec<_>>(),
        "recent": snap.recent.iter().map(record_to_json).collect::<Vec<_>>(),
    });
    println!("{}", serde_json::to_string_pretty(&json).expect("snapshot is JSON-clean"));
}

fn record_to_json(r: &crate::daemon::proto::JobRecord) -> serde_json::Value {
    serde_json::json!({
        "job_id": r.job_id.0,
        "kind": format!("{:?}", r.kind),
        "args_summary": r.args_summary,
        "priority": r.priority,
        "submitted_at": r.submitted_at,
        "started_at": r.started_at,
        "finished_at": r.finished_at,
        "exit_code": r.exit_code,
    })
}

fn print_status_table(snap: &StatusSnapshot) {
    println!("daemon pid : {}", snap.daemon_pid);
    println!("account    : {}", snap.account.as_deref().unwrap_or("(none)"));
    if let Some(active) = &snap.active {
        println!("\nActive:");
        println!("  {} ({}) {}", active.job_id, format!("{:?}", active.kind), active.args_summary);
    } else {
        println!("\nActive: (idle)");
    }
    if !snap.queue.is_empty() {
        println!("\nQueue:");
        for j in &snap.queue {
            let mark = if j.priority { "*" } else { " " };
            println!("  {} {} ({}) {}", mark, j.job_id, format!("{:?}", j.kind), j.args_summary);
        }
    }
    if !snap.recent.is_empty() {
        println!("\nRecent:");
        for j in &snap.recent {
            let ec = j.exit_code.map(|c| format!("exit {c}")).unwrap_or_default();
            println!("  {} ({}) {} {}", j.job_id, format!("{:?}", j.kind), j.args_summary, ec);
        }
    }
}
```

- [ ] **Step 2: Wire subcommand into `main.rs`**

In `async_main`, add the arm for `Command::Daemon`:

```rust
        Command::Daemon(args) => daemon::client::run_daemon_subcommand(args.command).await,
```

- [ ] **Step 3: Build and smoke-test**

Run: `cargo build -p steamroom-cli`
Expected: clean.

With a running daemon, exercise:

```bash
cargo run -p steamroom-cli -- daemon info
cargo run -p steamroom-cli -- daemon status --once
cargo run -p steamroom-cli -- daemon status --once --format json | jq .
cargo run -p steamroom-cli -- daemon stop
```

- [ ] **Step 4: Commit**

```bash
jj describe -m "feat(daemon): daemon subcommand (stop, status --once, info, attach)"
jj new
```

---

### Phase 5 milestone

Run the adversarial review subagent. Address findings before Phase 6.

---

## Phase 6: Ratatui status TUI

Goal: `steamroom daemon status` (default, no `--once`) launches a usable dashboard with the agreed three-pane layout and keybindings.

### Task 21: TUI scaffolding

**Files:**
- Modify: `crates/steamroom-cli/src/daemon/tui.rs`

- [ ] **Step 1: Skeleton**

Replace stub with:

```rust
//! Ratatui status dashboard. Routes input from crossterm and events from
//! a Subscribe RPC connection through a single state machine.

use crossterm::event::{Event as CtEvent, KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use std::io::Stdout;
use std::time::Duration;
use tokio::sync::mpsc;

use crate::daemon::client::connect;
use crate::daemon::framing::{read_frame, write_frame};
use crate::daemon::proto::{
    Event as ProtoEvent, Frame, JobId, JobRecord, Request, Response, StatusSnapshot,
};
use crate::errors::CliError;

pub async fn run_tui() -> Result<(), CliError> {
    let mut terminal = init_terminal()?;
    let result = main_loop(&mut terminal).await;
    restore_terminal(&mut terminal).ok();
    result
}

fn init_terminal() -> Result<Terminal<CrosstermBackend<Stdout>>, CliError> {
    crossterm::terminal::enable_raw_mode().map_err(|e| CliError::Io(std::io::Error::other(e)))?;
    let mut stdout = std::io::stdout();
    crossterm::execute!(stdout,
        crossterm::terminal::EnterAlternateScreen,
        crossterm::event::EnableMouseCapture
    ).map_err(|e| CliError::Io(std::io::Error::other(e)))?;
    let backend = CrosstermBackend::new(stdout);
    Terminal::new(backend).map_err(|e| CliError::Io(std::io::Error::other(e)))
}

fn restore_terminal(t: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<(), CliError> {
    crossterm::terminal::disable_raw_mode().map_err(|e| CliError::Io(std::io::Error::other(e)))?;
    crossterm::execute!(t.backend_mut(),
        crossterm::terminal::LeaveAlternateScreen,
        crossterm::event::DisableMouseCapture
    ).map_err(|e| CliError::Io(std::io::Error::other(e)))?;
    t.show_cursor().ok();
    Ok(())
}

struct TuiState {
    snapshot: StatusSnapshot,
    selected_queue_idx: usize,
    log: std::collections::VecDeque<String>,
    log_cap: usize,
}

impl TuiState {
    fn new(snapshot: StatusSnapshot) -> Self {
        Self { snapshot, selected_queue_idx: 0, log: Default::default(), log_cap: 1000 }
    }
    fn selected_job(&self) -> Option<&JobRecord> {
        self.snapshot.queue.get(self.selected_queue_idx)
    }
    fn push_log(&mut self, line: String) {
        if self.log.len() == self.log_cap { self.log.pop_front(); }
        self.log.push_back(line);
    }
}

async fn main_loop(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<(), CliError> {
    // Seed via Status request, then open Subscribe stream.
    let mut status_stream = connect().await?;
    write_frame(&mut status_stream, &Frame::Request(Request::Status)).await?;
    let snap = match read_frame(&mut status_stream).await? {
        Frame::Response(Response::Status(s)) => s,
        other => return Err(CliError::MalformedFrame(format!("status: {other:?}"))),
    };
    drop(status_stream);

    let mut state = TuiState::new(snap);

    // Spawn a task that streams Subscribe events into a tokio channel.
    let (ev_tx, mut ev_rx) = mpsc::channel::<ProtoEvent>(256);
    let subscribe_task = tokio::spawn(async move {
        let Ok(mut sub) = connect().await else { return; };
        if write_frame(&mut sub, &Frame::Request(Request::Subscribe)).await.is_err() { return; }
        loop {
            match read_frame(&mut sub).await {
                Ok(Frame::Event(ev)) => {
                    if ev_tx.send(ev).await.is_err() { return; }
                }
                Ok(Frame::EndOfStream { .. }) => return,
                Ok(_) => continue,
                Err(_) => return,
            }
        }
    });

    // Spawn a task that reads crossterm key events into a channel.
    let (key_tx, mut key_rx) = mpsc::channel::<CtEvent>(64);
    let keys_task = tokio::task::spawn_blocking(move || {
        loop {
            if crossterm::event::poll(Duration::from_millis(200)).unwrap_or(false) {
                if let Ok(ev) = crossterm::event::read() {
                    if key_tx.blocking_send(ev).is_err() { return; }
                }
            }
        }
    });

    loop {
        draw(terminal, &state)?;
        tokio::select! {
            ev = ev_rx.recv() => match ev {
                Some(e) => apply_event(&mut state, e),
                None => break,
            },
            key = key_rx.recv() => match key {
                Some(CtEvent::Key(k)) => {
                    if handle_key(&mut state, k).await? { break; }
                }
                Some(_) => continue,
                None => break,
            },
        }
    }

    subscribe_task.abort();
    keys_task.abort();
    Ok(())
}

fn apply_event(state: &mut TuiState, ev: ProtoEvent) {
    match ev {
        ProtoEvent::QueueChanged { snapshot } => {
            state.snapshot = snapshot;
            if state.selected_queue_idx >= state.snapshot.queue.len() {
                state.selected_queue_idx = state.snapshot.queue.len().saturating_sub(1);
            }
        }
        ProtoEvent::Progress { update, .. } => {
            if let Some(active) = state.snapshot.active.as_mut() {
                active.progress = Some(update);
            }
        }
        ProtoEvent::Log { level, target, message, .. } => {
            state.push_log(format!("[{level:?}] {target}: {message}"));
        }
        ProtoEvent::Stdout { line, .. } => state.push_log(line),
        ProtoEvent::JobStarted { .. } | ProtoEvent::JobFinished { .. } => {}
    }
}

async fn handle_key(state: &mut TuiState, k: KeyEvent) -> Result<bool, CliError> {
    match (k.code, k.modifiers) {
        (KeyCode::Char('q'), _) => return Ok(true),
        (KeyCode::Char('c'), KeyModifiers::CONTROL) => return Ok(true),
        (KeyCode::Up, _) => state.selected_queue_idx = state.selected_queue_idx.saturating_sub(1),
        (KeyCode::Down, _) => {
            if state.selected_queue_idx + 1 < state.snapshot.queue.len() {
                state.selected_queue_idx += 1;
            }
        }
        (KeyCode::Char('p'), _) => {
            if let Some(j) = state.selected_job() {
                send_one(Request::TogglePriority { job_id: j.job_id }).await?;
            }
        }
        (KeyCode::Char('x'), _) => {
            if let Some(j) = state.selected_job() {
                send_one(Request::Cancel { job_id: j.job_id }).await?;
            }
        }
        _ => {}
    }
    Ok(false)
}

async fn send_one(req: Request) -> Result<(), CliError> {
    let mut s = connect().await?;
    write_frame(&mut s, &Frame::Request(req)).await?;
    let _ = read_frame(&mut s).await?;
    Ok(())
}

fn draw(_terminal: &mut Terminal<CrosstermBackend<Stdout>>, _state: &TuiState) -> Result<(), CliError> {
    // Implemented in task 22.
    Ok(())
}
```

- [ ] **Step 2: Build**

Run: `cargo build -p steamroom-cli`
Expected: clean (no panics yet; `draw` is a no-op).

- [ ] **Step 3: Commit**

```bash
jj describe -m "feat(daemon): TUI scaffolding (event/key channels, state machine)"
jj new
```

---

### Task 22: TUI rendering and keybindings

**Files:**
- Modify: `crates/steamroom-cli/src/daemon/tui.rs`

- [ ] **Step 1: Implement `draw`**

Replace the `draw` stub with a real impl that renders three panes:

```rust
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Gauge, List, ListItem, Paragraph, Wrap};

fn draw(terminal: &mut Terminal<CrosstermBackend<Stdout>>, state: &TuiState) -> Result<(), CliError> {
    terminal.draw(|f| {
        let outer = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(8), Constraint::Length(8), Constraint::Length(1)])
            .split(f.area());
        let top = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
            .split(outer[0]);

        // Queue pane.
        let items: Vec<ListItem> = state.snapshot.queue.iter().enumerate().map(|(i, j)| {
            let star = if j.priority { "* " } else { "  " };
            let prefix = if i == state.selected_queue_idx { "> " } else { "  " };
            ListItem::new(format!("{prefix}{star}{} {:?} {}", j.job_id, j.kind, j.args_summary))
        }).collect();
        let queue = List::new(items)
            .block(Block::default().borders(Borders::ALL).title(format!("queue ({})", state.snapshot.queue.len())));
        f.render_widget(queue, top[0]);

        // Active pane.
        let active_block = Block::default().borders(Borders::ALL).title("active job");
        match &state.snapshot.active {
            Some(j) => {
                let chunks = Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([Constraint::Length(2), Constraint::Length(3), Constraint::Min(0)])
                    .split(active_block.inner(top[1]));
                f.render_widget(active_block.clone(), top[1]);
                let summary = Paragraph::new(format!("{} {:?}\n{}", j.job_id, j.kind, j.args_summary))
                    .wrap(Wrap { trim: true });
                f.render_widget(summary, chunks[0]);

                if let Some(p) = &j.progress {
                    let pct = if p.bytes_total > 0 {
                        (p.bytes_done as f64 / p.bytes_total as f64 * 100.0) as u16
                    } else { 0 };
                    let gauge = Gauge::default()
                        .gauge_style(Style::default().fg(Color::Cyan))
                        .percent(pct)
                        .label(format!("{}/{} {}/s ETA {}s",
                            human_bytes(p.bytes_done), human_bytes(p.bytes_total),
                            human_bytes(p.rate_bytes_per_sec), p.eta_seconds));
                    f.render_widget(gauge, chunks[1]);
                }
            }
            None => {
                let p = Paragraph::new("idle").block(active_block);
                f.render_widget(p, top[1]);
            }
        }

        // Log pane.
        let log_lines: Vec<Line> = state.log.iter().rev().take(outer[1].height as usize).rev()
            .map(|s| Line::from(Span::raw(s.clone()))).collect();
        let log = Paragraph::new(log_lines)
            .block(Block::default().borders(Borders::ALL).title("log"))
            .wrap(Wrap { trim: false });
        f.render_widget(log, outer[1]);

        // Footer.
        let footer = Paragraph::new("q quit   up/down select   p toggle priority   x cancel")
            .style(Style::default().add_modifier(Modifier::DIM));
        f.render_widget(footer, outer[2]);
    }).map_err(|e| CliError::Io(std::io::Error::other(e)))?;
    Ok(())
}

fn human_bytes(b: u64) -> String {
    if b >= 1 << 30 { format!("{:.2} GiB", b as f64 / (1u64 << 30) as f64) }
    else if b >= 1 << 20 { format!("{:.2} MiB", b as f64 / (1u64 << 20) as f64) }
    else if b >= 1 << 10 { format!("{:.2} KiB", b as f64 / (1u64 << 10) as f64) }
    else { format!("{b} B") }
}
```

- [ ] **Step 2: Build and smoke-test**

Run: `cargo build -p steamroom-cli`
Expected: clean.

With a running daemon and a submitted job in another terminal:

```bash
cargo run -p steamroom-cli -- daemon status
```

Expected: TUI appears, queue + active + log panes render; pressing `q` exits cleanly with no leftover terminal corruption.

- [ ] **Step 3: Commit**

```bash
jj describe -m "feat(daemon): ratatui dashboard with queue, active, log panes"
jj new
```

---

### Phase 6 milestone

Run the adversarial review subagent. Address findings before Phase 7.

---

## Phase 7: Tests, docs, polish

### Task 23: Wire progress bar into the daemon attach path

**Files:**
- Modify: `crates/steamroom-cli/src/daemon/client.rs`
- Modify: `crates/steamroom-cli/src/commands/download.rs` (sink-emit progress)

- [ ] **Step 1: Make `commands::download::run_download` emit progress via the sink**

In `run_download`, alongside the existing `event_tx`/`event_rx` mpsc, add a forwarder that translates each `DownloadEvent::ChunkCompleted` into `sink.progress(ProgressUpdate { ... })`. Keep the existing direct-mode progress renderer attached to the same channel; the sink path is additive and only meaningful in daemon mode.

```rust
// after `let (event_tx, event_rx) = mpsc::unbounded_channel();`
let sink_for_forwarder = sink as &dyn JobSink; // already what we have
// Spawn forwarder. Use a clone of the receiver only if needed; the
// existing progress renderer takes ownership, so split the events via
// a broadcast or fan-out. Simplest: have the progress renderer also
// drive the sink directly. Modify `crate::download::spawn_progress_renderer`
// to take an optional `&dyn JobSink`.
```

Defer the actual implementation to a small refactor: extend `spawn_progress_renderer` with a second arg `sink: Option<Arc<dyn JobSink>>` (or pass through a callback). Direct mode passes `None`; daemon mode passes `Some(BroadcastSink-as-arc)`.

- [ ] **Step 2: Render `Event::Progress` in the attached CLI**

In `daemon::client::render_event`, install an `indicatif::ProgressBar` lazily on the first `Progress` event and update it with each subsequent event. Style it identically to direct mode's bar in `download.rs`.

- [ ] **Step 3: Smoke-test**

Run a `--use-daemon download` against Spacewar. The submitting terminal shows a progress bar identical to direct mode.

- [ ] **Step 4: Commit**

```bash
jj describe -m "feat(daemon): forward DownloadEvent through JobSink::progress; render bar client-side"
jj new
```

---

### Task 24: Integration tests with a mock SteamClient

**Files:**
- Create: `crates/steamroom-cli/tests/daemon.rs`

- [ ] **Step 1: Decide on the mock approach**

Two viable paths:
1. A hand-rolled trait `SteamClientLike` that `SteamClient<LoggedIn>` implements, with a test mock implementing the same trait. Requires refactoring `commands/*` to take `&dyn SteamClientLike`. Large change.
2. Use the existing `capture` infrastructure in `steamroom-cli` to record a real session against Spacewar (free, anonymous), then replay it in tests.

Choose **(2)** for v1 (it requires no library refactor). Tests are network-free at execution time but need a one-time capture step.

- [ ] **Step 2: Add a `daemon_smoke` integration test**

Create `crates/steamroom-cli/tests/daemon.rs`:

```rust
//! Daemon integration tests. These exercise the in-process daemon harness
//! (no real socket; uses `tokio::io::duplex`) but full request/response
//! cycles end-to-end.

use steamroom_cli::daemon::framing::{read_frame, write_frame};
use steamroom_cli::daemon::proto::*;
use steamroom_cli::daemon::server::{handle_connection, DaemonState};
use tokio::io::duplex;

#[tokio::test]
async fn submit_two_info_jobs_priority_ordering() {
    // ... build a DaemonState without a real SteamClient (use a stub
    // that fails immediately; this test asserts queue behavior, not
    // dispatch).
}
```

Full test bodies are out of scope for this plan; the task is to create the file with at least one passing test that exercises Status round-trip end-to-end.

NOTE: `steamroom-cli` is a binary crate. To run integration tests against its modules, either:
- Add a `[lib]` target to its Cargo.toml exposing `mod daemon`, `mod commands`, etc., or
- Move daemon code into a sibling library crate (`steamroom-cli-daemon`).

Pick (a). Edit `crates/steamroom-cli/Cargo.toml`:

```toml
[lib]
name = "steamroom_cli"
path = "src/lib.rs"
```

Create `crates/steamroom-cli/src/lib.rs` that re-exports the modules under `pub`:

```rust
pub mod cli;
pub mod commands;
pub mod daemon;
pub mod errors;
pub mod sink;
pub mod download;
```

(`main.rs` keeps its `mod ...` for the binary path; ideally it imports from the library, but doing both works.)

- [ ] **Step 3: Run tests**

Run: `cargo test -p steamroom-cli --test daemon`
Expected: passes.

- [ ] **Step 4: Commit**

```bash
jj describe -m "test(daemon): integration test harness via lib target"
jj new
```

---

### Task 25: README updates and self-review

**Files:**
- Modify: `README.md`
- Modify: any plan-level loose ends found during self-review.

- [ ] **Step 1: Add a daemon section to README**

Add a section after `## Commands`:

```markdown
## Daemon mode

`steamroom` can run as a background daemon that holds an authenticated
Steam session and serves subsequent CLI invocations over a local socket.

```bash
# Start in the background. Authenticates once; prints PID and stop info.
steamroom --daemon --username myaccount

# Route subsequent commands through the running daemon.
steamroom --use-daemon info --app 730
steamroom --use-daemon download --app 480 --depot 481 -o spacewar/

# Jump the queue.
steamroom --use-daemon --priority info --app 730

# Submit without waiting.
steamroom --use-daemon --detach download --app 480 --depot 481

# Observe the daemon.
steamroom daemon status            # ratatui dashboard
steamroom daemon status --once     # one-shot text snapshot
steamroom daemon status --once --format json
steamroom daemon info              # pid + socket + stop command (no RPC)

# Stop.
steamroom daemon stop
steamroom daemon stop --force      # cancel the active job too
```

The daemon serves exactly one account: the one it authenticated as at
launch. To use a different account, stop and restart the daemon.
```

- [ ] **Step 2: Run the full self-review checklist**

- Compile clean: `cargo build -p steamroom-cli`
- Tests pass: `cargo test -p steamroom-cli`
- Grep for forbidden patterns: `grep -RnE '[—–→←⇒⇐]' crates/steamroom-cli/src` should return nothing.
- Grep for unwrap_or in new code: `grep -RnE 'unwrap_or(_else|_default)?' crates/steamroom-cli/src/daemon crates/steamroom-cli/src/commands crates/steamroom-cli/src/sink.rs` -- each remaining occurrence must have a justifying comment per `AGENTS.md`.
- Manual smoke: start daemon, submit info/download, attach, detach, stop.

- [ ] **Step 3: Commit**

```bash
jj describe -m "docs(daemon): README section; final polish"
jj new
```

---

## Self-review (post-plan)

After writing this plan, the author should check the spec against it and fill gaps. Known gaps to validate before execution:

1. **`Cli::into_rpc_request` ordering** -- the implementation in task 18 must validate `--priority` and `--use-daemon` interaction. Confirm `clap`'s `conflicts_with` covers `--daemon` vs `--use-daemon`; the `priority`-without-`use-daemon` case is checked manually in `validate()`.

2. **`run_files` signature drift** -- `commands::files::run_files` takes `Option<SteamClient<LoggedIn>>`. The daemon's `dispatch` arm for `Request::Files` must pass `Some(client)`. The direct path passes `None` when `--manifest-file` is set.

3. **Replay buffer** -- the spec calls for a per-job ring of events so `Attach { job_id }` to a finished job replays its event log. Task 14 has a TODO for this; if review insists on it being shipped in v1, add a task after task 14 that adds a `per_job_logs: Mutex<HashMap<JobId, RingBuffer<Event>>>` to `DaemonState` and replays from it on `Attach` to a `recent` job. If not v1, document the gap in the README.

4. **`unwrap_or` audit** -- the existing CLI code has many `unwrap_or` / `unwrap_or_else` calls. The refactor moves them; it does not remove them. Per `AGENTS.md`, these are flagged for review. Decide whether to fix them as part of this work or open a follow-up issue.

5. **Cancellation granularity** -- `tokio::select!` against the cancel token drops the in-flight future. For `run_download` this drops the manifest fetch / job orchestration future; the spawned chunk download tasks held by `DepotJob` may continue until they hit a yield point. This is acceptable for v1 but should be documented in a comment near the `select!`.

6. **Windows path** -- `detach_and_exec_resume` returns `Unsupported` on Windows. Confirm with the user whether v1 ships with foreground-only `--daemon` on Windows or excludes Windows from the daemon feature entirely.

7. **`logind`/`launchd` integration** -- out of scope per spec; the daemon's log file lives in `$TMPDIR` and survives only until rotation. If the user wants persistent logs they should use shell redirection or a process manager.

---

## Execution Handoff

Plan complete and saved to `docs/superpowers/plans/2026-06-02-daemon-mode.md`. Two execution options:

**1. Subagent-Driven (recommended)** -- a fresh subagent per task, with adversarial review between phases. Fastest iteration, best alignment with the `AGENTS.md` review-at-milestones rule.

**2. Inline Execution** -- execute tasks in this session using superpowers:executing-plans, with checkpoints at phase boundaries.

Which approach?
