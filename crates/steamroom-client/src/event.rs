/// Events emitted during a depot download. Subscribe via
/// [`DepotJobBuilder::event_sender`](crate::download::DepotJobBuilder::event_sender)
/// to drive progress bars, logging, or any other UI.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub enum DownloadEvent {
    DownloadStarted {
        total_bytes: u64,
        total_files: u64,
    },
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
        error: ErrorChain,
    },
    DepotProgress {
        completed_bytes: u64,
        total_bytes: u64,
    },
}

/// Flattened, serializable view of a `std::error::Error` source chain.
///
/// Events are forwarded across process boundaries (daemon RPC, UI) where the
/// concrete error types are not available, but the human-readable chain is
/// what consumers want to display. `From<&E>` walks `Error::source` so each
/// link is preserved instead of being collapsed to `to_string()` on the head.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ErrorChain {
    pub messages: Vec<String>,
}

impl ErrorChain {
    pub fn from_error<E: std::error::Error + ?Sized>(err: &E) -> Self {
        let mut messages = vec![err.to_string()];
        let mut source = err.source();
        while let Some(s) = source {
            messages.push(s.to_string());
            source = s.source();
        }
        Self { messages }
    }
}

impl std::fmt::Display for ErrorChain {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (i, m) in self.messages.iter().enumerate() {
            if i > 0 {
                write!(f, ": ")?;
            }
            write!(f, "{m}")?;
        }
        Ok(())
    }
}
