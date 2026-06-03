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
use crate::daemon::server::DaemonState;

/// PID file and log file live in a stable per-user cache dir so they
/// can be found from any shell. `$TMPDIR` on macOS is session-specific
/// (`/var/folders/<hash>/T/`), which would cause `daemon info` and
/// `daemon stop` to miss the daemon from a different shell session.
fn cache_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("XDG_CACHE_HOME") {
        return PathBuf::from(dir).join("steamroom");
    }
    if let Some(home) = std::env::var_os("HOME") {
        return PathBuf::from(home).join(".cache").join("steamroom");
    }
    // Last resort: /tmp + uid so we at least match the socket location.
    PathBuf::from("/tmp").join(format!("steamroom-{}", unix_uid()))
}

pub fn pid_file_path() -> PathBuf {
    cache_dir().join("daemon.pid")
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

pub fn log_path() -> PathBuf {
    cache_dir().join("daemon.log")
}

/// Path to the JSON file where the daemon persists its `recent` ring on
/// shutdown and reloads on startup. Co-located with the PID and log.
pub fn recent_history_path() -> PathBuf {
    cache_dir().join("recent.json")
}

/// Load persisted recent-job history, if any, into the given state's
/// ring. Silent on failure -- a missing or corrupt file just means we
/// start with an empty history.
pub async fn load_recent_history(state: &DaemonState) {
    let path = recent_history_path();
    let Ok(data) = std::fs::read_to_string(&path) else { return; };
    let Ok(records) = serde_json::from_str::<Vec<crate::daemon::proto::JobRecord>>(&data) else {
        tracing::warn!("recent history at {} is corrupt; ignoring", path.display());
        return;
    };
    let mut recent = state.recent.lock().await;
    for r in records {
        recent.push(r);
    }
}

/// Snapshot the recent ring to disk. Best-effort: any I/O failure is
/// logged but does not block shutdown.
pub async fn save_recent_history(state: &DaemonState) {
    let path = recent_history_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let records: Vec<_> = state.recent.lock().await.iter().cloned().collect();
    match serde_json::to_string(&records) {
        Ok(json) => {
            if let Err(e) = std::fs::write(&path, json) {
                tracing::warn!("failed to write recent history to {}: {e}", path.display());
            }
        }
        Err(e) => {
            tracing::warn!("failed to serialize recent history: {e}");
        }
    }
}

/// Spawn the daemon child detached from this process and probe the
/// socket until it binds.
///
/// On Unix we use `pre_exec` to call `setsid` so the child becomes its
/// own session leader (no controlling terminal). On Windows we set
/// `DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP` so the child has no
/// console attached and survives the parent's exit. Both platforms
/// redirect the child's stdin/stdout/stderr to the daemon log file
/// before spawn so the user sees nothing from the daemon on this
/// terminal.
///
/// The parent does NOT wait on the spawned child. It instead polls the
/// daemon socket via `wait_for_socket` to confirm `serve_resumed`'s
/// `bind_listener` actually succeeded; on timeout it prints a failure
/// pointing at the log instead of an unfounded success banner.
pub fn detach_and_exec_resume(username: &str, log_path: &std::path::Path) -> Result<(), CliError> {
    use std::process::{Command, Stdio};

    if let Some(parent) = log_path.parent() {
        std::fs::create_dir_all(parent).map_err(CliError::Io)?;
    }
    let log_out = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
        .map_err(CliError::Io)?;
    let log_err = log_out.try_clone().map_err(CliError::Io)?;

    let exe = std::env::current_exe().map_err(CliError::Io)?;
    let mut cmd = Command::new(exe);
    // `daemon start` is included only to satisfy clap's subcommand
    // requirement. `main()` checks `cli.daemon_resume` first and routes
    // to `serve_resumed` before the subcommand handler runs.
    cmd.arg("--daemon-resume")
        .arg(username)
        .arg("daemon")
        .arg("start");
    cmd.stdin(Stdio::null());
    cmd.stdout(Stdio::from(log_out));
    cmd.stderr(Stdio::from(log_err));

    #[cfg(unix)]
    unsafe {
        use std::os::unix::process::CommandExt;
        // SAFETY: `setsid` is async-signal-safe and has no Rust
        // invariants to uphold. `pre_exec` runs after fork() and before
        // execve() so this happens in the child only.
        cmd.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // From winbase.h:
        //   DETACHED_PROCESS          = 0x00000008
        //   CREATE_NEW_PROCESS_GROUP  = 0x00000200
        // Together: the child has no console attached, is in its own
        // process group, and is unaffected by Ctrl-C delivered to the
        // launching console.
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        cmd.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }

    let child = cmd.spawn().map_err(CliError::Io)?;
    let pid = child.id();
    // Detach: don't reap the child. On Unix it gets reparented to init
    // when this process exits; on Windows the new process group keeps
    // it alive independently.
    std::mem::forget(child);

    if !wait_for_socket(std::time::Duration::from_secs(5)) {
        eprintln!("steamroom daemon (pid {pid}) failed to bind socket within 5s");
        eprintln!("check the log for the failure:");
        eprintln!("  {}", log_path.display());
        std::process::exit(1);
    }
    println!("steamroom daemon started");
    println!("  pid    : {pid}");
    println!("  socket : {}", socket_name_string());
    println!("  stop   : steamroom daemon stop    (or: kill {pid} on Unix; taskkill /PID {pid} /F on Windows)");
    println!("  logs   : {}", log_path.display());
    std::process::exit(0);
}

/// Poll the daemon socket until `Status` round-trips successfully or
/// `timeout` elapses. Used by the parent of `detach_and_exec_resume`
/// to verify the grandchild actually finished `serve_resumed`'s
/// `bind_listener` step before reporting success.
fn wait_for_socket(timeout: std::time::Duration) -> bool {
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(_) => return false,
    };
    rt.block_on(async move {
        let deadline = std::time::Instant::now() + timeout;
        let mut delay = std::time::Duration::from_millis(50);
        while std::time::Instant::now() < deadline {
            if crate::daemon::ipc::probe_peer().await.is_ok() {
                return true;
            }
            tokio::time::sleep(delay).await;
            // Backoff: 50ms, 100ms, 200ms, then 200ms thereafter.
            delay = (delay * 2).min(std::time::Duration::from_millis(200));
        }
        false
    })
}

