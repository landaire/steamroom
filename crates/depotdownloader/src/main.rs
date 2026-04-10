mod cli;
mod download;
mod errors;

use std::path::PathBuf;
use clap::Parser;
use prost::Message;
use tracing::{debug, error, info, warn};
use cli::*;
use errors::CliError;
use steam::apps::{AccessToken, AppInfo};
use steam::cdn::CdnClient;
use steam::client::{self, SteamClient, LoggedIn, PROTOCOL_VERSION};
use steam::client::msg::ClientMsg;
use steam::connection;
use steam::depot::*;
use steam::depot::chunk;
use steam::depot::manifest::DepotManifest;
use steam::messages::EMsg;
use steam::transport::tcp::TcpTransport;
use steam::types::key_value::{self, KeyValue, KvValue};

#[tokio::main]
async fn main() -> Result<(), CliError> {
    let cli = Cli::parse();
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| {
                    if cli.debug {
                        "debug".into()
                    } else {
                        "info".into()
                    }
                }),
        )
        .init();

    match cli.command {
        Command::Info(args) => run_info(args).await,
        Command::Manifests(args) => run_manifests(args).await,
        Command::Files(args) => run_files(args).await,
        Command::Download(args) => run_download(args, &cli.auth).await,
        Command::Workshop(args) => run_workshop(args).await,
    }
}

async fn connect_and_login(
    auth: &AuthOptions,
) -> Result<SteamClient<LoggedIn>, CliError> {
    info!("discovering CM servers...");
    let servers = connection::fetch_cm_servers().await.unwrap_or_else(|_| {
        warn!("failed to fetch CM servers, using defaults");
        connection::default_cm_servers()
    });

    let server = servers.first().ok_or_else(|| CliError::Other("no CM servers".into()))?;
    info!("connecting to {:?}", server.addr);

    let transport = TcpTransport::connect(server).await?;
    let (client, _rx) = SteamClient::connect(transport).await?;

    info!("encrypting connection...");
    let client = client.encrypt().await?;

    // Build logon message
    let (logon, steam_id) = build_logon_body(auth);
    let logon_bytes = logon.encode_to_vec();
    let mut msg = ClientMsg::with_body(EMsg(5514), &logon_bytes);
    msg.header.steamid = Some(steam_id);

    info!("logging in...");
    let (client, _resp) = client.login(msg).await?;

    info!("logged in successfully");
    Ok(client)
}

fn build_logon_body(auth: &AuthOptions) -> (steam::generated::CMsgClientLogon, u64) {
    let mut logon = steam::generated::CMsgClientLogon {
        protocol_version: Some(PROTOCOL_VERSION),
        cell_id: Some(0),
        client_os_type: Some(16), // Windows
        ..Default::default()
    };

    if let Some(ref username) = auth.username {
        logon.account_name = Some(username.clone());
        // If we have an access token, use that; otherwise password
        // For now just set the fields
        if let Some(ref password) = auth.password {
            logon.password = Some(password.clone());
        }
        // Individual account SteamID
        let steam_id = steam::types::SteamId::from_parts(1, 1, 1, 0);
        return (logon, steam_id.raw());
    }

    // Anonymous login
    // Anonymous user: universe=1, type=AnonUser(10), instance=1, account_id=0
    let steam_id = steam::types::SteamId::from_parts(1, 10, 1, 0);
    (logon, steam_id.raw())
}

