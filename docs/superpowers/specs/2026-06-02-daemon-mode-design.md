# Daemon mode for `steamroom`

## Background

Every `steamroom` invocation today re-runs a full bring-up: CM server discovery, transport selection, encryption handshake, auth (refresh token / password / QR / Steam Guard), and a `CLIENT_LOG_ON` round trip. For one-off commands this is fine. For users running many commands in a row -- `info`, then `files`, then `download` against the same app -- the cost is repeated, visible, and avoidable. The expensive piece is the authenticated `SteamClient<LoggedIn>`; everything downstream (PICS lookups, depot keys, manifest fetches, CDN downloads) is cheap by comparison.

This spec defines a daemon mode that authenticates once, then services subsequent CLI invocations over a local socket -- reusing the live `SteamClient<LoggedIn>`, a queue of requests, and a streaming progress channel.

The IPC pattern (length-prefixed rkyv frames over `interprocess::local_socket::GenericNamespaced`) is borrowed from `hxy`'s single-instance IPC. The wire schema is steamroom's own.

## Scope

In:

- A new `daemon` module inside `steamroom-cli` with submodules for IPC, protocol, server, client, TUI, and lifecycle.
- Two global CLI flags: `--daemon` (launch a daemon) and `--use-daemon` (route this command through the running daemon).
- A `--priority` flag on every command, valid only with `--use-daemon`, that jumps the daemon's queue.
- A new `daemon` subcommand namespace for control: `daemon stop`, `daemon status`, `daemon info`, `daemon attach <job-id>`.
- A ratatui status TUI; a non-interactive snapshot mode (`daemon status --once` / `--format json`).
- Refactor of `run_download` / `run_info` / `run_files` / `run_manifests` / `run_diff` / `run_packages` / `run_save_manifest` / `run_workshop` / `run_local_info` in `steamroom-cli/src/main.rs` so they take an injected output sink and an externally-supplied authenticated `SteamClient<LoggedIn>`.

Out:

- Multi-account daemon. A daemon serves exactly one account: the one used to authenticate at launch.
- Lazy / on-first-request authentication. The daemon is unusable until it has authenticated.
- Auto-spawn of a daemon when `--use-daemon` is passed and none is running. The CLI errors out and tells the user how to start one.
- Auto-reconnect of the underlying CM connection if it drops mid-session. The next job fails; the daemon does not heal itself in v1.
- Persistent job history across daemon restarts. The `recent` ring buffer lives in memory only.
- Multi-account / re-auth RPCs to a running daemon.
- A Windows `--daemon` launch path beyond a documented re-exec scheme (see Daemon launch below).
- Authentication via the running daemon (e.g. attaching a new account to a live daemon).

## Design decisions

| Decision | Choice | Why |
|---|---|---|
| Auth model | Single-account, set at daemon launch | Matches the "reuse one connection" intent; avoids pooling complexity. |
| Concurrency | Strict FIFO with priority insertion; one active job at a time | Avoids contention over the single CM connection, CDN bandwidth, and disk; keeps queue/priority semantics honest. |
| Submission behavior | Attach by default; `--detach` to fire-and-forget; `Ctrl-C` detaches without cancelling | Feels like running the command directly while still benefiting from the daemon. |
| No-daemon fallback | Error out cleanly | Keeps the mental model explicit; scripts can detect and act. |
| Daemon launch | Foreground auth, then double-fork to detach; print pid/socket/stop info before detaching | Interactive Steam Guard works normally; long-running daemon is detached after auth. Windows uses a re-exec equivalent. |
| Status UI | Ratatui TUI by default; `--once`/`--format json` for non-interactive snapshots | TUI gives high signal density (queue + active + log + priority/cancel controls); non-interactive mode covers scripts and CI. |
| RPC types | Separate `proto` module distinct from clap structs | Lets the wire format evolve independently; CLI types stay clap-shaped, RPC types stay rkyv-shaped. |
| Wire framing | `u32` LE length prefix + rkyv `Frame` enum, 16 MiB cap, `proto_version: u16` per frame | Same shape as hxy IPC; version prefix lets us reject mismatched daemon/client cleanly. |
| Output preservation | `JobSink` trait injected into every `run_*` function; daemon's sink broadcasts `Event`s, direct CLI's sink writes stdout | Daemon-mode and direct-mode produce identical output to the user. |
| Tracing in daemon | Custom `tracing_subscriber::Layer` that turns `job_id`-scoped events into `Event::Log` for that job | Per-job log isolation without restructuring how each command emits diagnostics. |

