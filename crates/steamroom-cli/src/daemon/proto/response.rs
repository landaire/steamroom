use super::JobId;
use super::status::StatusSnapshot;
use rkyv::Archive;
use rkyv::Deserialize;
use rkyv::Serialize;

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