use crate::cli::Cli;
use crate::commands::shared;
use crate::daemon::ipc;
use crate::daemon::server::{handle_connection, worker_loop};
use crate::daemon::tracing_layer::{JobIdAttachmentInstaller, JobScopedLogLayer};

/// Phase 1 of `daemon start`: preflight + foreground authentication.
///
/// Eager (`--username` or any other auth flag was passed): authenticate
/// in the foreground so Steam Guard prompts can be answered on the
/// launching terminal. Returns `Some(username)`; the resumed child
/// re-authenticates non-interactively using the saved refresh token.
///
/// Lazy (no auth flags): skip launch-time login entirely. The worker
/// authenticates on the first job that needs a Steam connection,
/// falling back to auto-detected saved tokens or anonymous. Returns
/// `None`.
pub async fn launch_daemon_authenticate(cli: &Cli) -> Result<Option<String>, CliError> {
    // Fail fast if a daemon is already up.
    if ipc::probe_peer().await.is_ok() {
        return Err(CliError::DaemonAlreadyRunning);
    }
    // Clear a stale PID file pointing at a dead process.
    if let Ok(stale_pid) = read_pid_file() {
        if !pid_is_alive(stale_pid) {
            remove_pid_file();
        }
    }

    let auth = &cli.auth;
    let has_explicit_auth = auth.username.is_some()
        || auth.password.is_some()
        || auth.qr
        || auth.use_steam_token
        || auth.device_name.is_some();

    if !has_explicit_auth {
        return Ok(None);
    }

    let client = shared::connect_and_login(auth).await?;
    let username = auth
        .username
        .clone()
        .or_else(|| shared::detect_steam_user().map(|(u, _)| u))
        .ok_or(CliError::InteractiveAuthRequired)?;
    drop(client);
    Ok(Some(username))
}

