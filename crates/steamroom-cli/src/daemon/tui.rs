//! Ratatui status dashboard. Routes input from crossterm and events from
//! a Subscribe RPC connection through a single state machine.

use crossterm::event::{Event as CtEvent, KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Gauge, List, ListItem, Paragraph, Wrap};
use std::io::Stdout;
use std::time::Duration;
use tokio::sync::mpsc;

use crate::daemon::client::connect;
use crate::daemon::framing::{read_frame, write_frame};
use crate::daemon::proto::{
    Event as ProtoEvent, Frame, JobRecord, Request, Response,
    StatusSnapshot,
};
use crate::errors::CliError;

pub async fn run_tui() -> Result<(), CliError> {
    let mut terminal = init_terminal()?;
    let result = main_loop(&mut terminal).await;
    restore_terminal(&mut terminal).ok();
    result
}

fn init_terminal() -> Result<Terminal<CrosstermBackend<Stdout>>, CliError> {
    crossterm::terminal::enable_raw_mode().map_err(|e| CliError::Io(std::io::Error::other(e)))?;
    let mut stdout = std::io::stdout();
    crossterm::execute!(
        stdout,
        crossterm::terminal::EnterAlternateScreen,
        crossterm::event::EnableMouseCapture
    )
    .map_err(|e| CliError::Io(std::io::Error::other(e)))?;
    let backend = CrosstermBackend::new(stdout);
    Terminal::new(backend).map_err(|e| CliError::Io(std::io::Error::other(e)))
}

fn restore_terminal(t: &mut Terminal<CrosstermBackend<Stdout>>) -> Result<(), CliError> {
    crossterm::terminal::disable_raw_mode().map_err(|e| CliError::Io(std::io::Error::other(e)))?;
    crossterm::execute!(
        t.backend_mut(),
        crossterm::terminal::LeaveAlternateScreen,
        crossterm::event::DisableMouseCapture
    )
    .map_err(|e| CliError::Io(std::io::Error::other(e)))?;
    t.show_cursor().ok();
    Ok(())
}

struct TuiState {
    snapshot: StatusSnapshot,
    selected_queue_idx: usize,
    log: std::collections::VecDeque<String>,
    log_cap: usize,
}

impl TuiState {
    fn new(snapshot: StatusSnapshot) -> Self {
        Self {
            snapshot,
            selected_queue_idx: 0,
            log: Default::default(),
            log_cap: 1000,
        }
    }

    fn selected_job(&self) -> Option<&JobRecord> {
        self.snapshot.queue.get(self.selected_queue_idx)
    }

    fn push_log(&mut self, line: String) {
        if self.log.len() == self.log_cap {
            self.log.pop_front();
        }
        self.log.push_back(line);
    }
}

async fn main_loop(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
) -> Result<(), CliError> {
    // Seed state via Status, then open a Subscribe stream for live updates.
    let mut status_stream = connect().await?;
    write_frame(&mut status_stream, &Frame::Request(Request::Status)).await?;
    let snap = match read_frame(&mut status_stream).await? {
        Frame::Response(Response::Status(s)) => s,
        other => return Err(CliError::MalformedFrame(format!("status: {other:?}"))),
    };
    drop(status_stream);

    let mut state = TuiState::new(snap);

    // Subscribe events into a channel.
    let (ev_tx, mut ev_rx) = mpsc::channel::<ProtoEvent>(256);
    let subscribe_task = tokio::spawn(async move {
        let Ok(mut sub) = connect().await else { return; };
        if write_frame(&mut sub, &Frame::Request(Request::Subscribe))
            .await
            .is_err()
        {
            return;
        }
        loop {
            match read_frame(&mut sub).await {
                Ok(Frame::Event(ev)) => {
                    if ev_tx.send(ev).await.is_err() {
                        return;
                    }
                }
                Ok(Frame::EndOfStream { .. }) => return,
                Ok(_) => continue,
                Err(_) => return,
            }
        }
    });

    // crossterm key events into a channel.
    let (key_tx, mut key_rx) = mpsc::channel::<CtEvent>(64);
    let keys_task = tokio::task::spawn_blocking(move || loop {
        if crossterm::event::poll(Duration::from_millis(200)).unwrap_or(false)
            && let Ok(ev) = crossterm::event::read()
            && key_tx.blocking_send(ev).is_err()
        {
            return;
        }
    });

    loop {
        draw(terminal, &state)?;
        tokio::select! {
            ev = ev_rx.recv() => match ev {
                Some(e) => apply_event(&mut state, e),
                None => break,
            },
            key = key_rx.recv() => match key {
                Some(CtEvent::Key(k)) => {
                    if handle_key(&mut state, k).await? { break; }
                }
                Some(_) => continue,
                None => break,
            },
        }
    }

    subscribe_task.abort();
    keys_task.abort();
    Ok(())
}

fn apply_event(state: &mut TuiState, ev: ProtoEvent) {
    match ev {
        ProtoEvent::QueueChanged { snapshot } => {
            state.snapshot = snapshot;
            if state.selected_queue_idx >= state.snapshot.queue.len() {
                state.selected_queue_idx =
                    state.snapshot.queue.len().saturating_sub(1);
            }
        }
        ProtoEvent::Progress { update, .. } => {
            if let Some(active) = state.snapshot.active.as_mut() {
                active.progress = Some(update);
            }
        }
        ProtoEvent::Log {
            level,
            target,
            message,
            ..
        } => {
            state.push_log(format!("[{level:?}] {target}: {message}"));
        }
        ProtoEvent::Stdout { line, .. } => state.push_log(line),
        ProtoEvent::JobStarted { .. } | ProtoEvent::JobFinished { .. } => {}
    }
}

