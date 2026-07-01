//! `steamroom download`: orchestrate the full depot-content download
//! pipeline (PICS lookup, key fetch, CDN selection, manifest download,
//! `DepotJob`).
//!
//! `show_progress` is currently a per-call parameter so the existing
//! direct-mode progress renderer (`crate::download::spawn_progress_renderer`)
//! can be toggled by `--no-progress`. T9 wires the sink-side progress
//! events and the cancellation `select!`; daemon callers will pass
//! `show_progress=false` and drive progress through the sink instead.

use crate::cli::DownloadArgs;
use crate::commands::shared::decompress_manifest;
use crate::commands::shared::fetch_app_kv;
use crate::commands::shared::find_first_depot;
use crate::commands::shared::find_manifest_for_depot;
use crate::commands::shared::fmt_size;
use crate::download as direct_progress;
use crate::errors::CliError;
use crate::sink::JobSink;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use steamroom::cdn::CdnClient;
use steamroom::client::LoggedIn;
use steamroom::client::SteamClient;
use steamroom::depot::manifest::DepotManifest;
use steamroom::depot::*;
use steamroom_client::download::OldChunkLoc;
use tokio_util::sync::CancellationToken;
use tracing::debug;
use tracing::info;
use tracing::warn;

pub async fn run_download(
    args: DownloadArgs,
    client: SteamClient<LoggedIn>,
    sink: Arc<dyn JobSink>,
    cancel: CancellationToken,
    show_progress: bool,
) -> Result<(), CliError> {
    let app_id = AppId(args.app);

    // Get access tokens
    info!("getting PICS access tokens for app {}", app_id);
    let kv = fetch_app_kv(&client, app_id).await?;

    // Find depots
    let depot_id = if let Some(d) = args.depot {
        DepotId(d)
    } else {
        // Find first depot from the KV data
        let depots_kv = kv.get("depots").ok_or(CliError::NoDepots)?;
        find_first_depot(depots_kv)?
    };

    let branch = args.branch.as_deref().unwrap_or("public");

    // For non-public branches with --local-keys, try using cached beta hash
    if args.local_keys && branch != "public" {
        let steam_dir =
            steamroom_client::steam_creds::steam_dir().ok_or(CliError::SteamNotFound)?;
        let config = steamroom_client::steam_creds::read_config(&steam_dir)
            .ok_or(CliError::SteamNotFound)?;
        if let Some(hash) = steamroom_client::steam_creds::find_beta_hash(&config, app_id.0, branch)
        {
            info!("using cached beta hash for branch \"{branch}\"");
            let token = kv
                .get("depots")
                .and_then(|d| d.get(&depot_id.0.to_string()))
                .and_then(|d| d.get("manifests"))
                .and_then(|d| d.get(branch))
                .and_then(|d| d.get("gid"))
                .and_then(|v| v.as_str());
            // If we can't find the manifest in PICS, try requesting private beta access
            if token.is_none() {
                let access_tokens = client.pics_get_access_tokens(&[app_id]).await?;
                let access_token = access_tokens.first().map(|t| t.token).unwrap_or(0);
                match client
                    .request_private_beta(app_id, access_token, branch, &hash)
                    .await
                {
                    Ok(Some(depot_section)) => {
                        info!("private beta access granted for branch \"{branch}\"");
                        debug!("depot_section: {} bytes", depot_section.len());
                    }
                    Ok(None) => {
                        warn!("private beta access denied for branch \"{branch}\"");
                    }
                    Err(e) => {
                        warn!("private beta request failed: {e}");
                    }
                }
            }
        }
    }

    // Find manifest ID
    let manifest_id = if let Some(m) = args.manifest {
        ManifestId(m)
    } else {
        let depots_kv = kv.get("depots").ok_or(CliError::NoDepots)?;
        find_manifest_for_depot(depots_kv, depot_id, branch)?
    };

    info!(
        "depot={}, manifest={}, branch={}",
        depot_id, manifest_id, branch
    );

    // Get depot decryption key
    let depot_key = if args.local_keys {
        let steam_dir =
            steamroom_client::steam_creds::steam_dir().ok_or(CliError::SteamNotFound)?;
        let config = steamroom_client::steam_creds::read_config(&steam_dir)
            .ok_or(CliError::SteamNotFound)?;
        steamroom_client::steam_creds::find_depot_key(&config, depot_id)
            .ok_or(CliError::NoLocalKey(depot_id.0))?
    } else {
        info!("getting depot key for depot {depot_id} app {app_id}...");
        client.get_depot_decryption_key(depot_id, app_id).await?
    };
    debug!("depot key: {:02x?}", &depot_key.0);

    // Get CDN servers
    info!("getting CDN servers...");
    let cdn_servers = client.get_cdn_servers(CellId(0), Some(20)).await?;
    if cdn_servers.is_empty() {
        return Err(CliError::NoCdnServers);
    }
    info!("got {} CDN servers", cdn_servers.len());
    let cdn_server = &cdn_servers[0];
    let cdn_pool = steamroom::cdn::CdnServerPool::new(cdn_servers.clone());

    // Get manifest request code (optional, old manifests may not have one)
    info!("getting manifest request code...");
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

    // Get CDN auth token (needed for authenticated depots / old manifests)
    let cdn_auth_token = match client
        .get_cdn_auth_token(app_id, depot_id, &cdn_server.host)
        .await
    {
        Ok(t) => t.token,
        Err(e) => {
            debug!("CDN auth token request failed ({e}), continuing without");
            None
        }
    };

    // Download manifest (with cache)
    let cdn = CdnClient::new().map_err(CliError::Steam)?;
    let manifest_cache = steamroom_client::manifest::ManifestCache::new(
        steamroom_client::manifest::ManifestCache::default_path(),
    );

    let (manifest_bytes, cdn_raw) = if let Some(cached) = manifest_cache.load(depot_id, manifest_id)
    {
        debug!("using cached manifest for {depot_id}_{manifest_id}");
        let raw = manifest_cache.load_raw(depot_id, manifest_id);
        (cached, raw)
    } else {
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
        let _ = manifest_cache.save(depot_id, manifest_id, &decompressed, &raw);
        (decompressed, Some(raw.to_vec()))
    };

    // Debug: dump section magics
    {
        let mut off = 0;
        while off + 8 <= manifest_bytes.len() {
            let magic = u32::from_le_bytes(manifest_bytes[off..off + 4].try_into().unwrap());
            let size = u32::from_le_bytes(manifest_bytes[off + 4..off + 8].try_into().unwrap());
            debug!("  section at {off}: magic=0x{magic:08x} size={size}");
            if magic == 0xD64BF064 {
                break;
            }
            off += 8 + size as usize;
        }
    }

    // Parse manifest
    let mut manifest = DepotManifest::parse(&manifest_bytes)?;
    info!(
        "manifest parsed: {} files, encrypted={}",
        manifest.files.len(),
        manifest.filenames_encrypted
    );
    if manifest.filenames_encrypted {
        match manifest.decrypt_filenames(&depot_key) {
            Ok(()) => info!("decrypted filenames"),
            Err(e) => warn!("filename decryption failed ({e}), using raw names"),
        }
    }

    let output_dir = args
        .output
        .unwrap_or_else(|| PathBuf::from("depots").join(depot_id.0.to_string()));
    std::fs::create_dir_all(&output_dir)?;

    // Check for interrupted install
    let mut depot_config = steamroom_client::depot_config::DepotConfig::load(&output_dir);
    if let Some(interrupted_id) = depot_config.is_installing(depot_id) {
        warn!(
            "previous install of manifest {interrupted_id} was interrupted, resuming with {manifest_id}"
        );
    }

    // Load the previously-installed manifest, used both to remove files absent
    // from the new manifest and to reuse unchanged chunks by content.
    let old_manifest = match depot_config.get_installed(depot_id) {
        Some((old_id, old_key)) if old_id != manifest_id => {
            debug!("previous manifest: {old_id}, loading for delta");
            steamroom_client::depot_config::DepotConfig::load_manifest_decompressed(
                &output_dir,
                depot_id,
                old_id,
            )
            .and_then(|bytes| {
                let mut old = DepotManifest::parse(&bytes).ok()?;
                if old.filenames_encrypted {
                    let _ = old.decrypt_filenames(&old_key);
                }
                Some(old)
            })
        }
        _ => None,
    };

    // Set up download orchestration
    let (event_tx, event_rx) = tokio::sync::mpsc::unbounded_channel();

    let fetcher = steamroom_client::download::CdnChunkFetcher::new(cdn, cdn_pool, cdn_auth_token);

    let mut builder = steamroom_client::download::DepotJob::builder()
        .depot_id(depot_id)
        .depot_key(depot_key.clone())
        .install_dir(output_dir.clone())
        .verify(args.verify)
        .non_atomic(args.non_atomic)
        .event_sender(event_tx);

    if let Some(old) = old_manifest {
        info!("delta update: will remove files not in new manifest and reuse unchanged chunks");
        let old_files = old.files.iter().map(|f| f.normalized_path()).collect();
        builder = builder
            .old_manifest_files(old_files)
            .old_file_layouts(old_file_layouts(&old));
    }

    if let Some(max) = args.max_downloads {
        builder = builder.max_downloads(max);
    }

    if let Some(ref filelist_path) = args.filelist {
        let content = std::fs::read_to_string(filelist_path)?;
        let lines: Vec<String> = content
            .lines()
            .map(|l| l.trim().to_string())
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .collect();
        builder = builder.file_filter(steamroom_client::download::FileFilter::from_filelist(
            &lines,
        )?);
    } else if let Some(ref pattern) = args.file_regex {
        builder = builder.file_filter(steamroom_client::download::FileFilter::Regex(
            regex::Regex::new(pattern)?,
        ));
    }

    let job = builder.build()?;

    info!("downloading to {}", output_dir.display());

    // Mark as installing before starting
    depot_config.set_installing(depot_id, manifest_id, &depot_key);
    let _ = depot_config.save(&output_dir);

    let progress_handle =
        direct_progress::spawn_progress_renderer(event_rx, show_progress, Some(sink.clone()));

    // Run the download inside a block so the download future drops (releasing
    // its borrow of `job`) when it finishes or is cancelled.
    let stats_result = {
        let download_fut = job.download(&manifest, std::sync::Arc::new(fetcher));
        tokio::pin!(download_fut);
        tokio::select! {
            res = &mut download_fut => Some(res.map_err(CliError::Download)),
            _ = cancel.cancelled() => None,
        }
    };
    // `job` owns the event_sender. Drop it so the progress channel closes and
    // the renderer task ends; otherwise `progress_handle.await` blocks forever
    // (the renderer loops until every sender drops) and the job never reports
    // completion to the client.
    drop(job);
    let _ = progress_handle.await;
    let stats = match stats_result {
        Some(res) => res?,
        None => return Err(CliError::Cancelled),
    };

    // Mark as installed (clears installing state)
    let _ = steamroom_client::depot_config::DepotConfig::save_manifest_decompressed(
        &output_dir,
        depot_id,
        manifest_id,
        &manifest_bytes,
    );
    depot_config.set_installed(depot_id, manifest_id, &depot_key);
    let _ = depot_config.save(&output_dir);

    // Optionally save the raw CDN manifest for preservation
    if args.save_manifests
        && let Some(ref raw) = cdn_raw
    {
        let _ = steamroom_client::depot_config::DepotConfig::save_manifest_raw(
            &output_dir,
            depot_id,
            manifest_id,
            raw,
        );
    }

    let mut summary = format!(
        "download complete: {} files, {}",
        stats.files_completed,
        fmt_size(stats.bytes_downloaded),
    );
    if stats.files_removed > 0 {
        summary.push_str(&format!(", {} removed", stats.files_removed));
    }
    info!("{summary}");
    Ok(())
}

/// Map each file in the previously-installed manifest to its chunk layout
/// (identity + offset in the installed file) for content-addressed reuse.
/// Offsets absent from the manifest fall back to a running cumulative sum,
/// matching how the download pipeline positions chunks.
fn old_file_layouts(old: &DepotManifest) -> HashMap<String, Vec<OldChunkLoc>> {
    let mut map = HashMap::with_capacity(old.files.len());
    for f in &old.files {
        if f.chunks.is_empty() {
            continue;
        }
        let mut locs = Vec::with_capacity(f.chunks.len());
        let mut running: u64 = 0;
        for c in &f.chunks {
            let offset = c.offset.unwrap_or(running);
            locs.push(OldChunkLoc {
                id: c.id.clone(),
                offset,
                size: c.uncompressed_size,
            });
            running = offset + u64::from(c.uncompressed_size);
        }
        map.insert(f.normalized_path(), locs);
    }
    map
}
