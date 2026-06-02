//! `steamroom save-manifest`: download a single depot manifest from the
//! CDN and persist it to disk (raw + decompressed + depot.json).

use crate::cli::SaveManifestArgs;
use crate::commands::shared::decompress_manifest;
use crate::commands::shared::find_manifest_for_depot;
use crate::commands::shared::parse_app_kv;
use crate::errors::CliError;
use crate::sink::JobSink;
use steamroom::apps::AccessToken;
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
    _sink: &dyn JobSink,
    _cancel: CancellationToken,
) -> Result<(), CliError> {
    let app_id = AppId(args.app);
    let depot_id = DepotId(args.depot);
    let branch = args.branch.as_deref().unwrap_or("public");

    let manifest_id = if let Some(m) = args.manifest {
        ManifestId(m)
    } else {
        let tokens = client.pics_get_access_tokens(&[app_id]).await?;
        let token = tokens
            .into_iter()
            .next()
            .unwrap_or(AccessToken { app_id, token: 0 });
        let infos = client.pics_get_product_info(&[token]).await?;
        let app_info = infos
            .into_iter()
            .next()
            .ok_or(CliError::NoProductInfo(app_id.0))?;
        let kv_data = app_info.kv_data.ok_or(CliError::NoKvData(app_id.0))?;
        let kv = parse_app_kv(&kv_data)?;
        let depots_kv = kv.get("depots").ok_or(CliError::NoDepots)?;
        find_manifest_for_depot(depots_kv, depot_id, branch)?
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
    info!("downloading manifest...");
    let raw = cdn
        .download_manifest(
            cdn_server,
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
