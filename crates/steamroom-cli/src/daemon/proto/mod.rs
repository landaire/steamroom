//! Wire types for the daemon RPC. Owned, rkyv-archivable; never contain
//! `PathBuf`, `Regex`, or other types that rkyv cannot archive directly.

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
}
