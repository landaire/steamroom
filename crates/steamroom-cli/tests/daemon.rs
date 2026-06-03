//! Integration tests for the daemon's connection-level behavior. These
//! drive `handle_connection` against a `tokio::io::duplex` so the wire
//! protocol, request dispatch, and replay buffer can be exercised
//! without a real socket or a real `SteamClient`.
//!
//! Tests that need a worker actually running (`Download` / `Info` /
//! etc.) are not covered here -- those require a SteamClient. The
//! protocol-level surface (Status, Subscribe, Attach, Cancel,
//! TogglePriority, Stop) is the focus.

use std::time::Duration;
use tokio::io::duplex;

use steamroom_cli::daemon::framing::read_frame;
use steamroom_cli::daemon::framing::write_frame;
use steamroom_cli::daemon::proto::ErrorKind;
use steamroom_cli::daemon::proto::Event;
use steamroom_cli::daemon::proto::Frame;
use steamroom_cli::daemon::proto::InfoParams;
use steamroom_cli::daemon::proto::JobId;
use steamroom_cli::daemon::proto::JobKind;
use steamroom_cli::daemon::proto::JobRecord;
use steamroom_cli::daemon::proto::LogLevel;
use steamroom_cli::daemon::proto::OutputFormat;
use steamroom_cli::daemon::proto::ProgressUpdate;
use steamroom_cli::daemon::proto::Request;
use steamroom_cli::daemon::proto::Response;
use steamroom_cli::daemon::server::DaemonState;
use steamroom_cli::daemon::server::handle_connection;
use steamroom_cli::daemon::server::spawn_replay_collector;

/// `handle_connection` against a duplex: write a Status request, read
/// back a Status response, assert the snapshot looks right.
#[tokio::test]
async fn status_round_trip() {
    let s = DaemonState::new(Some("alice".into()), 42, 1000);
    let (mut client, server) = duplex(64 * 1024);
    let server_state = s.clone();
    let server_task = tokio::spawn(async move { handle_connection(server_state, server).await });

    write_frame(&mut client, &Frame::Request(Request::Status))
        .await
        .unwrap();
    let resp = read_frame(&mut client).await.unwrap();
    match resp {
        Frame::Response(Response::Status(snap)) => {
            assert_eq!(snap.daemon_pid, 42);
            assert_eq!(snap.account.as_deref(), Some("alice"));
            assert!(snap.active.is_none());
            assert!(snap.queue.is_empty());
        }
        other => panic!("expected Status, got {other:?}"),
    }
    server_task.await.unwrap();
}

/// `Attach` for an unknown job should reply Error{JobNotFound}, not
/// hang waiting for events that will never come.
#[tokio::test]
async fn attach_unknown_job_replies_job_not_found() {
    let s = DaemonState::new(None, 1, 0);
    let (mut client, server) = duplex(64 * 1024);
    let server_state = s.clone();
    let server_task = tokio::spawn(async move { handle_connection(server_state, server).await });

    write_frame(
        &mut client,
        &Frame::Request(Request::Attach { job_id: JobId(999) }),
    )
    .await
    .unwrap();
    let resp = read_frame(&mut client).await.unwrap();
    match resp {
        Frame::Response(Response::Error { kind, .. }) => {
            assert_eq!(kind, ErrorKind::JobNotFound);
        }
        other => panic!("expected Error{{JobNotFound}}, got {other:?}"),
    }
    server_task.await.unwrap();
}