async fn run_download(args: DownloadArgs, auth: &AuthOptions) -> Result<(), CliError> {
    let client = connect_and_login(auth).await?;
    let app_id = AppId(args.app);

    // Get access tokens
    info!("getting PICS access tokens for app {}", app_id);
    let tokens = client.pics_get_access_tokens(&[app_id]).await?;
    let token = tokens.into_iter().next().unwrap_or(AccessToken {
        app_id,
        token: 0,
    });

    // Get product info
    info!("getting product info...");
    let infos = client.pics_get_product_info(&[token]).await?;
    let app_info = infos
        .into_iter()
        .next()
        .ok_or_else(|| CliError::Other("no product info returned".into()))?;

    // Parse KV data
    let kv_data = app_info.kv_data.ok_or_else(|| CliError::Other("no KV data".into()))?;
    let kv = parse_app_kv(&kv_data)?;

    // Find depots
    let depot_id = if let Some(d) = args.depot {
        DepotId(d)
    } else {
        // Find first depot from the KV data
        let depots_kv = kv.get("depots").ok_or_else(|| CliError::Other("no depots in app info".into()))?;
        find_first_depot(&depots_kv)?
    };

    let branch = args.branch.as_deref().unwrap_or("public");

    // Find manifest ID
    let manifest_id = if let Some(m) = args.manifest {
        ManifestId(m)
    } else {
        let depots_kv = kv.get("depots").ok_or_else(|| CliError::Other("no depots".into()))?;
        find_manifest_for_depot(&depots_kv, depot_id, branch)?
    };

    info!("depot={}, manifest={}, branch={}", depot_id, manifest_id, branch);

    // Get depot decryption key
    info!("getting depot decryption key...");
    let depot_key = client.get_depot_decryption_key(depot_id, app_id).await?;

    // Get CDN servers
    info!("getting CDN servers...");
    let cdn_servers = client
        .get_cdn_servers(CellId(0), Some(20))
        .await?;
    let cdn_server = cdn_servers
        .first()
        .ok_or_else(|| CliError::Other("no CDN servers".into()))?;
    info!("using CDN server: {}", cdn_server.host);

    // Get manifest request code
    info!("getting manifest request code...");
    let request_code = client
        .get_manifest_request_code(app_id, depot_id, manifest_id, Some(branch), None)
        .await?
        .unwrap_or(0);

    // Download manifest
    let cdn = CdnClient::new().map_err(|e| CliError::Steam(e))?;
    info!("downloading manifest...");
    let manifest_data = cdn
        .download_manifest(cdn_server, depot_id, manifest_id, request_code, None)
        .await?;

    // Decompress manifest (it's zipped)
    let manifest_bytes = decompress_manifest(&manifest_data)?;

    // Parse manifest
    let mut manifest = DepotManifest::parse(&manifest_bytes)?;
    if manifest.filenames_encrypted {
        manifest.decrypt_filenames(&depot_key)?;
    }

    let output_dir = args.output.unwrap_or_else(|| PathBuf::from("depots").join(depot_id.0.to_string()));
    std::fs::create_dir_all(&output_dir)?;

    info!(
        "manifest has {} files, downloading to {}",
        manifest.files.len(),
        output_dir.display()
    );

    // Download chunks
    let mut downloaded_bytes: u64 = 0;
    let total_bytes: u64 = manifest.files.iter().filter_map(|f| f.size).sum();

    for file in &manifest.files {
        let filename = file.filename.as_deref().unwrap_or("unknown");
        let file_path = output_dir.join(filename);

        // Check if it's a directory
        if let Some(flags) = file.flags {
            let depot_flags = steam::enums::DepotFileFlags(flags);
            if depot_flags.is_directory() {
                std::fs::create_dir_all(&file_path)?;
                continue;
            }
        }

        // Skip zero-size files (just create them)
        if file.size.unwrap_or(0) == 0 && file.chunks.is_empty() {
            if let Some(parent) = file_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&file_path, &[])?;
            continue;
        }

        // Check for symlinks
        if let Some(ref target) = file.link_target {
            if let Some(parent) = file_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            // Just write a text file with the target for now
            debug!("symlink: {} -> {}", filename, target);
            continue;
        }

        if let Some(parent) = file_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        debug!("downloading: {} ({} chunks)", filename, file.chunks.len());

        // Download and assemble chunks in order
        let mut file_data = Vec::with_capacity(file.size.unwrap_or(0) as usize);

        for chunk_meta in &file.chunks {
            let chunk_id = chunk_meta.id.as_ref().ok_or_else(|| {
                CliError::Other("chunk missing ID".into())
            })?;

            let chunk_data = cdn
                .download_chunk(cdn_server, depot_id, chunk_id, None)
                .await?;

            let processed = chunk::process_chunk(
                &chunk_data,
                &depot_key,
                chunk_meta.uncompressed_size.unwrap_or(0),
                chunk_meta.checksum.unwrap_or(0),
            )?;

            file_data.extend_from_slice(&processed);
            downloaded_bytes += processed.len() as u64;
        }

        std::fs::write(&file_path, &file_data)?;

        let pct = if total_bytes > 0 {
            downloaded_bytes as f64 / total_bytes as f64 * 100.0
        } else {
            100.0
        };
        info!(
            "[{:.1}%] {} ({})",
            pct,
            filename,
            fmt_size(file_data.len() as u64)
        );
    }

    info!("download complete: {} total", fmt_size(downloaded_bytes));
    Ok(())
}

