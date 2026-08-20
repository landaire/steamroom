//! `steamroom save-manifest`: download a single depot manifest from the
//! CDN and persist it to disk (raw + decompressed + depot.json).

use crate::cli::SaveManifestArgs;
use crate::commands::shared::decompress_manifest;
use crate::commands::shared::fetch_app_details;
use crate::errors::CliError;
use crate::sink::JobSink;
use std::sync::Arc;
use steamroom::cdn::CdnClient;
use steamroom::client::LoggedIn;
use steamroom::client::SteamClient;
use steamroom::depot::*;
use tokio_util::sync::CancellationToken;
use tracing::debug;
use tracing::info;

pub async fn run_save_manifest(
    args: SaveManifestArgs,
    client: SteamClient<LoggedIn>,
    _sink: Arc<dyn JobSink>,
    _cancel: CancellationToken,
) -> Result<(), CliError> {
    let app_id = AppId(args.app);
    let depot_id = DepotId(args.depot);
    let branch = args.branch.as_deref().unwrap_or("public");

    let manifest_id = if let Some(m) = args.manifest {
        ManifestId(m)
    } else {
        let details = fetch_app_details(&client, app_id).await?;
        details
            .depot(depot_id)
            .and_then(|d| d.manifest(branch))
            .map(|m| m.manifest_id)
            .ok_or(CliError::ManifestNotFound {
                depot: depot_id.0,
                branch: branch.to_string(),
            })?
    };

    info!("depot={depot_id}, manifest={manifest_id}");

    let depot_key = client.get_depot_decryption_key(depot_id, app_id).await?;
    let cdn_servers = client.get_cdn_servers(CellId(0), Some(5)).await?;
    let cdn_server = cdn_servers.first().ok_or(CliError::NoCdnServers)?;

    let request_code = match client
        .get_manifest_request_code(app_id, depot_id, manifest_id, Some(branch), None)
        .await
    {
        Ok(Some(code)) => code,
        Ok(None) => 0,
        Err(e) => {
            debug!("manifest request code failed ({e}), trying without");
            0
        }
    };

    let cdn_auth_token = match client
        .get_cdn_auth_token(app_id, depot_id, &cdn_server.host)
        .await
    {
        Ok(t) => t.token,
        Err(e) => {
            debug!("CDN auth token failed ({e}), continuing without");
            None
        }
    };

    let cdn = CdnClient::new().map_err(CliError::Steam)?;
    let cdn_pool = steamroom::cdn::CdnServerPool::new(cdn_servers.clone());
    info!("downloading manifest...");
    let raw = cdn
        .download_manifest_pooled(
            &cdn_pool,
            depot_id,
            manifest_id,
            request_code,
            cdn_auth_token.as_deref(),
        )
        .await?;
    let decompressed = decompress_manifest(&raw)?;

    std::fs::create_dir_all(&args.output)?;
    steamroom_client::depot_config::DepotConfig::save_manifest_raw(
        &args.output,
        depot_id,
        manifest_id,
        &raw,
    )?;
    steamroom_client::depot_config::DepotConfig::save_manifest_decompressed(
        &args.output,
        depot_id,
        manifest_id,
        &decompressed,
    )?;

    let mut depot_config = steamroom_client::depot_config::DepotConfig::load(&args.output);
    depot_config.set_installed(depot_id, manifest_id, &depot_key);
    depot_config.save(&args.output)?;

    info!("saved manifest {manifest_id} to {}", args.output.display());
    Ok(())
}
