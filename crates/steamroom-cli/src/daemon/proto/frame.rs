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