/// `kill(pid, 0)` returns 0 if the process exists and we have signal
/// permission, ESRCH if the pid is unknown, EPERM if it exists but is
/// owned by another user. Both EPERM and 0 count as "alive" -- a pid
/// we cannot signal is still occupying the pid namespace.
#[cfg(unix)]
fn pid_is_alive(pid: u32) -> bool {
    // SAFETY: libc::kill is always safe to call with signal 0.
    let rc = unsafe { libc::kill(pid as libc::pid_t, 0) };
    if rc == 0 {
        return true;
    }
    // SAFETY: __error()/errno_location depending on platform, but the
    // io::Error::last_os_error() wrapper handles both.
    std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}
#[cfg(not(unix))]
fn pid_is_alive(_pid: u32) -> bool { true }

/// The actual long-lived daemon process, post-exec. Builds a fresh
/// tokio runtime above this; this just runs the accept loop.
///
/// `username` is empty for daemons launched without auth flags (lazy
/// mode): the worker authenticates on first job. Non-empty means the
/// parent did the interactive auth dance and saved a refresh token;
/// we re-login non-interactively here.
pub async fn serve_resumed(username: String, _cli: Cli) -> Result<(), CliError> {
    let (initial_client, preferred_user) = if username.is_empty() {
        // Lazy mode: no eager login. The worker will authenticate on
        // the first job, using whatever saved token / autodetect /
        // anonymous path applies at that time.
        (None, None)
    } else {
        let token = shared::load_saved_token(&username).ok_or(CliError::InteractiveAuthRequired)?;
        let client = steamroom_client::login::LoginBuilder::new()
            .device_name("steamroom")
            .with_refresh_token(&username, &token)
            .login()
            .await?;
        (Some(client), Some(username.clone()))
    };

    let listener = ipc::bind_listener().await?;

    let pid = std::process::id();
    write_pid_file(pid)?;
    let account_label = if username.is_empty() { None } else { Some(username.clone()) };
    let state = DaemonState::new(account_label, pid, unix_now_lifecycle());

    // Seed the recent ring from disk so `daemon status` and `daemon
    // attach` see jobs from prior daemon lifetimes.
    load_recent_history(&state).await;

    use tracing_subscriber::layer::SubscriberExt;
    use tracing_subscriber::util::SubscriberInitExt;
    let _ = tracing_subscriber::registry()
        .with(tracing_subscriber::fmt::layer())
        .with(JobIdAttachmentInstaller)
        .with(JobScopedLogLayer::new(state.events.clone()))
        .try_init();

    let worker_state = state.clone();
    let worker_client = initial_client;
    let worker_user = preferred_user;
    let mut worker_task = Some(tokio::spawn(async move {
        worker_loop(worker_state, worker_client, worker_user).await;
    }));

    // Collector that drains the broadcast channel into per-job replay
    // rings. Exits cleanly when the broadcast sender drops.
    crate::daemon::server::spawn_replay_collector(state.clone());

    loop {
        // worker_task is always Some here; the None arm is unreachable but
        // avoids an unwrap().
        let join_arm = match worker_task {
            Some(ref mut h) => h,
            None => break,
        };
        tokio::select! {
            _ = state.shutdown.cancelled() => break,
            res = join_arm => {
                // Worker ended -- graceful drain, panic, or abort.
                match res {
                    Ok(()) => tracing::info!("worker_loop exited"),
                    Err(ref e) if e.is_panic() => tracing::error!("worker_loop panicked: {e}"),
                    Err(ref e) => tracing::warn!("worker_loop join error: {e}"),
                }
                worker_task = None;
                state.shutdown.cancel();
                break;
            }
            res = ipc::accept(&listener) => match res {
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

    let _ = state.events.send(crate::daemon::proto::Event::Log {
        job_id: None,
        level: crate::daemon::proto::LogLevel::Info,
        target: "daemon".into(),
        message: "shutting down".into(),
    });
    // If the accept loop exited via shutdown.cancelled() (not via the worker
    // arm), worker_task is still Some -- abort it.
    if let Some(h) = worker_task {
        h.abort();
        let _ = h.await;
    }
    // Persist the recent ring so the next daemon launch shows job
    // history across restarts.
    save_recent_history(&state).await;
    remove_pid_file();
    Ok(())
}

fn unix_now_lifecycle() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
