//! `steamroom workshop`: download a single Workshop item by its
//! `publishedfileid`.
//!
//! `show_progress` mirrors `commands::download::run_download`: T10 will
//! wire the sink-side progress events and the cancellation `select!`.

use crate::cli::WorkshopArgs;
use crate::commands::shared::decompress_manifest;
use crate::commands::shared::fmt_size;
use crate::download as direct_progress;
use crate::errors::CliError;
use crate::sink::JobSink;
use std::path::PathBuf;
use std::sync::Arc;
use steamroom::cdn::CdnClient;
use steamroom::client::LoggedIn;
use steamroom::client::SteamClient;
use steamroom::depot::manifest::DepotManifest;
use steamroom::depot::*;
use tokio_util::sync::CancellationToken;
use tracing::info;

pub async fn run_workshop(
    args: WorkshopArgs,
    client: SteamClient<LoggedIn>,
    sink: Arc<dyn JobSink>,
    cancel: CancellationToken,
    show_progress: bool,
) -> Result<(), CliError> {
    info!("fetching workshop item {} details...", args.item);
    let req = steamroom::generated::CPublishedFileGetDetailsRequest {
        publishedfileids: vec![args.item],
        includechildren: Some(true),
        ..Default::default()
    };
    let resp = client
        .call_service_method(
            "PublishedFile.GetDetails#1",
            &prost::Message::encode_to_vec(&req),
        )
        .await?;
    let details: steamroom::generated::CPublishedFileGetDetailsResponse = resp.decode()?;

    let item = details
        .publishedfiledetails
        .first()
        .ok_or(CliError::NoProductInfo(args.app))?;

    let title = item.title.as_deref().unwrap_or("(untitled)");
    let hcontent = item.hcontent_file.unwrap_or(0);
    let file_size = item.file_size.unwrap_or(0);
    let consumer_app = item.consumer_appid.unwrap_or(args.app);
    let filename = item.filename.as_deref().unwrap_or("workshop_content");

    info!("workshop item: {title}");
    info!("  content manifest: {hcontent}");
    info!("  file: {filename} ({} bytes)", file_size);

    if hcontent == 0 {
        info!("no downloadable content for this workshop item");
        return Ok(());
    }

    // Workshop items use the app's depot
    let app_id = AppId(consumer_app);
    let depot_id = DepotId(consumer_app);
    let manifest_id = ManifestId(hcontent);

    let depot_key = client.get_depot_decryption_key(depot_id, app_id).await?;
    let cdn_servers = client.get_cdn_servers(CellId(0), Some(5)).await?;
    if cdn_servers.is_empty() {
        return Err(CliError::NoCdnServers);
    }
    let cdn_pool = steamroom::cdn::CdnServerPool::new(cdn_servers.clone());
    let cdn = CdnClient::new().map_err(CliError::Steam)?;

    let request_code = client
        .get_manifest_request_code(app_id, depot_id, manifest_id, None, None)
        .await?
        .unwrap_or(0);

    let manifest_data = cdn
        .download_manifest_pooled(&cdn_pool, depot_id, manifest_id, request_code, None)
        .await?;
    let manifest_bytes = decompress_manifest(&manifest_data)?;
    let mut manifest = DepotManifest::parse(&manifest_bytes)?;
    if manifest.filenames_encrypted {
        manifest.decrypt_filenames(&depot_key)?;
    }

    let output_dir = args
        .output
        .unwrap_or_else(|| PathBuf::from("workshop").join(args.item.to_string()));
    std::fs::create_dir_all(&output_dir)?;

    let (event_tx, event_rx) = tokio::sync::mpsc::unbounded_channel();
    let fetcher = steamroom_client::download::CdnChunkFetcher::new(cdn, cdn_pool, None);
    let job = steamroom_client::download::DepotJob::builder()
        .depot_id(depot_id)
        .depot_key(depot_key)
        .install_dir(output_dir.clone())
        .event_sender(event_tx)
        .build()?;

    info!("downloading to {}", output_dir.display());

    let progress_handle =
        direct_progress::spawn_progress_renderer(event_rx, show_progress, Some(sink.clone()));

    // Run the download inside a block so the future (and its inner event_tx)
    // drops before we await the progress renderer. This ensures the renderer
    // sees the channel close and runs finish_and_clear on both the success
    // and cancellation paths.
    let stats_result = {
        let download_fut = job.download(&manifest, std::sync::Arc::new(fetcher));
        tokio::pin!(download_fut);
        tokio::select! {
            res = &mut download_fut => Some(res.map_err(CliError::Download)),
            // Dropping the future aborts the orchestration. Spawned chunk-fetch
            // tasks held inside DepotJob may continue until their next yield
            // point.
            _ = cancel.cancelled() => None,
        }
    };
    // download_fut's inner future has now dropped (block scope ended); event_tx is closed.
    let _ = progress_handle.await;
    let stats = match stats_result {
        Some(res) => res?,
        None => return Err(CliError::Cancelled),
    };

    info!(
        "workshop download complete: {} files, {}",
        stats.files_completed,
        fmt_size(stats.bytes_downloaded)
    );
    Ok(())
}
