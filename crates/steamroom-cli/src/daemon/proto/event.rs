use rkyv::{Archive, Deserialize, Serialize};
use super::status::StatusSnapshot;
use super::{JobId, JobKind, LogLevel, ProgressUpdate};

#[derive(Archive, Serialize, Deserialize, Debug, Clone)]
#[rkyv(derive(Debug))]
pub enum Event {
    JobStarted { job_id: JobId, kind: JobKind, args_summary: String },
    JobFinished { job_id: JobId, exit_code: i32 },
    Log { job_id: Option<JobId>, level: LogLevel, target: String, message: String },
    Progress { job_id: JobId, update: ProgressUpdate },
    Stdout { job_id: JobId, line: String },
    QueueChanged { snapshot: StatusSnapshot },
}

impl Event {
    /// `Some(job_id)` for events scoped to a specific job, `None` for
    /// daemon-wide events (`Log { job_id: None, .. }`, `QueueChanged`).
    pub fn job_id(&self) -> Option<JobId> {
        match self {
            Event::JobStarted { job_id, .. }
            | Event::JobFinished { job_id, .. }
            | Event::Progress { job_id, .. }
            | Event::Stdout { job_id, .. } => Some(*job_id),
            Event::Log { job_id, .. } => *job_id,
            Event::QueueChanged { .. } => None,
        }
    }
}