## Top-level layout

```
crates/steamroom-cli/src/daemon/
├── mod.rs        -- re-exports; feature-gated if we add a feature flag later
├── ipc.rs        -- socket binding, framing, version check
├── proto.rs      -- Request, Response, Event, JobState, JobKind, ProgressUpdate, ...
├── server.rs     -- DaemonState, accept loop, worker loop, queue, broadcast
├── client.rs     -- RPC client used by --use-daemon paths
├── tui.rs        -- ratatui status dashboard
└── lifecycle.rs  -- launch, double-fork, PID file, socket-path discovery, stop RPC
```

Two new global flags in `Cli`:

- `--daemon` -- implies "launch a daemon process". Mutually exclusive with `--use-daemon`. Auth flags (`--username`, `--password`, `--qr`, `--use-steam-token`, `--device-name`, `--remember-password`) are honored here.
- `--use-daemon` -- implies "send this command to the running daemon". Mutually exclusive with `--daemon`. Auth flags rejected at parse time with a clear message: `--use-daemon: per-request auth flags are not supported; the daemon is bound to the account it was launched with`.

A new `Command::Daemon(DaemonArgs)` subcommand:

```text
steamroom daemon stop [--force]
steamroom daemon status [--once] [--format <table|json|plain>]
steamroom daemon info
steamroom daemon attach <job-id>
```

`steamroom daemon info` is the one daemon-control path that does not contact the daemon -- it reads the PID file + socket path from disk and prints them, so it works even when the daemon is wedged.

Every command gains a `--priority` flag that is only accepted in combination with `--use-daemon`. With `--daemon` or in direct mode, `--priority` is rejected at parse time.

### Refactor of `run_*` functions

Today's signatures look like `async fn run_download(args: DownloadArgs, auth: &AuthOptions, show_progress: bool) -> Result<(), CliError>`. They build their own `SteamClient<LoggedIn>` via `connect_and_login(auth)` and write to stdout directly.

New shape:

```rust
async fn run_download(
    args: DownloadArgs,
    client: SteamClient<LoggedIn>,
    sink: &dyn JobSink,
    cancel: CancellationToken,
) -> Result<(), CliError>;
```

`async_main` in direct mode builds a `StdoutSink` (writes via `println!`/`eprintln!`, drives an `indicatif` progress bar), authenticates, and calls the run_* function. The daemon worker builds a `BroadcastSink` (each call becomes an `Event::Stdout`/`Event::Progress`/`Event::Log` tagged with `job_id`) and reuses the cached client.

`JobSink` lives in `daemon/proto.rs` so both modes import it from the same place:

```rust
pub trait JobSink: Send + Sync {
    fn stdout_line(&self, line: &str);
    fn progress(&self, p: ProgressUpdate);
    fn log(&self, level: LogLevel, target: &str, message: &str);
}
```

## Socket, framing, lifecycle

### Socket location

Per-user; `interprocess::local_socket::GenericNamespaced` resolves the platform-appropriate name:

- Linux: abstract socket `\0steamroom-<uid>` (no filesystem footprint).
- macOS: `/tmp/steamroom-<uid>.sock`.
- Windows: `\\.\pipe\steamroom-<username>`.

`ListenerOptions::new().name(name).reclaim_name(true).try_overwrite(true)` is used at bind time, matching the hxy pattern: `try_overwrite(true)` only fires after a liveness probe to a peer at that name fails. The probe is `Stream::connect` + send a `Request::Status` with a 200ms read timeout; success => daemon exits with `daemon already running (pid X)`, failure => proceed to overwrite.

### PID file

