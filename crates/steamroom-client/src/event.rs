/// Events emitted during a depot download. Subscribe via
/// [`DepotJobBuilder::event_sender`](crate::download::DepotJobBuilder::event_sender)
/// to drive progress bars, logging, or any other UI.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum DownloadEvent {
    FileStarted {
        filename: String,
    },
    FileCompleted {
        filename: String,
    },
    FileSkipped {
        filename: String,
    },
    FileRemoved {
        filename: String,
    },
    ChunkCompleted {
        bytes: u64,
    },
    ChunkFailed {
        error: String,
    },
    DepotProgress {
        completed_bytes: u64,
        total_bytes: u64,
    },
}