async fn handle_key(state: &mut TuiState, k: KeyEvent) -> Result<bool, CliError> {
    match (k.code, k.modifiers) {
        (KeyCode::Char('q'), _) => return Ok(true),
        (KeyCode::Char('c'), KeyModifiers::CONTROL) => return Ok(true),
        (KeyCode::Up, _) => {
            state.selected_queue_idx = state.selected_queue_idx.saturating_sub(1);
        }
        (KeyCode::Down, _) => {
            if state.selected_queue_idx + 1 < state.snapshot.queue.len() {
                state.selected_queue_idx += 1;
            }
        }
        (KeyCode::Char('p'), _) => {
            if let Some(j) = state.selected_job() {
                let id = j.job_id;
                send_one(Request::TogglePriority { job_id: id }).await?;
            }
        }
        (KeyCode::Char('x'), _) => {
            if let Some(j) = state.selected_job() {
                let id = j.job_id;
                send_one(Request::Cancel { job_id: id }).await?;
            }
        }
        _ => {}
    }
    Ok(false)
}

async fn send_one(req: Request) -> Result<(), CliError> {
    let mut s = connect().await?;
    write_frame(&mut s, &Frame::Request(req)).await?;
    // Ignore the response payload; state changes arrive via Subscribe's
    // QueueChanged events.
    let _ = read_frame(&mut s).await;
    Ok(())
}

fn draw(
    terminal: &mut Terminal<CrosstermBackend<Stdout>>,
    state: &TuiState,
) -> Result<(), CliError> {
    terminal
        .draw(|f| {
            let outer = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Min(8),
                    Constraint::Length(8),
                    Constraint::Length(1),
                ])
                .split(f.area());
            let top = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
                .split(outer[0]);

            // Queue pane.
            let items: Vec<ListItem> = state
                .snapshot
                .queue
                .iter()
                .enumerate()
                .map(|(i, j)| {
                    let star = if j.priority { "* " } else { "  " };
                    let prefix = if i == state.selected_queue_idx { "> " } else { "  " };
                    ListItem::new(format!(
                        "{prefix}{star}{} {:?} {}",
                        j.job_id, j.kind, j.args_summary
                    ))
                })
                .collect();
            let queue = List::new(items).block(
                Block::default()
                    .borders(Borders::ALL)
                    .title(format!("queue ({})", state.snapshot.queue.len())),
            );
            f.render_widget(queue, top[0]);

            // Active pane.
            let active_block = Block::default().borders(Borders::ALL).title("active job");
            match &state.snapshot.active {
                Some(j) => {
                    let inner = active_block.inner(top[1]);
                    f.render_widget(active_block.clone(), top[1]);
                    let chunks = Layout::default()
                        .direction(Direction::Vertical)
                        .constraints([
                            Constraint::Length(2),
                            Constraint::Length(3),
                            Constraint::Min(0),
                        ])
                        .split(inner);
                    let summary = Paragraph::new(format!(
                        "{} {:?}\n{}",
                        j.job_id, j.kind, j.args_summary
                    ))
                    .wrap(Wrap { trim: true });
                    f.render_widget(summary, chunks[0]);

                    if let Some(p) = &j.progress {
                        let pct = if p.bytes_total > 0 {
                            ((p.bytes_done as f64 / p.bytes_total as f64) * 100.0)
                                .clamp(0.0, 100.0) as u16
                        } else {
                            0
                        };
                        let gauge = Gauge::default()
                            .gauge_style(Style::default().fg(Color::Cyan))
                            .percent(pct)
                            .label(format!(
                                "{}/{} {}/s ETA {}s",
                                human_bytes(p.bytes_done),
                                human_bytes(p.bytes_total),
                                human_bytes(p.rate_bytes_per_sec),
                                p.eta_seconds
                            ));
                        f.render_widget(gauge, chunks[1]);
                    }
                }
                None => {
                    let p = Paragraph::new("idle").block(active_block);
                    f.render_widget(p, top[1]);
                }
            }

            // Log pane: most-recent lines, clamped to pane height.
            let log_height = outer[1].height.saturating_sub(2) as usize;
            let log_lines: Vec<Line> = state
                .log
                .iter()
                .rev()
                .take(log_height)
                .rev()
                .map(|s| Line::from(Span::raw(s.clone())))
                .collect();
            let log = Paragraph::new(log_lines)
                .block(Block::default().borders(Borders::ALL).title("log"))
                .wrap(Wrap { trim: false });
            f.render_widget(log, outer[1]);

            // Footer.
            let footer =
                Paragraph::new("q quit   up/down select   p toggle priority   x cancel")
                    .style(Style::default().add_modifier(Modifier::DIM));
            f.render_widget(footer, outer[2]);
        })
        .map_err(|e| CliError::Io(std::io::Error::other(e)))?;
    Ok(())
}

fn human_bytes(b: u64) -> String {
    if b >= 1 << 30 {
        format!("{:.2} GiB", b as f64 / (1u64 << 30) as f64)
    } else if b >= 1 << 20 {
        format!("{:.2} MiB", b as f64 / (1u64 << 20) as f64)
    } else if b >= 1 << 10 {
        format!("{:.2} KiB", b as f64 / (1u64 << 10) as f64)
    } else {
        format!("{b} B")
    }
}
