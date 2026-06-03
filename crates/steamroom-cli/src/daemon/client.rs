//! Client side of `--use-daemon`: connect, submit, attach to the event
//! stream, render events to stdout via the same formatting the direct
//! CLI uses.

use interprocess::local_socket::tokio::Stream;
use interprocess::local_socket::traits::tokio::Stream as _;

use crate::cli::Cli;
use crate::cli::DaemonSub;
use crate::cli::OutputFormat as CliOutputFormat;
use crate::daemon::framing::read_frame;
use crate::daemon::framing::write_frame;
use crate::daemon::ipc::socket_name;
use crate::daemon::proto::Event;
use crate::daemon::proto::Frame;
use crate::daemon::proto::JobId;
use crate::daemon::proto::JobRecord;
use crate::daemon::proto::LogLevel;
use crate::daemon::proto::Request;
use crate::daemon::proto::Response;
use crate::daemon::proto::StatusSnapshot;
use crate::errors::CliError;

pub async fn connect() -> Result<Stream, CliError> {
    let name = socket_name()?;
    Stream::connect(name).await.map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused => {
            CliError::NoDaemonRunning
        }
        _ => CliError::Io(e),
    })
}

pub async fn dispatch_use_daemon(cli: Cli) -> Result<(), CliError> {
    let detach = cli.detach;
    let quiet = cli.quiet;
    let no_progress = cli.no_progress;
    let request = cli.into_rpc_request()?;
    let mut stream = match connect().await {
        Ok(s) => s,
        Err(CliError::NoDaemonRunning) => {
            eprintln!("no daemon running; starting one in lazy auth mode...");
            auto_spawn_daemon().await?;
            connect().await?
        }
        Err(e) => return Err(e),
    };
    write_frame(&mut stream, &Frame::Request(request)).await?;

    let resp = read_frame(&mut stream).await?;
    let (job_id, position) = match resp {
        Frame::Response(Response::JobAccepted { job_id, position }) => (job_id, position),
        Frame::Response(Response::Error { kind, message }) => {
            return Err(CliError::DaemonError(format!("{kind:?}: {message}")));
        }
        other => {
            return Err(CliError::MalformedFrame(format!(
                "expected JobAccepted, got {other:?}"
            )));
        }
    };

    if detach {
        println!("job {} queued (position {})", job_id, position);
        return Ok(());
    }

    attach_loop(&mut stream, job_id, quiet, no_progress).await
}

/// Render attached events to the user's terminal. `quiet` suppresses
/// `Event::Stdout` lines (matching direct-mode `--quiet`). `no_progress`
/// suppresses the indicatif progress bar (matching direct-mode
/// `--no-progress`). `Event::Log` events are always forwarded through
/// `tracing` so the user's `--debug` / `--quiet` filter applies.
struct AttachRenderer {
    bar: Option<indicatif::ProgressBar>,
    quiet: bool,
    no_progress: bool,
}

impl AttachRenderer {
    fn new(quiet: bool, no_progress: bool) -> Self {
        Self {
            bar: None,
            quiet,
            no_progress,
        }
    }

    fn handle(&mut self, ev: Event) {
        match ev {
            Event::Stdout { line, .. } => {
                if !self.quiet {
                    println!("{line}");
                }
            }
            Event::Log {
                level,
                target,
                message,
                ..
            } => emit_log(level, &target, &message),
            Event::Progress { update, .. } => {
                if self.no_progress {
                    return;
                }
                let bar = self.bar.get_or_insert_with(|| {
                    let pb = indicatif::ProgressBar::new(update.bytes_total);
                    pb.set_style(
                        indicatif::ProgressStyle::default_bar()
                            .template("{spinner:.green} [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({bytes_per_sec}, {eta})")
                            .unwrap()
                            .progress_chars("=> "),
                    );
                    pb
                });
                bar.set_length(update.bytes_total);
                bar.set_position(update.bytes_done);
            }
            Event::JobFinished { .. } => {
                if let Some(bar) = self.bar.take() {
                    bar.finish_and_clear();
                }
            }
            Event::JobStarted { .. } | Event::QueueChanged { .. } => {}
        }
    }
}

