//! Daemon-side state, worker loop, and connection task. Decoupled from
//! socket I/O so the queue and dispatch logic can be unit-tested with
//! plain method calls.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::{Mutex, broadcast};
use tokio_util::sync::CancellationToken;

use crate::daemon::proto::{
    Event, JobId, JobKind, JobRecord, LogLevel, ProgressUpdate, Request, StatusSnapshot,
};

/// One unit of work waiting to run. The `request` carries the parameters;
/// `priority` is also reflected here so the queue can rebalance on
/// `TogglePriority` without re-reading the Request.
pub struct QueuedJob {
    pub job_id: JobId,
    pub kind: JobKind,
    pub request: Request,
    pub priority: bool,
    pub submitted_at: u64,
    pub cancel: CancellationToken,
    pub args_summary: String,
}

pub struct RunningJob {
    pub record: JobRecord,
    pub cancel: CancellationToken,
}

pub struct DaemonState {
    pub queue: Mutex<VecDeque<QueuedJob>>,
    pub active: Mutex<Option<RunningJob>>,
    pub recent: Mutex<RingBuffer<JobRecord>>,
    pub events: broadcast::Sender<Event>,
    pub next_job_id: AtomicU64,
    pub shutdown: CancellationToken,
    pub started_at: u64,
    pub daemon_pid: u32,
    pub account: Option<String>,
    pub queue_notify: tokio::sync::Notify,
}

impl DaemonState {
    pub fn new(account: Option<String>, daemon_pid: u32, started_at: u64) -> Arc<Self> {
        // Channel depth: enough to buffer one chunky download's worth of
        // progress events between worker emit and subscriber drain. Lag
        // is tolerated; clients reading slower than the worker writes
        // see `broadcast::error::RecvError::Lagged` and skip ahead.
        let (events, _) = broadcast::channel(512);
        Arc::new(Self {
            queue: Mutex::new(VecDeque::new()),
            active: Mutex::new(None),
            recent: Mutex::new(RingBuffer::new(32)),
            events,
            next_job_id: AtomicU64::new(1),
            shutdown: CancellationToken::new(),
            started_at,
            daemon_pid,
            account,
            queue_notify: tokio::sync::Notify::new(),
        })
    }

    pub fn allocate_job_id(&self) -> JobId {
        JobId(self.next_job_id.fetch_add(1, Ordering::Relaxed))
    }

    /// Insert a job. Priority items sit at the boundary between existing
    /// priority items and non-priority items; non-priority items append.
    /// Returns the queue position the caller should report: 0 if it will
    /// run next (queue empty and no active), 1+ otherwise. The "active"
    /// job is NOT counted because it is already running.
    pub async fn enqueue(&self, job: QueuedJob) -> u32 {
        let mut q = self.queue.lock().await;
        let position = if job.priority {
            let boundary = q.iter().take_while(|j| j.priority).count();
            q.insert(boundary, job);
            boundary as u32
        } else {
            let pos = q.len() as u32;
            q.push_back(job);
            pos
        };
        self.queue_notify.notify_one();
        let snap = self.snapshot_inner(&q, None).await;
        let _ = self.events.send(Event::QueueChanged { snapshot: snap });
        position
    }

    pub async fn toggle_priority(&self, job_id: JobId) -> Result<(), JobNotFound> {
        let mut q = self.queue.lock().await;
        let Some(idx) = q.iter().position(|j| j.job_id == job_id) else {
            return Err(JobNotFound);
        };
        let mut job = q.remove(idx).expect("index just found");
        job.priority = !job.priority;
        if job.priority {
            let boundary = q.iter().take_while(|j| j.priority).count();
            q.insert(boundary, job);
        } else {
            q.push_back(job);
        }
        let snap = self.snapshot_inner(&q, None).await;
        let _ = self.events.send(Event::QueueChanged { snapshot: snap });
        Ok(())
    }

    pub async fn cancel(&self, job_id: JobId) -> Result<(), JobNotFound> {
        // Active first.
        if let Some(running) = self.active.lock().await.as_ref() {
            if running.record.job_id == job_id {
                running.cancel.cancel();
                return Ok(());
            }
        }
        // Queued: remove and emit JobFinished with exit_code 130 (SIGINT-ish).
        let mut q = self.queue.lock().await;
        let Some(idx) = q.iter().position(|j| j.job_id == job_id) else {
            return Err(JobNotFound);
        };
        let removed = q.remove(idx).expect("index just found");
        let _ = self.events.send(Event::JobFinished { job_id: removed.job_id, exit_code: 130 });
        let snap = self.snapshot_inner(&q, None).await;
        let _ = self.events.send(Event::QueueChanged { snapshot: snap });
        Ok(())
    }

