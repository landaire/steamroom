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
    pub fn show_progress(&self) -> bool {
        self.show_progress
    }
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
