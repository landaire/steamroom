use super::JobId;
use super::JobKind;
use super::ProgressUpdate;
use rkyv::Archive;
use rkyv::Deserialize;
use rkyv::Serialize;

#[derive(Archive, Serialize, Deserialize, Debug, Clone, serde::Serialize, serde::Deserialize)]
#[rkyv(derive(Debug))]
pub struct JobRecord {
    pub job_id: JobId,
    pub kind: JobKind,
    pub args_summary: String,
    pub priority: bool,
    pub submitted_at: u64,
    pub started_at: Option<u64>,
    pub finished_at: Option<u64>,
    pub exit_code: Option<i32>,
    pub progress: Option<ProgressUpdate>,
}

#[derive(Archive, Serialize, Deserialize, Debug, Clone)]
#[rkyv(derive(Debug))]
pub struct StatusSnapshot {
    pub daemon_pid: u32,
    pub daemon_started_at: u64,
    pub account: Option<String>,
    pub active: Option<JobRecord>,
    pub queue: Vec<JobRecord>,
    pub recent: Vec<JobRecord>,
}
