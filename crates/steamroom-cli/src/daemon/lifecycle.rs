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

pub fn log_path() -> PathBuf {
    let tmp = std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".to_string());
    PathBuf::from(tmp).join(format!("steamroom-{}.log", unix_uid()))
}

/// Foreground-then-detach launch on Unix.
///
/// Steps:
/// 1. fork() -- parent waits on a pipe for the grandchild's PID, prints
///    the info block, and exits 0.
/// 2. Child setsid()s, fork()s again, then the intermediate exits 0.
/// 3. Grandchild dup2's stdout/stderr to the log file, then exec's the
///    same binary with `--daemon-resume <username>`. The resumed process
///    rebuilds tokio, re-authenticates with the cached token, binds the
///    socket, writes the PID file, and enters the accept loop.
#[cfg(unix)]
pub fn detach_and_exec_resume(username: &str, log_path: &std::path::Path) -> Result<(), CliError> {
    use nix::unistd::{fork, setsid, ForkResult, dup2, pipe, close, execv, read, write};
    use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd};
    use std::ffi::CString;

    let (read_end, write_end) = pipe().map_err(|e| CliError::Io(std::io::Error::other(e)))?;

    // Extract raw fds before forking so both child processes can safely
    // reference them without Rust's ownership model fighting the fork.
    // SAFETY: we own these fds and will not let the OwnedFd drop them
    // after the fork -- each process closes the ends it does not use.
    let read_fd = read_end.into_raw_fd();
    let write_fd = write_end.into_raw_fd();

    // SAFETY: fork doubles the process. The forked child observes everything
    // the parent saw; we keep both branches small and free of shared mutable
    // state across the fork.
    match unsafe { fork().map_err(|e| CliError::Io(std::io::Error::other(e)))? } {
        ForkResult::Parent { child: _ } => {
            close(write_fd).ok();
            let mut buf = [0u8; 16];
            let n = read(read_fd, &mut buf)
                .map_err(|e| CliError::Io(std::io::Error::other(e)))?;
            close(read_fd).ok();
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
            close(read_fd).ok();
            setsid().map_err(|e| CliError::Io(std::io::Error::other(e)))?;
            // SAFETY: same as the outer fork; child is small.
            match unsafe { fork().map_err(|e| CliError::Io(std::io::Error::other(e)))? } {
                ForkResult::Parent { child: grandchild } => {
                    // Report grandchild PID to the original parent, then exit.
                    let pid_str = format!("{}", grandchild.as_raw());
                    // SAFETY: write_fd is valid; we created it above.
                    let write_owned = unsafe {
                        std::os::fd::OwnedFd::from_raw_fd(write_fd)
                    };
                    write(&write_owned, pid_str.as_bytes())
                        .map_err(|e| CliError::Io(std::io::Error::other(e)))?;
                    std::process::exit(0);
                }
                ForkResult::Child => {
                    close(write_fd).ok();
                    let log = std::fs::OpenOptions::new().create(true).append(true)
                        .open(log_path).map_err(CliError::Io)?;
                    dup2(log.as_raw_fd(), 1).ok();
                    dup2(log.as_raw_fd(), 2).ok();
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
