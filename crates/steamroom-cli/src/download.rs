use std::sync::Arc;
use std::time::Instant;

use indicatif::MultiProgress;
use indicatif::ProgressBar;
use indicatif::ProgressStyle;
use steamroom_client::event::DownloadEvent;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

use crate::daemon::proto::ProgressUpdate;
use crate::sink::JobSink;

pub fn spawn_progress_renderer(
    mut rx: mpsc::UnboundedReceiver<DownloadEvent>,
    show_bars: bool,
    sink: Option<Arc<dyn JobSink>>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        if show_bars {
            run_with_bars(&mut rx, sink).await;
        } else {
            run_quiet(&mut rx, sink).await;
        }
    })
}

async fn run_with_bars(
    rx: &mut mpsc::UnboundedReceiver<DownloadEvent>,
    sink: Option<Arc<dyn JobSink>>,
) {
    let mp = MultiProgress::new();

    let total_bar = mp.add(ProgressBar::hidden());
    let file_bar = mp.add(ProgressBar::new_spinner());
    file_bar.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner:.green} {wide_msg}")
            .unwrap(),
    );

    let start_time = Instant::now();
    let mut bytes_done: u64 = 0;
    let mut bytes_total: u64 = 0;
    let mut files_done: u32 = 0;
    let mut files_total: u32 = 0;
    let mut last_sink_send = Instant::now();

    while let Some(event) = rx.recv().await {
        match event {
            DownloadEvent::DownloadStarted {
                total_bytes,
                total_files,
            } => {
                bytes_total = total_bytes;
                files_total = total_files as u32;
                total_bar.set_length(total_bytes);
                total_bar.set_style(
                    ProgressStyle::default_bar()
                        .template("{spinner:.green} [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({bytes_per_sec}, {eta})")
                        .unwrap()
                        .progress_chars("=> "),
                );
                total_bar.reset();
                mp.println(format!(
                    "downloading {total_files} files ({})",
                    fmt_bytes(total_bytes)
                ))
                .ok();
            }
            DownloadEvent::FileStarted { filename } => {
                file_bar.set_message(filename);
            }
            DownloadEvent::FileCompleted { filename } => {
                files_done += 1;
                file_bar.set_message(filename);
                if let Some(ref s) = sink {
                    let elapsed = start_time.elapsed().as_secs_f64();
                    let rate = if elapsed > 0.0 {
                        bytes_done as f64 / elapsed
                    } else {
                        0.0
                    };
                    let remaining = bytes_total.saturating_sub(bytes_done) as f64;
                    let eta = if rate > 0.0 { remaining / rate } else { 0.0 };
                    s.progress(ProgressUpdate {
                        bytes_done,
                        bytes_total,
                        files_done,
                        files_total,
                        rate_bytes_per_sec: rate as u64,
                        eta_seconds: eta as u32,
                    });
                    last_sink_send = Instant::now();
                }
            }
            DownloadEvent::FileSkipped { .. } => {}
            DownloadEvent::FileRemoved { filename } => {
                mp.println(format!("removed {filename}")).ok();
            }
            DownloadEvent::ChunkCompleted { bytes } => {
                bytes_done += bytes;
                total_bar.inc(bytes);
                if let Some(ref s) = sink {
                    let now = Instant::now();
                    if now.duration_since(last_sink_send) >= std::time::Duration::from_millis(100) {
                        let elapsed = start_time.elapsed().as_secs_f64();
                        let rate = if elapsed > 0.0 {
                            bytes_done as f64 / elapsed
                        } else {
                            0.0
                        };
                        let remaining = bytes_total.saturating_sub(bytes_done) as f64;
                        let eta = if rate > 0.0 { remaining / rate } else { 0.0 };
                        s.progress(ProgressUpdate {
                            bytes_done,
                            bytes_total,
                            files_done,
                            files_total,
                            rate_bytes_per_sec: rate as u64,
                            eta_seconds: eta as u32,
                        });
                        last_sink_send = now;
                    }
                }
            }
            DownloadEvent::ChunkFailed { error } => {
                mp.println(format!("warning: chunk failed (retrying): {error}"))
                    .ok();
            }
            _ => {}
        }
    }

    total_bar.finish_and_clear();
    file_bar.finish_and_clear();
}

async fn run_quiet(
    rx: &mut mpsc::UnboundedReceiver<DownloadEvent>,
    sink: Option<Arc<dyn JobSink>>,
) {
    let start_time = Instant::now();
    let mut bytes_done: u64 = 0;
    let mut bytes_total: u64 = 0;
    let mut files_done: u32 = 0;
    let mut files_total: u32 = 0;
    let mut last_sink_send = Instant::now();

    while let Some(event) = rx.recv().await {
        match event {
            DownloadEvent::DownloadStarted {
                total_bytes,
                total_files,
            } => {
                bytes_total = total_bytes;
                files_total = total_files as u32;
            }
            DownloadEvent::FileCompleted { filename } => {
                files_done += 1;
                let pct = if bytes_total > 0 {
                    bytes_done as f64 / bytes_total as f64 * 100.0
                } else {
                    0.0
                };
                tracing::info!("[{pct:.1}%] {filename}");
                if let Some(ref s) = sink {
                    let elapsed = start_time.elapsed().as_secs_f64();
                    let rate = if elapsed > 0.0 {
                        bytes_done as f64 / elapsed
                    } else {
                        0.0
                    };
                    let remaining = bytes_total.saturating_sub(bytes_done) as f64;
                    let eta = if rate > 0.0 { remaining / rate } else { 0.0 };
                    s.progress(ProgressUpdate {
                        bytes_done,
                        bytes_total,
                        files_done,
                        files_total,
                        rate_bytes_per_sec: rate as u64,
                        eta_seconds: eta as u32,
                    });
                    last_sink_send = Instant::now();
                }
            }
            DownloadEvent::ChunkCompleted { bytes } => {
                bytes_done += bytes;
                if let Some(ref s) = sink {
                    let now = Instant::now();
                    if now.duration_since(last_sink_send) >= std::time::Duration::from_millis(100) {
                        let elapsed = start_time.elapsed().as_secs_f64();
                        let rate = if elapsed > 0.0 {
                            bytes_done as f64 / elapsed
                        } else {
                            0.0
                        };
                        let remaining = bytes_total.saturating_sub(bytes_done) as f64;
                        let eta = if rate > 0.0 { remaining / rate } else { 0.0 };
                        s.progress(ProgressUpdate {
                            bytes_done,
                            bytes_total,
                            files_done,
                            files_total,
                            rate_bytes_per_sec: rate as u64,
                            eta_seconds: eta as u32,
                        });
                        last_sink_send = now;
                    }
                }
            }
            DownloadEvent::FileRemoved { filename } => {
                tracing::info!("removed {filename}");
            }
            DownloadEvent::ChunkFailed { error } => {
                tracing::warn!("chunk failed (retrying): {error}");
            }
            _ => {}
        }
    }
}

fn fmt_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = 1024 * KB;
    const GB: u64 = 1024 * MB;
    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}
