use steam_client::event::DownloadEvent;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

pub fn spawn_progress_renderer(
    rx: mpsc::UnboundedReceiver<DownloadEvent>,
) -> JoinHandle<()> {
    todo!()
}
