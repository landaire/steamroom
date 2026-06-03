//! Output sink abstraction shared by direct-mode CLI and the daemon
//! worker. Every `run_*` command in `commands/` writes results through a
//! `&dyn JobSink` so the same code paths produce stdout text in direct
//! mode and broadcast events in daemon mode.

use crate::daemon::proto::ProgressUpdate;

pub trait JobSink: Send + Sync {
    fn stdout_line(&self, line: &str);
    fn progress(&self, update: ProgressUpdate);
}

/// Direct-mode sink: writes to the inherited stdout and to `tracing`.
/// Progress updates are absorbed (direct mode wires the progress bar
/// separately through `download::spawn_progress_renderer`).
pub struct StdoutSink;

impl Default for StdoutSink {
    fn default() -> Self {
        Self::new()
    }
}

impl StdoutSink {
    pub fn new() -> Self {
        Self
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
}