fn parse_app_kv(data: &[u8]) -> Result<KeyValue, CliError> {
    // PICS KV data can be binary KV or text
    // Binary KV starts with 0x00 tag
    if data.first() == Some(&0x00) {
        key_value::parse_binary_kv(data).map_err(|e| CliError::Io(e))
    } else {
        // Try text parse, skip any leading null bytes
        let text = String::from_utf8_lossy(data);
        key_value::parse_text_kv(&text).map_err(|e| CliError::Other(e.to_string()))
    }
}

fn find_first_depot(depots_kv: &KeyValue) -> Result<DepotId, CliError> {
    if let KvValue::Children(ref map) = depots_kv.value {
        for (key, _) in map {
            if let Ok(id) = key.parse::<u32>() {
                if id > 0 {
                    return Ok(DepotId(id));
                }
            }
        }
    }
    Err(CliError::Other("no depots found".into()))
}

fn find_manifest_for_depot(
    depots_kv: &KeyValue,
    depot_id: DepotId,
    branch: &str,
) -> Result<ManifestId, CliError> {
    let depot_key = depot_id.0.to_string();
    let depot = depots_kv
        .get(&depot_key)
        .ok_or_else(|| CliError::Other(format!("depot {} not found in app info", depot_id)))?;

    // Look in depots -> {depot_id} -> manifests -> {branch} -> gid
    if let Some(manifests) = depot.get("manifests") {
        if let Some(branch_kv) = manifests.get(branch) {
            if let Some(gid) = branch_kv.get("gid") {
                if let Some(gid_str) = gid.as_str() {
                    let id: u64 = gid_str
                        .parse()
                        .map_err(|_| CliError::Other("invalid manifest ID".into()))?;
                    return Ok(ManifestId(id));
                }
            }
            // Maybe branch_kv itself is a string (manifest ID directly)
            if let Some(gid_str) = branch_kv.as_str() {
                let id: u64 = gid_str
                    .parse()
                    .map_err(|_| CliError::Other("invalid manifest ID".into()))?;
                return Ok(ManifestId(id));
            }
        }
    }

    Err(CliError::Other(format!(
        "manifest not found for depot {} branch {}",
        depot_id, branch
    )))
}

fn decompress_manifest(data: &[u8]) -> Result<Vec<u8>, CliError> {
    // Manifest data from CDN is zip-compressed
    if data.len() > 2 && data[0] == 0x50 && data[1] == 0x4B {
        let cursor = std::io::Cursor::new(data);
        let mut archive = zip::ZipArchive::new(cursor)
            .map_err(|e| CliError::Other(format!("zip: {e}")))?;
        if archive.is_empty() {
            return Err(CliError::Other("empty manifest archive".into()));
        }
        let mut file = archive.by_index(0)
            .map_err(|e| CliError::Other(format!("zip: {e}")))?;
        let mut buf = Vec::new();
        std::io::Read::read_to_end(&mut file, &mut buf)?;
        Ok(buf)
    } else {
        // Not compressed, return as-is
        Ok(data.to_vec())
    }
}

fn fmt_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.2} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}

async fn run_info(_args: InfoArgs) -> Result<(), CliError> {
    todo!()
}

async fn run_manifests(_args: ManifestsArgs) -> Result<(), CliError> {
    todo!()
}

async fn run_files(_args: FilesArgs) -> Result<(), CliError> {
    todo!()
}

async fn run_workshop(_args: WorkshopArgs) -> Result<(), CliError> {
    todo!()
}