/// `Attach` to a finished job (present in `recent`) replays its events
/// from the replay buffer and then writes EndOfStream.
#[tokio::test]
async fn attach_finished_job_replays_buffered_events() {
    let s = DaemonState::new(None, 1, 0);

    // Spawn the replay collector so events flowing through the broadcast
    // channel land in the per-job ring. Subscription is synchronous so
    // no events emitted after this line are lost.
    spawn_replay_collector(s.clone());

    let job_id = JobId(7);
    let kind = JobKind::Info;

    // Drive a fake job lifecycle through the broadcast channel. The
    // collector reads JobStarted to allocate the ring, then appends
    // Stdout, Log, Progress, JobFinished in order.
    let _ = s.events.send(Event::JobStarted {
        job_id,
        kind,
        args_summary: "fake".into(),
    });
    let _ = s.events.send(Event::Stdout {
        job_id,
        line: "first line".into(),
    });
    let _ = s.events.send(Event::Log {
        job_id: Some(job_id),
        level: LogLevel::Info,
        target: "test".into(),
        message: "hello".into(),
    });
    let _ = s.events.send(Event::Progress {
        job_id,
        update: ProgressUpdate {
            bytes_done: 50,
            bytes_total: 100,
            files_done: 1,
            files_total: 2,
            rate_bytes_per_sec: 1024,
            eta_seconds: 1,
        },
    });
    let _ = s.events.send(Event::JobFinished {
        job_id,
        exit_code: 0,
    });

    // Mark the job as finished in `recent` so the Attach handler routes
    // through the replay path instead of subscribing live.
    s.recent.lock().await.push(JobRecord {
        job_id,
        kind,
        args_summary: "fake".into(),
        priority: false,
        submitted_at: 0,
        started_at: Some(0),
        finished_at: Some(1),
        exit_code: Some(0),
        progress: None,
    });

    // Give the collector a moment to drain the broadcast queue.
    tokio::time::sleep(Duration::from_millis(50)).await;

    let (mut client, server) = duplex(64 * 1024);
    let server_state = s.clone();
    let server_task = tokio::spawn(async move { handle_connection(server_state, server).await });

    write_frame(&mut client, &Frame::Request(Request::Attach { job_id }))
        .await
        .unwrap();

    // Read frames until EndOfStream. Expect at least the Stdout line and
    // the JobFinished event in order.
    let mut saw_stdout = false;
    let mut saw_finished = false;
    let exit = loop {
        let frame = read_frame(&mut client).await.unwrap();
        match frame {
            Frame::Event(Event::Stdout { line, .. }) => {
                assert_eq!(line, "first line");
                saw_stdout = true;
            }
            Frame::Event(Event::JobFinished { .. }) => {
                saw_finished = true;
            }
            Frame::Event(_) => {}
            Frame::EndOfStream { exit_code } => break exit_code,
            other => panic!("unexpected: {other:?}"),
        }
    };
    assert!(saw_stdout, "Stdout event should be replayed");
    assert!(saw_finished, "JobFinished should be replayed");
    assert_eq!(exit, 0);
    server_task.await.unwrap();
}

/// `Cancel` for a queued job removes it and writes Ack.
#[tokio::test]
async fn cancel_queued_job_acks() {
    use steamroom_cli::daemon::server::QueuedJob;
    use tokio_util::sync::CancellationToken;

    let s = DaemonState::new(None, 1, 0);
    let job_id = s.allocate_job_id();
    s.enqueue(QueuedJob {
        job_id,
        kind: JobKind::Info,
        request: Request::Info {
            args: InfoParams {
                app: 480,
                format: Some(OutputFormat::Plain),
                os: None,
                show_all: false,
            },
            priority: false,
        },
        priority: false,
        submitted_at: 0,
        cancel: CancellationToken::new(),
        args_summary: "info app=480".into(),
    })
    .await;

    let (mut client, server) = duplex(64 * 1024);
    let server_state = s.clone();
    let server_task = tokio::spawn(async move { handle_connection(server_state, server).await });

    write_frame(&mut client, &Frame::Request(Request::Cancel { job_id }))
        .await
        .unwrap();
    let resp = read_frame(&mut client).await.unwrap();
    assert!(matches!(resp, Frame::Response(Response::Ack)));
    server_task.await.unwrap();

    // Queue should be empty after the cancel.
    let snap = s.snapshot().await;
    assert!(snap.queue.is_empty());
}

/// `TogglePriority` for an unknown id replies Error{JobNotFound}.
#[tokio::test]
async fn toggle_priority_unknown_job() {
    let s = DaemonState::new(None, 1, 0);
    let (mut client, server) = duplex(64 * 1024);
    let server_state = s.clone();
    let server_task = tokio::spawn(async move { handle_connection(server_state, server).await });

    write_frame(
        &mut client,
        &Frame::Request(Request::TogglePriority { job_id: JobId(123) }),
    )
    .await
    .unwrap();
    let resp = read_frame(&mut client).await.unwrap();
    match resp {
        Frame::Response(Response::Error { kind, .. }) => {
            assert_eq!(kind, ErrorKind::JobNotFound);
        }
        other => panic!("expected JobNotFound, got {other:?}"),
    }
    server_task.await.unwrap();
}

/// `Stop { force: false }` acks with Stopping and sets `accepting`.
#[tokio::test]
async fn stop_force_false_signals_accepting() {
    let s = DaemonState::new(None, 1, 0);
    let (mut client, server) = duplex(64 * 1024);
    let server_state = s.clone();
    let server_task = tokio::spawn(async move { handle_connection(server_state, server).await });

    write_frame(&mut client, &Frame::Request(Request::Stop { force: false }))
        .await
        .unwrap();
    let resp = read_frame(&mut client).await.unwrap();
    assert!(matches!(resp, Frame::Response(Response::Stopping)));
    server_task.await.unwrap();

    assert!(
        s.accepting.is_cancelled(),
        "graceful stop should cancel `accepting`"
    );
    assert!(
        !s.shutdown.is_cancelled(),
        "graceful stop should NOT cancel `shutdown` immediately"
    );
}