`$XDG_RUNTIME_DIR/steamroom.pid` if set, else `$TMPDIR/steamroom-<uid>.pid`. Contains the post-fork PID on one line. Used by:

- `daemon info` (to print the PID without contacting the daemon).
- `daemon stop` (to fall back to `kill(pid)` if the RPC stop fails).
- The launcher (to detect a stale PID file with no socket, or a stale socket with no PID file).

### Frame format

```
u16  proto_version   (current: 1)
u32  payload_length  (little-endian, ≤ 16 * 1024 * 1024)
[u8] rkyv-archived Frame
```

```rust
pub enum Frame {
    Request(Request),
    Response(Response),
    Event(Event),
    EndOfStream { exit_code: i32 },
}
```

Receivers that see a `proto_version` they don't recognize close the connection with `Error { kind: ProtocolMismatch, message: "incompatible daemon version; restart it" }` before attempting to deserialize the payload.

A single client connection carries one logical RPC call: one `Request` in, then either a single `Response` and close (control RPCs), or zero-or-more `Event` frames followed by `EndOfStream` (job submissions, `Subscribe`, `Attach`).

### Daemon lifecycle on launch

`steamroom --daemon [auth flags]`:

1. Parse `Cli`. Build a `LoginBuilder` with the same logic as `connect_and_login` (no refactor of that logic; it is reused).
2. Authenticate in the **foreground** so Steam Guard prompts, QR rendering, etc. work normally.
3. Save the refresh token via the existing `save_token` path.
4. Probe the socket name for a live peer; abort if one exists.
5. Bind the socket with `reclaim_name(true).try_overwrite(true)`.
6. Print the info block to stderr (before fork):

   ```text
   steamroom daemon started
     pid    : 12345
     socket : /tmp/steamroom-1000.sock
     stop   : steamroom daemon stop    (or: kill 12345)
     logs   : /tmp/steamroom-1000.log
   ```

7. Write the PID file.
8. Double-fork. The child redirects stdout/stderr to the log file, then enters the accept loop. The grandparent waits on a one-shot pipe for "bind confirmed", then exits 0.

On Windows: no `fork`. The launcher re-execs itself with `--daemon --detached-child <handoff-fd>` where the handoff carries the already-bound socket plus the live `SteamClient<LoggedIn>` state. If that handoff turns out to be non-trivial in practice, the v1 Windows behavior is "foreground only" -- `--daemon` runs in the foreground and the user backgrounds it themselves (documented).

### Stop semantics

`daemon stop` (CLI) -> `Request::Stop` (RPC). Daemon:

