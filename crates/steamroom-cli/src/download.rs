use indicatif::MultiProgress;
use indicatif::ProgressBar;
use indicatif::ProgressStyle;
use steamroom_client::event::DownloadEvent;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

pub fn spawn_progress_renderer(
    mut rx: mpsc::UnboundedReceiver<DownloadEvent>,
    total_bytes: u64,
    show_bars: bool,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        if show_bars {
            run_with_bars(&mut rx, total_bytes).await;
        } else {
            run_quiet(&mut rx, total_bytes).await;
        }
    })
}

async fn run_with_bars(rx: &mut mpsc::UnboundedReceiver<DownloadEvent>, total_bytes: u64) {
    let mp = MultiProgress::new();

    let total_bar = mp.add(ProgressBar::new(total_bytes));
    total_bar.set_style(
        ProgressStyle::default_bar()
            .template("{spinner:.green} [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({bytes_per_sec}, {eta})")
            .unwrap()
            .progress_chars("=> "),
    );

    let file_bar = mp.add(ProgressBar::new_spinner());
    file_bar.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner:.green} {wide_msg}")
            .unwrap(),
    );

    while let Some(event) = rx.recv().await {
        match event {
            DownloadEvent::FileStarted { filename } => {
                file_bar.set_message(filename);
            }
            DownloadEvent::FileCompleted { filename } => {
                file_bar.set_message(filename);
            }
            DownloadEvent::FileSkipped { .. } => {}
            DownloadEvent::FileRemoved { filename } => {
                file_bar.set_message(format!("removed {filename}"));
            }
            DownloadEvent::ChunkCompleted { bytes } => {
                total_bar.inc(bytes);
            }
            DownloadEvent::ChunkFailed { error } => {
                total_bar.suspend(|| {
                    tracing::warn!("chunk failed (retrying): {error}");
                });
            }
            DownloadEvent::DepotProgress { .. } | _ => {}
        }
    }

    total_bar.finish_and_clear();
    file_bar.finish_and_clear();
}

async fn run_quiet(rx: &mut mpsc::UnboundedReceiver<DownloadEvent>, total_bytes: u64) {
    let mut completed: u64 = 0;
    while let Some(event) = rx.recv().await {
        match event {
            DownloadEvent::FileCompleted { filename } => {
                let pct = if total_bytes > 0 {
                    completed as f64 / total_bytes as f64 * 100.0
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
