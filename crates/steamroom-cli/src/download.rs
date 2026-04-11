use indicatif::MultiProgress;
use indicatif::ProgressBar;
use indicatif::ProgressStyle;
use steamroom_client::event::DownloadEvent;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

pub fn spawn_progress_renderer(
    mut rx: mpsc::UnboundedReceiver<DownloadEvent>,
    show_bars: bool,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        if show_bars {
            run_with_bars(&mut rx).await;
        } else {
            run_quiet(&mut rx).await;
        }
    })
}

async fn run_with_bars(rx: &mut mpsc::UnboundedReceiver<DownloadEvent>) {
    let mp = MultiProgress::new();

    let total_bar = mp.add(ProgressBar::hidden());
    let file_bar = mp.add(ProgressBar::new_spinner());
    file_bar.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner:.green} {wide_msg}")
            .unwrap(),
    );

    while let Some(event) = rx.recv().await {
        match event {
            DownloadEvent::DownloadStarted {
                total_bytes,
                total_files,
            } => {
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
                file_bar.set_message(filename);
            }
            DownloadEvent::FileSkipped { .. } => {}
            DownloadEvent::FileRemoved { filename } => {
                mp.println(format!("removed {filename}")).ok();
            }
            DownloadEvent::ChunkCompleted { bytes } => {
                total_bar.inc(bytes);
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

async fn run_quiet(rx: &mut mpsc::UnboundedReceiver<DownloadEvent>) {
    let mut completed: u64 = 0;
    let mut total: u64 = 0;
    while let Some(event) = rx.recv().await {
        match event {
            DownloadEvent::DownloadStarted { total_bytes, .. } => {
                total = total_bytes;
            }
            DownloadEvent::FileCompleted { filename } => {
                let pct = if total > 0 {
                    completed as f64 / total as f64 * 100.0
                } else {
                    0.0
                };
                tracing::info!("[{pct:.1}%] {filename}");
            }
            DownloadEvent::ChunkCompleted { bytes } => {
                completed += bytes;
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