1. Stops accepting new submissions immediately.
2. Closes the listener.
3. Waits for the active job to finish, with a grace period (default 30s; configurable via `--force` which sends `Cancel` to the active job's `CancellationToken` first).
4. Sends `Event::Log { Info, "daemon shutdown" }` to all subscribers, then `EndOfStream { 0 }`.
5. Unlinks the PID file. Exits 0.

## Protocol (`proto.rs`)

### Request

```rust
pub enum Request {
    // Job submissions. Each returns Response::JobAccepted, then a stream of Events.
    Download     { args: DownloadParams,     priority: bool },
    Info         { args: InfoParams,         priority: bool },
    Files        { args: FilesParams,        priority: bool },
    Manifests    { args: ManifestsParams,    priority: bool },
    Diff         { args: DiffParams,         priority: bool },
    Packages     { args: PackagesParams,     priority: bool },
    SaveManifest { args: SaveManifestParams, priority: bool },
    Workshop     { args: WorkshopParams,     priority: bool },
    LocalInfo    { args: LocalInfoParams,    priority: bool },

    // Daemon control.
    Status,                                  // one-shot snapshot
    Subscribe,                               // open-ended event stream (TUI uses this)
    Attach         { job_id: JobId },        // re-attach to an existing job (running or finished)
    Cancel         { job_id: JobId },        // remove from queue, or signal active job
    TogglePriority { job_id: JobId },        // raise/lower a queued job's priority
    Stop           { force: bool },          // graceful shutdown (force => cancel active first)
}
```

The `*Params` types are owned, rkyv-archivable shadows of the corresponding clap structs:

- `PathBuf` => `String` (receiver wraps with `PathBuf::from`, lossless on every platform we ship -- same convention as hxy).
- `regex::Regex` => raw pattern string; daemon compiles on dispatch and surfaces compile errors as `Event::Log { Error, ... }` + non-zero `JobFinished`.
- `OutputFormat` (clap enum) => a plain rkyv-archivable `enum OutputFormat { Table, Json, Plain }`.
- `--debug`, `--non-interactive`, `--no-progress`, `--quiet` are client-side concerns and are not transmitted.
- `--capture` captures the network traffic of the underlying `SteamClient`, which is owned by the daemon. With `--use-daemon` it is rejected at parse time (`--capture is not supported with --use-daemon; pass it to --daemon at launch instead`). It is honored on `--daemon` launches, applying to the daemon's entire lifetime.

`Cli::into_rpc_request(self) -> Result<Request, CliError>` does the conversion. This is where the "no per-request auth" check fires.

### Response

```rust
pub enum Response {
    JobAccepted { job_id: JobId, position: u32 },   // 0 => running now
    Status(StatusSnapshot),
    Stopping,
    Error { kind: ErrorKind, message: String },
}

pub enum ErrorKind {
    ProtocolMismatch,
    InvalidRequest,
    DaemonBusy,           // daemon is shutting down; reject new jobs
    JobNotFound,          // for Attach/Cancel/TogglePriority targeting a missing id
    InternalError,
}
```

### Event

```rust
pub enum Event {
    JobStarted   { job_id: JobId, kind: JobKind, args_summary: String },
    JobFinished  { job_id: JobId, exit_code: i32 },
    Log          { job_id: Option<JobId>, level: LogLevel, target: String, message: String },
    Progress     { job_id: JobId, p: ProgressUpdate },
    Stdout       { job_id: JobId, line: String },
    QueueChanged { snapshot: StatusSnapshot },  // emitted after enqueue/cancel/toggle
}

pub enum LogLevel { Error, Warn, Info, Debug, Trace }

pub struct ProgressUpdate {
    pub bytes_done:  u64,
    pub bytes_total: u64,
    pub files_done:  u32,
    pub files_total: u32,
    pub rate_bytes_per_sec: u64,
    pub eta_seconds:        u32,
}
```

`Stdout` is the bridge that preserves per-command output formatting. The existing `run_*` functions emit tables, JSON, plain text -- under the daemon they go through `JobSink::stdout_line`, which becomes `Event::Stdout`. Attached CLIs echo that to their own stdout. Output is identical to direct mode.

### Status snapshot

```rust
pub struct StatusSnapshot {
    pub daemon_pid: u32,
    pub daemon_started_at: u64,     // unix seconds
    pub account: Option<String>,
    pub active: Option<JobRecord>,
    pub queue: Vec<JobRecord>,      // priority items first, then FIFO
    pub recent: Vec<JobRecord>,     // last 32 finished, newest first
}

pub struct JobRecord {
    pub job_id: JobId,
    pub kind: JobKind,
    pub args_summary: String,
    pub priority: bool,
    pub submitted_at: u64,
    pub started_at:  Option<u64>,
    pub finished_at: Option<u64>,
    pub exit_code:   Option<i32>,
    pub progress:    Option<ProgressUpdate>,
}
```

## Daemon internals (`server.rs`)

### State

```rust
pub struct DaemonState {
    pub client: SteamClient<LoggedIn>,
    pub queue:      Mutex<VecDeque<QueuedJob>>,     // priority items at front
    pub active:     Mutex<Option<RunningJob>>,
    pub recent:     Mutex<RingBuffer<JobRecord>>,   // capacity 32
    pub events:     broadcast::Sender<Event>,        // ~512-deep channel
    pub next_job_id: AtomicU64,
    pub shutdown:   CancellationToken,
    pub account:    String,
    pub started_at: u64,
}
```

Wrapped in `Arc<DaemonState>` and shared across tasks.

### Tasks

**Accept loop.** `listener.accept()` in a loop, spawning a `connection_task` per incoming socket. Exits when `shutdown` fires.

**Connection task** (per client). Reads one `Frame::Request`, dispatches:

- *Job submission* (`Download`, `Info`, ...) -> call `enqueue(args, priority)`, write `Response::JobAccepted { job_id, position }`. Then subscribe to `events`, filter by this `job_id`, stream matching `Event` frames until `JobFinished` arrives, write `EndOfStream { exit_code }`, close.
- `Subscribe` -> no job filter; stream every event until the client drops or `shutdown` fires.
- `Attach { job_id }` -> if the job is active or queued, behave like a submission's streaming half. If finished, replay buffered events for that job from `recent`'s associated log buffer (see Replay buffer below), then `EndOfStream`.
- `Cancel`/`TogglePriority` -> mutate the queue (or signal the active token), reply `Response::Stopping` for `Stop`, `Response::Error { JobNotFound }` if the id isn't present, otherwise broadcast `QueueChanged`.
- `Status` -> snapshot the state, reply `Response::Status(snapshot)`, close.
- `Stop { force }` -> set `shutdown`, optionally cancel active, reply `Response::Stopping`, close.

**Worker loop** (single task, single concurrent job):

```rust
loop {
    tokio::select! {
        _ = shutdown.cancelled() => break,
        job = wait_for_next_job(&queue) => run_job(job, &client, &events, &recent).await,
    }
}
```

`run_job` builds a `BroadcastSink` for this `job_id`, calls into the refactored `run_*` function, captures any returned `CliError` and emits it as `Event::Log { Error, ... }` before sending `JobFinished { exit_code }`.

### Replay buffer

For each finished job, the broadcast events tagged with that `job_id` are also captured into a per-job ring (capped at e.g. 200 entries) stored alongside its `JobRecord`. `Attach { job_id }` on a finished job sends these in order before `EndOfStream`. Capped to avoid unbounded memory: late attachers to a giant download get the start and end, with a gap in the middle.

### Queue + priority

`VecDeque<QueuedJob>` with priority items at the front in submit order; non-priority items follow in submit order. `enqueue` inserts at the boundary; `TogglePriority` moves the targeted job across the boundary; `Cancel` removes by id.

The active job is not in the queue. `position` in `Response::JobAccepted` is computed as `if priority { count_priority_ahead } else { queue.len() }`, with `position == 0` only when the queue was empty *and* no active job was running.

### Cancellation

One `CancellationToken` per job. For queued jobs, `Cancel` removes them from the deque and emits `Event::JobFinished { exit_code: 130 }` (no work was done). For the active job, `Cancel` calls `token.cancel()`; the run_* function observes this through a `tokio::select!` arm and aborts.

Wiring cancellation into existing code:

- `run_download` calls into `steamroom_client::download::DepotJob::download` which already runs its work as tokio tasks. We wrap that single `await` in `tokio::select! { res = job.download(...) => res, _ = cancel.cancelled() => Err(CliError::Cancelled) }`.
- Other run_* functions are PICS/CDN one-shots; they `tokio::select!` against the token at their top-level `await`s. Cancelling a half-done network request just drops the future.

### Tracing layer

A custom `tracing_subscriber::Layer` installed inside the daemon's runtime. It reads the current span's `job_id` field (which the worker sets when entering each job's span) and maps `tracing::Event`s to `Event::Log { job_id: Some(id), ... }`. Events outside any job's span have `job_id: None` and only land in the daemon log file (not broadcast).

## Client side, TUI, snapshots

### `--use-daemon` request flow

1. Parse `Cli`. `Cli::into_rpc_request()` converts to a `Request`, rejecting per-request auth flags.
2. Connect to the socket. On `ENOENT` / `ECONNREFUSED`: print `no daemon running; start one with \`steamroom --daemon\`` and exit 2.
3. Send the framed `Request`.
4. Read one `Response`:
   - `JobAccepted { job_id, position }` -> if `--detach`: print `job <id> queued (position N)`, exit 0. Else: enter the attach loop.
   - `Error { kind, message }` -> print, exit non-zero (exit code derived from `kind`).
5. *Attach loop* -- read frames, dispatch:
   - `Event::Stdout { line }` -> `println!("{line}")`.
   - `Event::Progress(p)` -> render via `indicatif`, same look as direct mode. Bar is hidden when `--no-progress` is passed (client-side filter).
   - `Event::Log { level, target, message }` -> forward to the client's `tracing` subscriber so existing `--debug` / `--quiet` filtering decides what shows.
   - `Event::JobFinished { exit_code }` -> after `EndOfStream`, exit with that code.
6. Ctrl-C while attached -> drop the socket, print `detached -- reattach with \`steamroom daemon attach <id>\``, exit 0. The daemon's broadcast send fails silently for that subscriber; the job keeps running.

### `daemon attach <job-id>`

Identical to the attach half of step 5, but for an existing job. Daemon's `Attach` handler replays buffered events first, then resumes live streaming if the job is still running.

### `daemon status`

Three modes:

- Default (TTY, no `--once`): TUI.
- `--once`: send `Request::Status`, render the `StatusSnapshot` as a multi-line text block, exit.
- `--format json` (implies `--once`): print the snapshot as JSON, exit.

### TUI (`daemon/tui.rs`)

Built with `ratatui` + `crossterm`. State machine driven by two concurrent inputs:

- A `Subscribe` connection's event stream.
- Keypresses via `crossterm`.

Initial flow: send `Request::Status` to seed state, then open a `Subscribe` connection for live updates.

Layout (matches the agreed mockup):

- Top-left "queue" pane: list with priority items starred. `↑/↓` selects.
- Top-right "active job" pane: kind, args summary, indicatif-style bar from `Progress` events, rate + ETA + bytes done/total.
- Bottom "log" pane: combined tracing log for all jobs, color-coded by level, capped at last 1000 lines.
- Footer: keybindings: `q` quit, `↑↓` select, `p` toggle priority, `x` cancel, `r` reset selection.

Key actions:

- `p` -> send `Request::TogglePriority { job_id: selected }`.
- `x` -> send `Request::Cancel { job_id: selected }`.
- `q` -> exit cleanly.

If the daemon dies during the session, the TUI prints `daemon disconnected` outside ratatui's screen and exits 1. No auto-reconnect in v1.

### `daemon stop`

Sends `Request::Stop { force }`, prints `stopping daemon (active job will finish, ~Ns grace)` (or `stopping daemon (cancelling active job)` with `--force`), waits for the connection to close, exits 0.

### `daemon info`

Reads the PID file + computes the socket path, prints them without contacting the daemon. Useful for debugging a wedged daemon or scripting.

## Error handling, edge cases

### Failure modes

- **Daemon authentication failure at launch** -- `--daemon` exits non-zero in the foreground before forking. No socket bound, no PID file written.
- **Stale socket from a crashed daemon** -- pre-bind probe times out -> socket file is overwritten via `try_overwrite(true)`. If the probe succeeds, the launcher exits with `daemon already running (pid X)`.
- **Stale PID file with no socket, or vice versa** -- `daemon info` reports the inconsistency; `--daemon` cleans up and proceeds.
- **Daemon dies mid-job with attached clients** -- broadcast channel drops -> connection tasks see `RecvError` -> they emit `Event::Log { Error, "daemon shutdown" }` and `EndOfStream { 130 }`. Attached CLIs print and exit. The TUI shows "daemon disconnected".
- **Client disconnects from a streaming attach** -- `events.send()` is best-effort across the `broadcast` channel; failures are ignored. The worker continues. The job is still visible in `daemon status` as active.
- **RPC version skew** between client and daemon (one half upgraded out of band) -- `proto_version` mismatch rejected before deserialization; receiver writes `Error { ProtocolMismatch, "incompatible daemon version; restart it" }` and closes.
- **`SteamClient` connection drops** while the daemon is idle -- surfaced on the next job as a connection error. The daemon does not auto-reconnect in v1; the job fails with non-zero `JobFinished`, the user restarts the daemon.
- **Per-request auth flags on `--use-daemon`** -- rejected at CLI-parse time in `Cli::into_rpc_request()` with a clear message, before any socket I/O.
- **Huge single-line JSON output** -- `Event::Stdout` carries a `String` per line; the 16 MiB frame cap is generous but we accept that a single `info --format json` line larger than that would fail. Not splitting in v1; flag as a future improvement if it ever bites.

### Filtering and verbosity

`--quiet` / `--no-progress` / `--debug` are client-side concerns. The daemon always emits the full event stream; each attached client filters what to render via its own `tracing` setup. Two attached clients of the same job can legitimately show different verbosity.

### Refresh tokens

When the daemon authenticates, it saves the refresh token via the existing `save_token` path. No new behavior, no daemon-side token store.

### Daemon stop interactions

- `daemon stop` while the TUI is open: daemon stops accepting new jobs, broadcast keeps flowing until the active job finishes, TUI sees `JobFinished` then daemon shutdown -> "daemon disconnected" -> exit.
- `daemon stop --force`: daemon cancels the active job before grace, then exits.

## Testing

### Unit tests

`daemon/proto.rs`:
- Round-trip every `Request` / `Response` / `Event` variant through rkyv (same shape as hxy's `roundtrips_open_message`).
- Reject oversized length prefix.
- Reject garbage payload (rkyv validation failure).
- Reject mismatched `proto_version`.

`daemon/server.rs`:
- Queue priority insertion ordering: enqueue mixed priority/non-priority, assert order.
- `TogglePriority` moves an item across the priority boundary.
- `Cancel` removes by id.
- `JobAccepted::position` calculation.

### Integration tests (`crates/steamroom-cli/tests/daemon.rs`)

Use a stub `SteamClient<LoggedIn>` -- either via the existing `capture` infrastructure, or a hand-rolled mock implementing just the methods the dispatcher touches (`pics_get_access_tokens`, `pics_get_product_info`, `get_depot_decryption_key`, etc.). The mock returns canned responses; no real Steam network.

- Submit `Request::Info`, attach, assert an ordered stream of `Stdout` / `JobFinished` events.
- Submit two jobs, verify FIFO. Add a third with `priority: true`, verify it runs after the active job but before the other queued one.
- `Cancel` a queued job -- emits `JobFinished { exit_code: 130 }`, removed from `Status`.
- `Cancel` an active job -- token observed, run_* returns `CliError::Cancelled`, `JobFinished` carries non-zero exit code.
- `Subscribe` from one connection while submitting two jobs from another -- subscriber receives events from both.
- Disconnect mid-attach, verify the worker keeps running and a second attach picks up where it left off (live), or replays from `recent` if it has already finished.

### Smoke test

Shell-driven script under `scripts/daemon-smoke.sh`:
1. `steamroom --daemon --use-steam-token` against a free app like Spacewar.
2. `steamroom daemon info` -- assert non-empty output.
3. `steamroom --use-daemon info --app 480` -- assert exit 0.
4. `steamroom daemon stop` -- assert daemon exits.

Not part of `cargo test` (network-dependent). Documented in `scripts/README.md`.

## Crate placement

`crates/steamroom-cli/src/daemon/`. The daemon is a CLI concern, not a library concern: it routes args structures and command dispatch through a socket, both of which live in the CLI crate already. Putting it in `steamroom-client` would force every library consumer to pull in `interprocess`, `ratatui`, `crossterm` for a feature only the binary uses.

If we later want the daemon protocol reusable by a hypothetical `steamroom-daemon-client` library, `daemon/proto.rs` is the small, dependency-light piece to extract. Out of scope for v1.

## New workspace dependencies

- `interprocess` -- same crate hxy uses; cross-platform local sockets.
- `rkyv` -- same crate hxy uses; archived/deserialized wire types.
- `ratatui` + `crossterm` -- TUI dashboard.
- `nix` (Unix-only) -- for `setsid`, double-fork primitives. Already used transitively elsewhere; pin a single version.
- Existing deps reused: `tokio` (broadcast, mpsc, CancellationToken via `tokio-util`), `tracing`, `tracing-subscriber`, `serde_json` (for `--format json` snapshot output), `indicatif`.
