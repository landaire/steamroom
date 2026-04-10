#[derive(Clone, Debug)]
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