    async fn snapshot_inner(
        &self,
        queue: &VecDeque<QueuedJob>,
        active_override: Option<&RunningJob>,
    ) -> StatusSnapshot {
        let active = match active_override {
            Some(r) => Some(r.record.clone()),
            None => self.active.lock().await.as_ref().map(|r| r.record.clone()),
        };
        StatusSnapshot {
            daemon_pid: self.daemon_pid,
            daemon_started_at: self.started_at,
            account: self.account.clone(),
            active,
            queue: queue.iter().map(job_record_for_queued).collect(),
            recent: self.recent.lock().await.iter().cloned().collect(),
        }
    }

    pub async fn snapshot(&self) -> StatusSnapshot {
        let q = self.queue.lock().await;
        self.snapshot_inner(&q, None).await
    }
}

fn job_record_for_queued(j: &QueuedJob) -> JobRecord {
    JobRecord {
        job_id: j.job_id,
        kind: j.kind,
        args_summary: j.args_summary.clone(),
        priority: j.priority,
        submitted_at: j.submitted_at,
        started_at: None,
        finished_at: None,
        exit_code: None,
        progress: None,
    }
}

#[derive(Debug)]
pub struct JobNotFound;

pub struct RingBuffer<T> {
    cap: usize,
    items: VecDeque<T>,
}

impl<T> RingBuffer<T> {
    pub fn new(cap: usize) -> Self { Self { cap, items: VecDeque::with_capacity(cap) } }
    pub fn push(&mut self, v: T) {
        if self.items.len() == self.cap { self.items.pop_front(); }
        self.items.push_back(v);
    }
    pub fn iter(&self) -> impl Iterator<Item = &T> { self.items.iter() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::proto::{InfoParams, OutputFormat};

    fn fake_queued(state: &DaemonState, priority: bool) -> QueuedJob {
        QueuedJob {
            job_id: state.allocate_job_id(),
            kind: JobKind::Info,
            request: Request::Info {
                args: InfoParams { app: 1, format: Some(OutputFormat::Plain), os: None, show_all: false },
                priority,
            },
            priority,
            submitted_at: 0,
            cancel: CancellationToken::new(),
            args_summary: "fake".into(),
        }
    }

    #[tokio::test]
    async fn enqueue_returns_position_zero_for_empty_queue() {
        let s = DaemonState::new(None, 1, 0);
        let pos = s.enqueue(fake_queued(&s, false)).await;
        assert_eq!(pos, 0);
    }

    #[tokio::test]
    async fn priority_jumps_non_priority() {
        let s = DaemonState::new(None, 1, 0);
        let _ = s.enqueue(fake_queued(&s, false)).await;
        let _ = s.enqueue(fake_queued(&s, false)).await;
        let prio_pos = s.enqueue(fake_queued(&s, true)).await;
        assert_eq!(prio_pos, 0, "first priority should land at the head");

        let snap = s.snapshot().await;
        let kinds: Vec<bool> = snap.queue.iter().map(|j| j.priority).collect();
        assert_eq!(kinds, vec![true, false, false]);
    }

    #[tokio::test]
    async fn cancel_queued_removes_and_emits_finished() {
        let s = DaemonState::new(None, 1, 0);
        let mut rx = s.events.subscribe();
        let _ = s.enqueue(fake_queued(&s, false)).await;
        let snap = s.snapshot().await;
        let target = snap.queue[0].job_id;
        s.cancel(target).await.expect("ok");
        let mut saw_finished = false;
        while let Ok(ev) = rx.try_recv() {
            if let Event::JobFinished { job_id, exit_code } = ev {
                assert_eq!(job_id, target);
                assert_eq!(exit_code, 130);
                saw_finished = true;
            }
        }
        assert!(saw_finished, "expected JobFinished after cancel");
    }

    #[tokio::test]
    async fn toggle_priority_moves_across_boundary() {
        let s = DaemonState::new(None, 1, 0);
        let _ = s.enqueue(fake_queued(&s, true)).await;
        let _ = s.enqueue(fake_queued(&s, false)).await;
        let target = s.snapshot().await.queue[1].job_id;
        s.toggle_priority(target).await.expect("ok");
        let kinds: Vec<bool> = s.snapshot().await.queue.iter().map(|j| j.priority).collect();
        assert_eq!(kinds, vec![true, true]);
    }
}

use crate::sink::JobSink;
use steamroom::client::{LoggedIn, SteamClient};

/// Daemon-side JobSink that translates every call into an Event and
/// broadcasts it. Cheap to construct per job.
pub struct BroadcastSink {
    pub job_id: JobId,
    pub events: broadcast::Sender<Event>,
}

impl JobSink for BroadcastSink {
    fn stdout_line(&self, line: &str) {
        let _ = self.events.send(Event::Stdout { job_id: self.job_id, line: line.to_string() });
    }
    fn progress(&self, update: ProgressUpdate) {
        let _ = self.events.send(Event::Progress { job_id: self.job_id, update });
    }
    fn log(&self, level: LogLevel, target: &str, message: &str) {
        let _ = self.events.send(Event::Log {
            job_id: Some(self.job_id),
            level,
            target: target.to_string(),
            message: message.to_string(),
        });
    }
}

async fn wait_for_next_job(state: &DaemonState) -> Option<QueuedJob> {
    loop {
        if state.shutdown.is_cancelled() {
            return None;
        }
        {
            let mut q = state.queue.lock().await;
            if let Some(job) = q.pop_front() {
                return Some(job);
            }
        }
        tokio::select! {
            _ = state.queue_notify.notified() => {}
            _ = state.shutdown.cancelled() => return None,
        }
    }
}

fn unix_now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        // Zero on error is acceptable: timestamps are advisory for display.
        .unwrap_or(0)
}

