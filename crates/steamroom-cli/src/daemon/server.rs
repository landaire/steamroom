//! Daemon-side state, worker loop, and connection task. Decoupled from
//! socket I/O so the queue and dispatch logic can be unit-tested with
//! plain method calls.

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use tokio::sync::{Mutex, broadcast};
use tokio_util::sync::CancellationToken;

/// How long a graceful `daemon stop` waits for the active job before
/// signalling the job's cancel token. Matches the spec's documented
/// behavior so a runaway job can't pin the daemon indefinitely.
const GRACEFUL_STOP_GRACE: std::time::Duration = std::time::Duration::from_secs(30);

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
    /// Per-job event history for `attach <id>` to a finished job. The
    /// collector task (spawned in `serve_resumed`) populates this from
    /// the broadcast channel; the Attach handler dumps it before
    /// EndOfStream. Bounded per-job and across jobs so a runaway
    /// process can't OOM the daemon.
    pub replay: Mutex<ReplayBuffer>,
    pub events: broadcast::Sender<Event>,
    pub next_job_id: AtomicU64,
    /// Set when Stop is received. The accept loop and worker_loop check
    /// this to refuse new jobs and exit when idle. Active connections
    /// keep streaming until their job completes.
    pub accepting: CancellationToken,
    /// Set for a hard stop (force=true) or when the queue drains after a
    /// graceful stop. Signals all subscribers and the accept loop to exit.
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
            replay: Mutex::new(ReplayBuffer::new(32, 200)),
            events,
            next_job_id: AtomicU64::new(1),
            accepting: CancellationToken::new(),
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
        // Pull the active job's cancel token under a scoped guard so the
        // active lock is released before we take the queue lock.
        let active_token = {
            let guard = self.active.lock().await;
            guard.as_ref().and_then(|r| {
                (r.record.job_id == job_id).then(|| r.cancel.clone())
            })
        };
        if let Some(token) = active_token {
            token.cancel();
            return Ok(());
        }
        // Not active; try queued.
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

    /// Snapshot + broadcast on the events channel. Called whenever the
    /// queue, active job, or recent ring changes so Subscribe stream
    /// clients (the TUI, daemon status) refresh without re-polling.
    pub async fn broadcast_snapshot(&self) {
        let snap = self.snapshot().await;
        let _ = self.events.send(Event::QueueChanged { snapshot: snap });
    }

    /// Look up a recently-finished job by id. Used by `stream_events` to
    /// recover the terminal exit code after a broadcast lag.
    pub async fn recent_exit_code(&self, job_id: JobId) -> Option<i32> {
        self.recent.lock().await.iter()
            .find(|r| r.job_id == job_id)
            .and_then(|r| r.exit_code)
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

/// Per-job ring of `Event`s used to replay history when `daemon attach`
/// targets a finished or in-flight job. Bounded both in jobs tracked
/// (oldest job dropped when full) and in events per job (oldest event
/// per job dropped when full).
pub struct ReplayBuffer {
    entries: VecDeque<(JobId, VecDeque<Event>)>,
    cap_jobs: usize,
    cap_per_job: usize,
}

impl ReplayBuffer {
    pub fn new(cap_jobs: usize, cap_per_job: usize) -> Self {
        Self { entries: VecDeque::with_capacity(cap_jobs), cap_jobs, cap_per_job }
    }

    /// Begin a new job's history. Drops the oldest tracked job if at
    /// capacity. Idempotent for repeated JobStarted events on the same
    /// id (which shouldn't happen but we tolerate it).
    pub fn start_job(&mut self, job_id: JobId) {
        if self.entries.iter().any(|(id, _)| *id == job_id) {
            return;
        }
        if self.entries.len() >= self.cap_jobs {
            self.entries.pop_front();
        }
        self.entries.push_back((job_id, VecDeque::with_capacity(64)));
    }

    /// Append an event to the named job's ring. Silently ignored if
    /// the job's entry has been evicted.
    pub fn append(&mut self, job_id: JobId, ev: Event) {
        if let Some((_, evs)) = self.entries.iter_mut().rev().find(|(id, _)| *id == job_id) {
            if evs.len() >= self.cap_per_job {
                evs.pop_front();
            }
            evs.push_back(ev);
        }
    }

    pub fn events_for(&self, job_id: JobId) -> Option<Vec<Event>> {
        self.entries
            .iter()
            .rev()
            .find(|(id, _)| *id == job_id)
            .map(|(_, evs)| evs.iter().cloned().collect())
    }
}

/// Long-running task: subscribes to the broadcast channel and routes
/// every per-job event into the replay buffer. Spawned once at daemon
/// startup. Exits when the broadcast sender drops.
pub async fn replay_collector(state: Arc<DaemonState>) {
    let mut rx = state.events.subscribe();
    loop {
        match rx.recv().await {
            Ok(ev) => {
                if let Some(job_id) = ev.job_id() {
                    let mut replay = state.replay.lock().await;
                    if matches!(&ev, Event::JobStarted { .. }) {
                        replay.start_job(job_id);
                    }
                    replay.append(job_id, ev);
                }
            }
            Err(broadcast::error::RecvError::Lagged(_)) => continue,
            Err(broadcast::error::RecvError::Closed) => return,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::daemon::proto::{InfoParams, OutputFormat};

    fn ev_stdout(job_id: JobId, line: &str) -> Event {
        Event::Stdout { job_id, line: line.into() }
    }

    #[test]
    fn replay_buffer_evicts_oldest_job_when_full() {
        let mut rb = ReplayBuffer::new(2, 10);
        rb.start_job(JobId(1));
        rb.start_job(JobId(2));
        rb.start_job(JobId(3)); // evicts JobId(1)
        assert!(rb.events_for(JobId(1)).is_none());
        assert!(rb.events_for(JobId(2)).is_some());
        assert!(rb.events_for(JobId(3)).is_some());
    }

    #[test]
    fn replay_buffer_caps_events_per_job() {
        let mut rb = ReplayBuffer::new(4, 3);
        rb.start_job(JobId(1));
        for i in 0..5 {
            rb.append(JobId(1), ev_stdout(JobId(1), &format!("line {i}")));
        }
        let events = rb.events_for(JobId(1)).expect("present");
        assert_eq!(events.len(), 3);
        // Oldest two were dropped; events 2,3,4 remain.
        match &events[0] {
            Event::Stdout { line, .. } => assert_eq!(line, "line 2"),
            _ => panic!("expected Stdout"),
        }
    }

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

    use tokio::io::duplex;

    #[tokio::test]
    async fn status_request_round_trips() {
        let s = DaemonState::new(Some("acct".into()), 42, 1000);
        let (mut client, server) = duplex(64 * 1024);
        let server_state = s.clone();
        let server_task = tokio::spawn(async move {
            handle_connection(server_state, server).await;
        });
        crate::daemon::framing::write_frame(&mut client, &Frame::Request(Request::Status)).await.unwrap();
        let resp = crate::daemon::framing::read_frame(&mut client).await.unwrap();
        match resp {
            Frame::Response(Response::Status(snap)) => {
                assert_eq!(snap.daemon_pid, 42);
                assert_eq!(snap.account.as_deref(), Some("acct"));
            }
            other => panic!("wrong: {other:?}"),
        }
        server_task.await.unwrap();
    }

    #[tokio::test]
    async fn cancel_active_job_releases_active_lock_promptly() {
        // Construct a DaemonState with a fake active job.
        let s = DaemonState::new(None, 1, 0);
        let active_cancel = CancellationToken::new();
        let record = JobRecord {
            job_id: JobId(99),
            kind: JobKind::Info,
            args_summary: "fake".into(),
            priority: false,
            submitted_at: 0,
            started_at: Some(0),
            finished_at: None,
            exit_code: None,
            progress: None,
        };
        *s.active.lock().await = Some(RunningJob { record, cancel: active_cancel.clone() });

        // Cancel should return quickly (no lock held across the queue lock).
        let res = tokio::time::timeout(
            std::time::Duration::from_millis(100),
            s.cancel(JobId(99)),
        ).await;
        assert!(res.is_ok(), "cancel timed out -- deadlock?");
        assert!(active_cancel.is_cancelled());
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
            if state.accepting.is_cancelled() {
                // Graceful Stop: queue empty, no more work coming.
                return None;
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
        // The job has moved from the queue to active. Subscribe streams
        // (TUI, status monitors) need a fresh snapshot to drop the queue
        // entry; QueueChanged is the only event they treat as
        // queue/active state-of-the-world.
        state.broadcast_snapshot().await;

        let sink: Arc<dyn JobSink> = Arc::new(sink);
        use futures::future::FutureExt;
        use tracing::Instrument;
        let dispatch_fut = dispatch(job.request, client.clone(), sink.clone(), job.cancel.clone())
            .instrument(tracing::info_span!("job", job_id = job.job_id.0));
        let dispatch_result = match std::panic::AssertUnwindSafe(dispatch_fut).catch_unwind().await {
            Ok(res) => res,
            Err(payload) => {
                let msg = if let Some(s) = payload.downcast_ref::<&str>() {
                    s.to_string()
                } else if let Some(s) = payload.downcast_ref::<String>() {
                    s.clone()
                } else {
                    "(non-string panic payload)".into()
                };
                let _ = state.events.send(Event::Log {
                    job_id: Some(job.job_id),
                    level: LogLevel::Error,
                    target: "daemon::worker".into(),
                    message: format!("job panicked: {msg}"),
                });
                Err(crate::errors::CliError::Io(std::io::Error::other("job panicked")))
            }
        };
        let exit_code = match dispatch_result {
            Ok(()) => 0,
            Err(crate::errors::CliError::Cancelled) => 130,
            Err(e) => {
                let _ = state.events.send(Event::Log {
                    job_id: Some(job.job_id),
                    level: LogLevel::Error,
                    target: "daemon::worker".into(),
                    message: format!("{e}"),
                });
                1
            }
        };

        {
            let mut active = state.active.lock().await;
            *active = None;
        }
        let mut finished = record;
        finished.finished_at = Some(unix_now());
        finished.exit_code = Some(exit_code);
        state.recent.lock().await.push(finished);
        let _ = state.events.send(Event::JobFinished { job_id: job.job_id, exit_code });
        // active cleared, recent grew -- refresh subscribed clients.
        state.broadcast_snapshot().await;
    }
    // If we got here because accepting was cancelled and the queue is empty,
    // signal full shutdown so the accept loop and any Subscribe streams wind down.
    if state.accepting.is_cancelled() {
        state.shutdown.cancel();
    }
}

async fn dispatch(
    req: Request,
    client: SteamClient<LoggedIn>,
    sink: Arc<dyn JobSink>,
    cancel: CancellationToken,
) -> Result<(), crate::errors::CliError> {
    use crate::commands;
    match req {
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
    }
}

use tokio::io::{AsyncRead, AsyncWrite};

use crate::daemon::framing::{read_frame, write_frame};
use crate::daemon::proto::{ErrorKind, Frame, Response};

/// Handle a single client connection. Reads exactly one Request, then
/// either replies with a single Response and closes (control RPCs), or
/// streams Events filtered by job id (job submissions, Subscribe, Attach)
/// until terminated by JobFinished or a shutdown signal.
pub async fn handle_connection<S>(state: Arc<DaemonState>, mut stream: S)
where
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    let req = match read_frame(&mut stream).await {
        Ok(Frame::Request(r)) => r,
        Ok(other) => {
            let _ = write_frame(&mut stream, &Frame::Response(Response::Error {
                kind: ErrorKind::InvalidRequest,
                message: format!("expected Request, got {other:?}"),
            })).await;
            return;
        }
        Err(e) => {
            let _ = write_frame(&mut stream, &Frame::Response(Response::Error {
                kind: ErrorKind::InvalidRequest,
                message: e.to_string(),
            })).await;
            return;
        }
    };

    match req {
        Request::Status => {
            let snap = state.snapshot().await;
            let _ = write_frame(&mut stream, &Frame::Response(Response::Status(snap))).await;
        }
        Request::Stop { force } => {
            state.accepting.cancel();           // refuse new submissions
            state.queue_notify.notify_one();    // unstick worker if idle
            if force {
                // Hard stop: cancel active job and tell all subscribers.
                if let Some(running) = state.active.lock().await.as_ref() {
                    running.cancel.cancel();
                }
                state.shutdown.cancel();
            } else {
                // Graceful: do NOT signal `shutdown` here. The
                // worker_loop will set shutdown once active is None AND
                // queue is empty. Spawn a grace-timer task that cancels
                // the active job (whichever job is active at that
                // moment) after 30s so a 6-hour download can't pin the
                // daemon forever.
                let state = state.clone();
                tokio::spawn(async move {
                    tokio::time::sleep(GRACEFUL_STOP_GRACE).await;
                    if state.shutdown.is_cancelled() {
                        // Already fully shut down (queue drained); nothing to do.
                        return;
                    }
                    if let Some(running) = state.active.lock().await.as_ref() {
                        let _ = state.events.send(Event::Log {
                            job_id: Some(running.record.job_id),
                            level: LogLevel::Warn,
                            target: "daemon::stop".into(),
                            message: format!(
                                "graceful stop grace ({}s) elapsed; cancelling active job",
                                GRACEFUL_STOP_GRACE.as_secs()
                            ),
                        });
                        running.cancel.cancel();
                    }
                });
            }
            let _ = write_frame(&mut stream, &Frame::Response(Response::Stopping)).await;
        }
        Request::Cancel { job_id } => {
            let resp = match state.cancel(job_id).await {
                Ok(()) => Response::Ack,
                Err(_) => Response::Error { kind: ErrorKind::JobNotFound, message: format!("{job_id}") },
            };
            let _ = write_frame(&mut stream, &Frame::Response(resp)).await;
        }
        Request::TogglePriority { job_id } => {
            let resp = match state.toggle_priority(job_id).await {
                Ok(()) => Response::Ack,
                Err(_) => Response::Error { kind: ErrorKind::JobNotFound, message: format!("{job_id}") },
            };
            let _ = write_frame(&mut stream, &Frame::Response(resp)).await;
        }
        Request::Subscribe => {
            let rx = state.events.subscribe();
            stream_events(state.clone(), &mut stream, None, rx).await;
        }
        Request::Attach { job_id } => {
            let active_match = state.active.lock().await.as_ref()
                .map(|r| r.record.job_id == job_id).unwrap_or(false);
            let queued = state.queue.lock().await.iter().any(|j| j.job_id == job_id);

            if !active_match && !queued {
                // Either finished or never existed. If finished, replay
                // the buffered events before EndOfStream so the user
                // sees stdout/log/progress, not just an exit code.
                if let Some(exit_code) = state.recent_exit_code(job_id).await {
                    let events = state.replay.lock().await.events_for(job_id);
                    if let Some(events) = events {
                        for ev in events {
                            if write_frame(&mut stream, &Frame::Event(ev)).await.is_err() {
                                return;
                            }
                        }
                    }
                    let _ = write_frame(&mut stream, &Frame::EndOfStream { exit_code }).await;
                } else {
                    let _ = write_frame(&mut stream, &Frame::Response(Response::Error {
                        kind: ErrorKind::JobNotFound,
                        message: format!("{job_id}"),
                    })).await;
                }
                return;
            }
            // In-flight: replay everything we have so the user sees what
            // happened before they attached, then continue live.
            let replayed = state.replay.lock().await.events_for(job_id);
            if let Some(events) = replayed {
                for ev in events {
                    if write_frame(&mut stream, &Frame::Event(ev)).await.is_err() {
                        return;
                    }
                }
            }
            let rx = state.events.subscribe();
            stream_events(state.clone(), &mut stream, Some(job_id), rx).await;
        }
        // Job submissions.
        other => {
            let priority = matches!(&other,
                Request::Download { priority: true, .. }
                | Request::Info { priority: true, .. }
                | Request::Files { priority: true, .. }
                | Request::Manifests { priority: true, .. }
                | Request::Diff { priority: true, .. }
                | Request::Packages { priority: true, .. }
                | Request::SaveManifest { priority: true, .. }
                | Request::Workshop { priority: true, .. }
                | Request::LocalInfo { priority: true, .. });
            let kind = job_kind_of(&other);
            let args_summary = summarize(&other);
            let job_id = state.allocate_job_id();
            let cancel = CancellationToken::new();
            let job = QueuedJob {
                job_id,
                kind,
                request: other,
                priority,
                submitted_at: unix_now(),
                cancel,
                args_summary,
            };
            // Subscribe BEFORE enqueue so we don't miss JobStarted.
            let rx = state.events.subscribe();
            let position = state.enqueue(job).await;
            let _ = write_frame(&mut stream, &Frame::Response(Response::JobAccepted { job_id, position })).await;
            stream_events(state.clone(), &mut stream, Some(job_id), rx).await;
        }
    }
}

async fn stream_events<S>(
    state: Arc<DaemonState>,
    stream: &mut S,
    filter: Option<JobId>,
    mut rx: tokio::sync::broadcast::Receiver<Event>,
)
where
    S: AsyncWrite + Unpin,
{
    loop {
        tokio::select! {
            _ = state.shutdown.cancelled() => {
                let _ = write_frame(stream, &Frame::EndOfStream { exit_code: 130 }).await;
                return;
            }
            ev = rx.recv() => match ev {
                Ok(ev) => {
                    if let Some(want) = filter
                        && ev.job_id() != Some(want)
                    {
                        continue;
                    }
                    // A JobFinished terminates a single-job attach (filter
                    // = Some(id)) but is just another broadcast to a global
                    // Subscribe (filter = None) -- the TUI / monitor stays
                    // up for the next job.
                    let terminal_exit = match (&ev, filter) {
                        (Event::JobFinished { exit_code, .. }, Some(_)) => Some(*exit_code),
                        _ => None,
                    };
                    if write_frame(stream, &Frame::Event(ev)).await.is_err() {
                        return; // Client dropped; that is fine.
                    }
                    if let Some(exit_code) = terminal_exit {
                        let _ = write_frame(stream, &Frame::EndOfStream { exit_code }).await;
                        return;
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                    // If the filtered job already finished while we were lagging,
                    // recover its exit code from `recent` and write the terminal
                    // frame. Otherwise resume reading and hope the buffer recovers.
                    if let Some(want) = filter
                        && let Some(exit_code) = state.recent_exit_code(want).await
                    {
                        let _ = write_frame(stream, &Frame::EndOfStream { exit_code }).await;
                        return;
                    }
                    continue;
                }
                Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
            }
        }
    }
}

fn job_kind_of(r: &Request) -> JobKind {
    match r {
        Request::Download { .. } => JobKind::Download,
        Request::Info { .. } => JobKind::Info,
        Request::Files { .. } => JobKind::Files,
        Request::Manifests { .. } => JobKind::Manifests,
        Request::Diff { .. } => JobKind::Diff,
        Request::Packages { .. } => JobKind::Packages,
        Request::SaveManifest { .. } => JobKind::SaveManifest,
        Request::Workshop { .. } => JobKind::Workshop,
        Request::LocalInfo { .. } => JobKind::LocalInfo,
        _ => unreachable!("control variants do not produce jobs"),
    }
}

fn summarize(r: &Request) -> String {
    match r {
        Request::Download { args, .. } => format!("download app={} depot={:?}", args.app, args.depot),
        Request::Info { args, .. } => format!("info app={}", args.app),
        Request::Files { args, .. } => format!("files app={:?}", args.app),
        Request::Manifests { args, .. } => format!("manifests app={}", args.app),
        Request::Diff { args, .. } => format!("diff depot={} from={} to={}", args.depot, args.from, args.to),
        Request::Packages { args, .. } => format!("packages count={}", args.packages.len()),
        Request::SaveManifest { args, .. } => format!("save-manifest app={} depot={}", args.app, args.depot),
        Request::Workshop { args, .. } => format!("workshop item={}", args.item),
        Request::LocalInfo { .. } => "local-info".to_string(),
        _ => "(control)".to_string(),
    }
}
