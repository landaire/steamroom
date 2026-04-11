use steamroom_client::event::DownloadEvent;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

pub fn spawn_progress_renderer(
    _rx: mpsc::UnboundedReceiver<DownloadEvent>,
) -> JoinHandle<()> {
    todo!()
}