/// Single-job worker loop. Owns the authenticated SteamClient and runs
/// it through every `run_*` dispatch.
pub async fn worker_loop(state: Arc<DaemonState>, client: SteamClient<LoggedIn>) {
    while let Some(job) = wait_for_next_job(&state).await {
        let started_at = unix_now();
        let sink = BroadcastSink { job_id: job.job_id, events: state.events.clone() };
        let record = JobRecord {
            job_id: job.job_id,
            kind: job.kind,
            args_summary: job.args_summary.clone(),
            priority: job.priority,
            submitted_at: job.submitted_at,
            started_at: Some(started_at),
            finished_at: None,
            exit_code: None,
            progress: None,
        };
        {
            let mut active = state.active.lock().await;
            *active = Some(RunningJob { record: record.clone(), cancel: job.cancel.clone() });
        }
        let _ = state.events.send(Event::JobStarted {
            job_id: job.job_id,
            kind: job.kind,
            args_summary: job.args_summary.clone(),
        });

        let exit_code = dispatch(job.request, client.clone(), &sink, job.cancel.clone()).await;

        {
            let mut active = state.active.lock().await;
            *active = None;
        }
        let mut finished = record;
        finished.finished_at = Some(unix_now());
        finished.exit_code = Some(exit_code);
        state.recent.lock().await.push(finished);
        let _ = state.events.send(Event::JobFinished { job_id: job.job_id, exit_code });
    }
}

async fn dispatch(
    req: Request,
    client: SteamClient<LoggedIn>,
    sink: &dyn JobSink,
    cancel: CancellationToken,
) -> i32 {
    use crate::commands;
    let result = match req {
        Request::Download { args, .. } => {
            // show_progress=false: progress flows via sink.progress, not indicatif.
            commands::download::run_download(args.into(), client, sink, cancel, false).await
        }
        Request::Info { args, .. } => {
            commands::info::run_info(args.into(), client, sink, cancel).await
        }
        Request::Files { args, .. } => {
            commands::files::run_files(args.into(), Some(client), sink, cancel).await
        }
        Request::Manifests { args, .. } => {
            commands::manifests::run_manifests(args.into(), client, sink, cancel).await
        }
        Request::Diff { args, .. } => {
            commands::diff::run_diff(args.into(), client, sink, cancel).await
        }
        Request::Packages { args, .. } => {
            commands::packages::run_packages(args.into(), client, sink, cancel).await
        }
        Request::SaveManifest { args, .. } => {
            commands::save_manifest::run_save_manifest(args.into(), client, sink, cancel).await
        }
        Request::Workshop { args, .. } => {
            // show_progress=false: progress flows via sink.progress, not indicatif.
            commands::workshop::run_workshop(args.into(), client, sink, cancel, false).await
        }
        Request::LocalInfo { args, .. } => {
            commands::local_info::run_local_info(args.into(), sink, cancel).await
        }
        Request::Status
        | Request::Subscribe
        | Request::Attach { .. }
        | Request::Cancel { .. }
        | Request::TogglePriority { .. }
        | Request::Stop { .. } => {
            // Control variants are handled by handle_connection (T14), not dispatched as jobs.
            unreachable!("control variants do not produce jobs");
        }
    };
    match result {
        Ok(()) => 0,
        Err(crate::errors::CliError::Cancelled) => 130,
        Err(_) => 1,
    }
}