/// Stream events from a connection and render them as the direct CLI
/// would. Returns when EndOfStream arrives, when Ctrl-C detaches, or
/// when the socket closes cleanly.
pub async fn attach_loop(
    stream: &mut Stream,
    job_id: JobId,
    quiet: bool,
    no_progress: bool,
) -> Result<(), CliError> {
    let mut renderer = AttachRenderer::new(quiet, no_progress);
    let ctrl_c = tokio::signal::ctrl_c();
    tokio::pin!(ctrl_c);

    loop {
        let frame_fut = read_frame(stream);
        tokio::pin!(frame_fut);
        tokio::select! {
            _ = &mut ctrl_c => {
                eprintln!("detached -- reattach with `steamroom daemon attach {}`", job_id.0);
                return Ok(());
            }
            r = &mut frame_fut => match r {
                Ok(Frame::Event(ev)) => renderer.handle(ev),
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

fn emit_log(level: LogLevel, target: &str, message: &str) {
    match level {
        LogLevel::Error => tracing::error!(target: "daemon", "{target}: {message}"),
        LogLevel::Warn => tracing::warn!(target: "daemon", "{target}: {message}"),
        LogLevel::Info => tracing::info!(target: "daemon", "{target}: {message}"),
        LogLevel::Debug => tracing::debug!(target: "daemon", "{target}: {message}"),
        LogLevel::Trace => tracing::trace!(target: "daemon", "{target}: {message}"),
    }
}

// -- Subcommand handlers (T20) ----------------------------------------

pub async fn run_daemon_subcommand(
    sub: DaemonSub,
    quiet: bool,
    no_progress: bool,
) -> Result<(), CliError> {
    match sub {
        // `Start` is intercepted in main() before the tokio runtime is built,
        // because launching the daemon must fork after dropping the runtime.
        // Reaching this arm means the dispatcher missed it.
        DaemonSub::Start => unreachable!("daemon start is dispatched in main() pre-runtime"),
        DaemonSub::Info => {
            crate::daemon::lifecycle::render_daemon_info();
            Ok(())
        }
        DaemonSub::Stop { force } => stop_daemon(force).await,
        DaemonSub::Attach { job_id } => attach_existing(JobId(job_id), quiet, no_progress).await,
        DaemonSub::Status { text, format } => {
            // Any explicit format implies text mode -- the TUI doesn't
            // render JSON / plain / table choices, only its own widgets.
            // When built without the `tui` feature, fall back to a text
            // snapshot regardless of which flags were passed.
            #[cfg(feature = "tui")]
            {
                if text || format.is_some() {
                    print_status_once(format).await
                } else {
                    crate::daemon::tui::run_tui().await
                }
            }
            #[cfg(not(feature = "tui"))]
            {
                let _ = text; // both flags are honored as "print once" here
                print_status_once(format).await
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
        other => Err(CliError::MalformedFrame(format!(
            "expected Stopping, got {other:?}"
        ))),
    }
}

async fn attach_existing(job_id: JobId, quiet: bool, no_progress: bool) -> Result<(), CliError> {
    let mut stream = connect().await?;
    write_frame(&mut stream, &Frame::Request(Request::Attach { job_id })).await?;
    attach_loop(&mut stream, job_id, quiet, no_progress).await
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
        other => {
            return Err(CliError::MalformedFrame(format!(
                "expected Status, got {other:?}"
            )));
        }
    };
    match format {
        Some(CliOutputFormat::Json) => print_status_json(&snap),
        _ => print_status_table(&snap),
    }
    Ok(())
}

fn print_status_json(snap: &StatusSnapshot) {
    let json = serde_json::json!({
        "daemon_pid": snap.daemon_pid,
        "daemon_started_at": snap.daemon_started_at,
        "account": snap.account,
        "active": snap.active.as_ref().map(record_to_json),
        "queue": snap.queue.iter().map(record_to_json).collect::<Vec<_>>(),
        "recent": snap.recent.iter().map(record_to_json).collect::<Vec<_>>(),
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&json).expect("snapshot is JSON-clean")
    );
}

fn record_to_json(r: &JobRecord) -> serde_json::Value {
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
    println!(
        "account    : {}",
        snap.account.as_deref().unwrap_or("(none)")
    );
    println!();
    match &snap.active {
        Some(active) => {
            println!("Active:");
            println!(
                "  {} ({:?}) {}",
                active.job_id, active.kind, active.args_summary
            );
        }
        None => println!("Active: (idle)"),
    }
    if !snap.queue.is_empty() {
        println!();
        println!("Queue:");
        for j in &snap.queue {
            let mark = if j.priority { "*" } else { " " };
            println!("  {} {} ({:?}) {}", mark, j.job_id, j.kind, j.args_summary);
        }
    }
    if !snap.recent.is_empty() {
        println!();
        println!("Recent:");
        for j in &snap.recent {
            let ec = j.exit_code.map(|c| format!("exit {c}")).unwrap_or_default();
            println!("  {} ({:?}) {} {}", j.job_id, j.kind, j.args_summary, ec);
        }
    }
}

/// Auto-spawn a lazy-auth daemon when `--use-daemon` finds no running
/// daemon. Runs `steamroom daemon start` as a subprocess; that process
/// does its own fork+exec dance, probes the socket until bound, and
/// exits 0. Returning Ok here guarantees the socket is up.
async fn auto_spawn_daemon() -> Result<(), CliError> {
    use std::process::Stdio;
    let exe = std::env::current_exe().map_err(CliError::Io)?;
    let status = tokio::process::Command::new(exe)
        .args(["daemon", "start"])
        .stdin(Stdio::null())
        // The spawned `daemon start` would otherwise print its info
        // banner to our stdout, which mixes with the user's --use-daemon
        // output. Suppress both streams; the daemon log file has the
        // same content.
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .await
        .map_err(CliError::Io)?;
    if !status.success() {
        return Err(CliError::DaemonError(format!(
            "auto-spawn `daemon start` exited with status {status}"
        )));
    }
    Ok(())
}
