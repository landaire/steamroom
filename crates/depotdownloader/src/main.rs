mod cli;
mod download;
mod errors;

use std::path::PathBuf;
use clap::Parser;
use prost::Message;
use tracing::{debug, info, warn};
use cli::*;
use errors::CliError;
use steam::apps::AccessToken;
use steam::cdn::CdnClient;
use steam::client::{SteamClient, LoggedIn, PROTOCOL_VERSION};
use steam::client::msg::ClientMsg;
use steam::connection;
use steam::depot::*;
use steam::depot::chunk;
use steam::depot::manifest::DepotManifest;
use steam::messages::EMsg;
use steam::transport::websocket::WebSocketTransport;
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
        Command::Info(args) => run_info(args, &cli.auth).await,
        Command::Manifests(args) => run_manifests(args, &cli.auth).await,
        Command::Files(args) => run_files(args, &cli.auth).await,
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

    // Try TCP first if available, fall back to WebSocket
    let tcp_server = servers
        .iter()
        .find(|s| s.protocol == connection::Protocol::Tcp);

    let client = if let Some(server) = tcp_server {
        info!("connecting via TCP to {:?}", server.addr);
        let transport = steam::transport::tcp::TcpTransport::connect(server).await?;
        let (client, _rx) = SteamClient::connect(transport).await?;
        info!("encrypting...");
        client.encrypt().await?
    } else {
        let ws_server = servers
            .iter()
            .find(|s| s.protocol == connection::Protocol::WebSocket)
            .ok_or(CliError::NoCmServers)?;
        info!("connecting via WebSocket to {:?}", ws_server.addr);
        let transport = WebSocketTransport::connect(ws_server).await?;
        let (client, _rx) = SteamClient::connect_ws(transport).await?;
        client
    };

    // Determine auth mode and get token if needed
    let (logon, steam_id) = if let Some(ref username) = auth.username {
        if let Some(token) = load_saved_token(username) {
            info!("using saved refresh token for {username}");
            build_token_logon(username, &token)
        } else if auth.qr {
            let tokens = authenticate_qr(&client).await?;
            save_token(&tokens.account_name.as_deref().unwrap_or(username), &tokens.refresh_token);
            build_token_logon(
                tokens.account_name.as_deref().unwrap_or(username),
                &tokens.access_token,
            )
        } else {
            let password = auth.password.clone().unwrap_or_else(|| {
                rpassword::prompt_password(format!("Password for {username}: ")).unwrap_or_default()
            });
            let tokens = authenticate_credentials(&client, username, &password).await?;
            save_token(&tokens.account_name.as_deref().unwrap_or(username), &tokens.refresh_token);
            build_token_logon(
                tokens.account_name.as_deref().unwrap_or(username),
                &tokens.access_token,
            )
        }
    } else {
        build_anon_logon()
    };

    let logon_bytes = logon.encode_to_vec();
    let mut msg = ClientMsg::with_body(EMsg(5514), &logon_bytes);
    msg.header.steamid = Some(steam_id);
    msg.header.client_sessionid = Some(0);

    info!("logging in...");
    let (client, _resp) = client.login(msg).await?;

    info!("logged in successfully");
    Ok(client)
}

fn tokens_path() -> Option<std::path::PathBuf> {
    Some(dirs_next::home_dir()?.join(".depotdownloader").join("tokens.json"))
}

fn load_saved_token(username: &str) -> Option<String> {
    let data = std::fs::read_to_string(tokens_path()?).ok()?;
    let parsed: serde_json::Value = serde_json::from_str(&data).ok()?;
    parsed["tokens"][username].as_str().map(|s| s.to_string())
}

fn save_token(username: &str, refresh_token: &str) {
    let Some(path) = tokens_path() else { return };
    let mut root = match std::fs::read_to_string(&path) {
        Ok(data) => serde_json::from_str::<serde_json::Value>(&data).unwrap_or_default(),
        Err(_) => serde_json::json!({}),
    };
    root["tokens"][username] = serde_json::Value::String(refresh_token.to_string());
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&path, serde_json::to_string_pretty(&root).unwrap_or_default());
    info!("saved refresh token for {username}");
}

async fn authenticate_credentials(
    client: &SteamClient<steam::client::Encrypted>,
    username: &str,
    password: &str,
) -> Result<steam::auth::AuthTokens, CliError> {
    info!("getting RSA public key for {username}...");
    let rsa = client.get_password_rsa_public_key(username).await?;
    let modulus = rsa.publickey_mod.ok_or(CliError::Steam(
        steam::error::Error::Connection(steam::error::ConnectionError::EncryptionFailed),
    ))?;
    let exponent = rsa.publickey_exp.ok_or(CliError::Steam(
        steam::error::Error::Connection(steam::error::ConnectionError::EncryptionFailed),
    ))?;
    let timestamp = rsa.timestamp.unwrap_or(0);

    let encrypted_password = steam::crypto::rsa::encrypt_with_rsa_public_key(
        password.as_bytes(),
        &modulus,
        &exponent,
    )?;
    let encoded_password = base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        &encrypted_password,
    );

    info!("beginning auth session...");
    let req = steam::generated::CAuthenticationBeginAuthSessionViaCredentialsRequest {
        account_name: Some(username.to_string()),
        encrypted_password: Some(encoded_password),
        encryption_timestamp: Some(timestamp),
        remember_login: Some(true),
        persistence: Some(1), // Persistent
        ..Default::default()
    };
    let session = client.begin_auth_session_via_credentials(req).await?;

    // Handle 2FA if required
    for guard in &session.allowed_confirmations {
        match guard {
            steam::auth::GuardType::DeviceCode | steam::auth::GuardType::EmailCode => {
                let prompt = match guard {
                    steam::auth::GuardType::DeviceCode => "Steam Guard code (from authenticator app): ",
                    steam::auth::GuardType::EmailCode => "Steam Guard code (from email): ",
                    _ => unreachable!(),
                };
                let code = rpassword::prompt_password(prompt).unwrap_or_default();
                if let (Some(client_id), Some(steam_id)) = (session.client_id, session.steam_id) {
                    client
                        .submit_steam_guard_code(client_id, steam_id, &code, *guard)
                        .await?;
                }
                break;
            }
            steam::auth::GuardType::DeviceConfirmation => {
                info!("confirm login on your Steam mobile app...");
                break;
            }
            _ => {}
        }
    }

    // Poll for tokens
    let client_id = session.client_id.unwrap_or(0);
    let request_id = session.request_id.unwrap_or_default();
    let interval = session.poll_interval.unwrap_or(5.0);

    loop {
        tokio::time::sleep(std::time::Duration::from_secs_f32(interval)).await;
        if let Some(tokens) = client.poll_auth_session(client_id, &request_id).await? {
            return Ok(tokens);
        }
    }
}

async fn authenticate_qr(
    client: &SteamClient<steam::client::Encrypted>,
) -> Result<steam::auth::AuthTokens, CliError> {
    info!("generating QR code...");
    let req = steam::generated::CAuthenticationBeginAuthSessionViaQrRequest {
        device_friendly_name: Some("ddl".to_string()),
        ..Default::default()
    };
    let session = client.begin_auth_session_via_qr(req).await?;

    if let Some(ref url) = session.challenge_url {
        // Print QR code to terminal
        let qr = qrcode::QrCode::new(url.as_bytes()).map_err(|e| {
            CliError::Steam(steam::error::Error::Io(std::io::Error::new(
                std::io::ErrorKind::Other,
                e,
            )))
        })?;
        let rendered = qr
            .render::<qrcode::render::unicode::Dense1x2>()
            .build();
        eprintln!("{rendered}");
        eprintln!("Scan this QR code with the Steam mobile app");
        eprintln!("Or open: {url}");
    }

    let client_id = session.client_id.unwrap_or(0);
    let request_id = session.request_id.unwrap_or_default();
    let interval = session.poll_interval.unwrap_or(5.0);

    loop {
        tokio::time::sleep(std::time::Duration::from_secs_f32(interval)).await;
        if let Some(tokens) = client.poll_auth_session(client_id, &request_id).await? {
            return Ok(tokens);
        }
    }
}

fn build_token_logon(username: &str, token: &str) -> (steam::generated::CMsgClientLogon, u64) {
    let logon = steam::generated::CMsgClientLogon {
        protocol_version: Some(PROTOCOL_VERSION),
        cell_id: Some(0),
        client_os_type: Some(20),
        account_name: Some(username.to_string()),
        access_token: Some(token.to_string()),
        ..Default::default()
    };
    let steam_id = steam::types::SteamId::from_parts(1, 1, 1, 0);
    (logon, steam_id.raw())
}

fn build_anon_logon() -> (steam::generated::CMsgClientLogon, u64) {
    let logon = steam::generated::CMsgClientLogon {
        protocol_version: Some(PROTOCOL_VERSION),
        cell_id: Some(0),
        client_os_type: Some(20),
        ..Default::default()
    };
    let steam_id = steam::types::SteamId::from_parts(1, 10, 0, 0);
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
        .ok_or_else(|| CliError::NoProductInfo(app_id.0))?;

    // Parse KV data
    let kv_data = app_info.kv_data.ok_or_else(|| CliError::NoKvData(app_id.0))?;
    let kv = parse_app_kv(&kv_data)?;

    // Find depots
    let depot_id = if let Some(d) = args.depot {
        DepotId(d)
    } else {
        // Find first depot from the KV data
        let depots_kv = kv.get("depots").ok_or_else(|| CliError::NoDepots)?;
        find_first_depot(&depots_kv)?
    };

    let branch = args.branch.as_deref().unwrap_or("public");

    // Find manifest ID
    let manifest_id = if let Some(m) = args.manifest {
        ManifestId(m)
    } else {
        let depots_kv = kv.get("depots").ok_or_else(|| CliError::NoDepots)?;
        find_manifest_for_depot(&depots_kv, depot_id, branch)?
    };

    info!("depot={}, manifest={}, branch={}", depot_id, manifest_id, branch);

    // Get depot decryption key
    info!("getting depot key for depot {depot_id} app {app_id}...");
    let depot_key = client.get_depot_decryption_key(depot_id, app_id).await?;
    debug!("depot key: {:02x?}", &depot_key.0);

    // Get CDN servers
    info!("getting CDN servers...");
    let cdn_servers = client
        .get_cdn_servers(CellId(0), Some(20))
        .await?;
    let cdn_server = cdn_servers
        .first()
        .ok_or_else(|| CliError::NoCdnServers)?;
    info!("using CDN server: {}", cdn_server.host);

    // Get manifest request code
    info!("getting manifest request code...");
    let request_code = client
        .get_manifest_request_code(app_id, depot_id, manifest_id, Some(branch), None)
        .await?
        .unwrap_or(0);

    // Download manifest (with cache)
    let cdn = CdnClient::new().map_err(CliError::Steam)?;
    let manifest_cache = steam_client::manifest::ManifestCache::new(
        steam_client::manifest::ManifestCache::default_path(),
    );

    let manifest_bytes = if let Some(cached) = manifest_cache.load(depot_id, manifest_id) {
        debug!("using cached manifest for {depot_id}_{manifest_id}");
        cached
    } else {
        info!("downloading manifest...");
        let manifest_data = cdn
            .download_manifest(cdn_server, depot_id, manifest_id, request_code, None)
            .await?;
        let decompressed = decompress_manifest(&manifest_data)?;
        let _ = manifest_cache.save(depot_id, manifest_id, &decompressed);
        decompressed
    };

    // Debug: dump section magics
    {
        let mut off = 0;
        while off + 8 <= manifest_bytes.len() {
            let magic = u32::from_le_bytes(manifest_bytes[off..off+4].try_into().unwrap());
            let size = u32::from_le_bytes(manifest_bytes[off+4..off+8].try_into().unwrap());
            debug!("  section at {off}: magic=0x{magic:08x} size={size}");
            if magic == 0xD64BF064 { break; }
            off += 8 + size as usize;
        }
    }

    // Parse manifest
    let mut manifest = DepotManifest::parse(&manifest_bytes)?;
    info!("manifest parsed: {} files, encrypted={}", manifest.files.len(), manifest.filenames_encrypted);
    if manifest.filenames_encrypted {
        match manifest.decrypt_filenames(&depot_key) {
            Ok(()) => info!("decrypted filenames"),
            Err(e) => warn!("filename decryption failed ({e}), using raw names"),
        }
    }

    let output_dir = args.output.unwrap_or_else(|| PathBuf::from("depots").join(depot_id.0.to_string()));
    std::fs::create_dir_all(&output_dir)?;

    // Set up download orchestration
    let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel();

    let fetcher = steam_client::download::CdnChunkFetcher {
        cdn,
        server: cdn_server.clone(),
        cdn_auth_token: None,
    };

    let mut builder = steam_client::download::DepotJob::builder()
        .depot_id(depot_id)
        .depot_key(depot_key)
        .install_dir(output_dir.clone())
        .event_sender(event_tx);

    if let Some(max) = args.max_downloads {
        builder = builder.max_downloads(max);
    }

    if let Some(ref filelist_path) = args.filelist {
        let content = std::fs::read_to_string(filelist_path)?;
        let files: Vec<String> = content.lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect();
        builder = builder.file_filter(steam_client::download::FileFilter::FileList(files));
    } else if let Some(ref pattern) = args.file_regex {
        builder = builder.file_filter(steam_client::download::FileFilter::Regex(regex::Regex::new(pattern)?));
    }

    let job = builder.build().map_err(|e| CliError::Steam(steam::error::Error::Io(
        std::io::Error::new(std::io::ErrorKind::InvalidInput, e),
    )))?;

    let total_bytes: u64 = manifest.files.iter().filter_map(|f| f.size).sum();
    info!(
        "downloading {} files ({}) to {}",
        manifest.files.len(),
        fmt_size(total_bytes),
        output_dir.display()
    );

    // Spawn progress renderer
    let progress_handle = tokio::spawn(async move {
        let mut completed: u64 = 0;
        while let Some(event) = event_rx.recv().await {
            match event {
                steam_client::event::DownloadEvent::FileCompleted { filename } => {
                    let pct = if total_bytes > 0 {
                        completed as f64 / total_bytes as f64 * 100.0
                    } else {
                        0.0
                    };
                    info!("[{pct:.1}%] {filename}");
                }
                steam_client::event::DownloadEvent::ChunkCompleted { bytes } => {
                    completed += bytes;
                }
                steam_client::event::DownloadEvent::ChunkFailed { error } => {
                    warn!("chunk failed (retrying): {error}");
                }
                _ => {}
            }
        }
    });

    let stats = job.download(&manifest, &fetcher).await.map_err(|e| {
        CliError::Steam(steam::error::Error::Io(
            std::io::Error::new(std::io::ErrorKind::Other, e),
        ))
    })?;

    drop(job); // drop to close the event channel
    let _ = progress_handle.await;

    info!(
        "download complete: {} files, {}",
        stats.files_completed,
        fmt_size(stats.bytes_downloaded)
    );
    Ok(())
}

fn parse_app_kv(data: &[u8]) -> Result<KeyValue, CliError> {
    // PICS KV data can be binary KV or text
    // Binary KV starts with 0x00 tag
    if data.first() == Some(&0x00) {
        key_value::parse_binary_kv(data).map_err(CliError::Io)
    } else {
        // Try text parse, skip any leading null bytes
        let text = String::from_utf8_lossy(data);
        Ok(key_value::parse_text_kv(&text)?)
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
    Err(CliError::NoDepots)
}

fn find_manifest_for_depot(
    depots_kv: &KeyValue,
    depot_id: DepotId,
    branch: &str,
) -> Result<ManifestId, CliError> {
    let depot_key = depot_id.0.to_string();
    let depot = depots_kv
        .get(&depot_key)
        .ok_or(CliError::DepotNotFound(depot_id.0))?;

    // Look in depots -> {depot_id} -> manifests -> {branch} -> gid
    if let Some(manifests) = depot.get("manifests") {
        if let Some(branch_kv) = manifests.get(branch) {
            if let Some(gid) = branch_kv.get("gid") {
                if let Some(gid_str) = gid.as_str() {
                    let id: u64 = gid_str
                        .parse()
                        .map_err(|_| CliError::InvalidManifestId)?;
                    return Ok(ManifestId(id));
                }
            }
            // Maybe branch_kv itself is a string (manifest ID directly)
            if let Some(gid_str) = branch_kv.as_str() {
                let id: u64 = gid_str
                    .parse()
                    .map_err(|_| CliError::InvalidManifestId)?;
                return Ok(ManifestId(id));
            }
        }
    }

    Err(CliError::ManifestNotFound {
        depot: depot_id.0,
        branch: branch.to_string(),
    })
}

fn decompress_manifest(data: &[u8]) -> Result<Vec<u8>, CliError> {
    // Manifest data from CDN is zip-compressed
    if data.len() > 2 && data[0] == 0x50 && data[1] == 0x4B {
        let cursor = std::io::Cursor::new(data);
        let mut archive = zip::ZipArchive::new(cursor)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        if archive.is_empty() {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "empty manifest archive").into());
        }
        let mut file = archive.by_index(0)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
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

async fn fetch_app_kv(
    auth: &AuthOptions,
    app_id: AppId,
) -> Result<(SteamClient<LoggedIn>, KeyValue), CliError> {
    let client = connect_and_login(auth).await?;
    let tokens = client.pics_get_access_tokens(&[app_id]).await?;
    let token = tokens.into_iter().next().unwrap_or(AccessToken {
        app_id,
        token: 0,
    });
    let infos = client.pics_get_product_info(&[token]).await?;
    let app_info = infos
        .into_iter()
        .next()
        .ok_or(CliError::NoProductInfo(app_id.0))?;
    let kv_data = app_info.kv_data.ok_or(CliError::NoKvData(app_id.0))?;
    let kv = parse_app_kv(&kv_data)?;
    Ok((client, kv))
}

async fn run_info(args: InfoArgs, auth: &AuthOptions) -> Result<(), CliError> {
    let app_id = AppId(args.app);
    let (_client, kv) = fetch_app_kv(auth, app_id).await?;

    let name = kv
        .get("common")
        .and_then(|c| c.get("name"))
        .and_then(|n| n.as_str())
        .unwrap_or("(unknown)");
    let app_type = kv
        .get("common")
        .and_then(|c| c.get("type"))
        .and_then(|t| t.as_str())
        .unwrap_or("(unknown)");

    println!("App ID:  {}", app_id);
    println!("Name:    {name}");
    println!("Type:    {app_type}");

    if let Some(depots) = kv.get("depots") {
        if let KvValue::Children(ref map) = depots.value {
            let depot_ids: Vec<&String> = map
                .keys()
                .filter(|k| k.parse::<u32>().is_ok())
                .collect();
            println!("Depots:  {}", depot_ids.len());
            for id in &depot_ids {
                let depot = map.get(*id).unwrap();
                let dname = depot
                    .get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or("");
                println!("  {id}: {dname}");
            }
        }
    }

    if let Some(depots) = kv.get("depots") {
        if let Some(branches) = depots.get("branches") {
            if let KvValue::Children(ref map) = branches.value {
                println!("Branches: {}", map.len());
                for (name, branch) in map {
                    let build_id = branch
                        .get("buildid")
                        .and_then(|b| b.as_str())
                        .unwrap_or("-");
                    let pwd = branch.get("pwdrequired").and_then(|p| p.as_str());
                    let lock = if pwd == Some("1") { " (password)" } else { "" };
                    println!("  {name}: build {build_id}{lock}");
                }
            }
        }
    }

    if args.format == Some(OutputFormat::Json) {
        println!("{}", serde_json::to_string_pretty(&kv_to_json(&kv))?);
    }

    Ok(())
}

async fn run_manifests(args: ManifestsArgs, auth: &AuthOptions) -> Result<(), CliError> {
    let app_id = AppId(args.app);
    let (_client, kv) = fetch_app_kv(auth, app_id).await?;
    let branch = args.branch.as_deref().unwrap_or("public");

    let depots = kv.get("depots").ok_or(CliError::NoDepots)?;
    if let KvValue::Children(ref map) = depots.value {
        for (key, depot) in map {
            let Ok(depot_id) = key.parse::<u32>() else {
                continue;
            };
            if let Some(manifests) = depot.get("manifests") {
                if let Some(branch_kv) = manifests.get(branch) {
                    let gid = branch_kv
                        .get("gid")
                        .and_then(|g| g.as_str())
                        .or_else(|| branch_kv.as_str());
                    if let Some(manifest_id) = gid {
                        let dname = depot
                            .get("name")
                            .and_then(|n| n.as_str())
                            .unwrap_or("");
                        println!("{depot_id}\t{manifest_id}\t{dname}");
                    }
                }
            }
        }
    }

    Ok(())
}

async fn run_files(args: FilesArgs, auth: &AuthOptions) -> Result<(), CliError> {
    let app_id = AppId(args.app);
    let (client, kv) = fetch_app_kv(auth, app_id).await?;
    let branch = args.branch.as_deref().unwrap_or("public");

    let depot_id = args
        .depot
        .map(DepotId)
        .or_else(|| {
            kv.get("depots")
                .and_then(|d| find_first_depot(d).ok())
        })
        .ok_or(CliError::NoDepots)?;

    let manifest_id = args.manifest.map(ManifestId).or_else(|| {
        kv.get("depots")
            .and_then(|d| find_manifest_for_depot(d, depot_id, branch).ok())
    }).ok_or(CliError::ManifestNotFound {
        depot: depot_id.0,
        branch: branch.to_string(),
    })?;

    let depot_key = client.get_depot_decryption_key(depot_id, app_id).await?;
    let request_code = client
        .get_manifest_request_code(app_id, depot_id, manifest_id, Some(branch), None)
        .await?
        .unwrap_or(0);

    let cdn_servers = client.get_cdn_servers(CellId(0), Some(5)).await?;
    let cdn_server = cdn_servers.first().ok_or(CliError::NoCdnServers)?;
    let cdn = CdnClient::new().map_err(CliError::Steam)?;
    let manifest_data = cdn
        .download_manifest(cdn_server, depot_id, manifest_id, request_code, None)
        .await?;
    let manifest_bytes = decompress_manifest(&manifest_data)?;
    let mut manifest = DepotManifest::parse(&manifest_bytes)?;
    if manifest.filenames_encrypted {
        manifest.decrypt_filenames(&depot_key)?;
    }

    for file in &manifest.files {
        let name = file.filename.as_deref().unwrap_or("(encrypted)");
        let size = file.size.unwrap_or(0);
        let flags = file.flags.unwrap_or(0);
        let is_dir = steam::enums::DepotFileFlags(flags).is_directory();
        if is_dir {
            println!("{name}/");
        } else {
            println!("{name}\t{}", fmt_size(size));
        }
    }

    Ok(())
}

fn kv_to_json(kv: &KeyValue) -> serde_json::Value {
    match &kv.value {
        KvValue::Children(map) => {
            let obj: serde_json::Map<String, serde_json::Value> = map
                .iter()
                .map(|(k, v)| (k.clone(), kv_to_json(v)))
                .collect();
            serde_json::Value::Object(obj)
        }
        KvValue::String(s) => serde_json::Value::String(s.clone()),
        KvValue::Int32(v) => serde_json::Value::Number((*v).into()),
        KvValue::UInt64(v) => serde_json::Value::Number((*v).into()),
        KvValue::Int64(v) => serde_json::Value::Number((*v).into()),
        KvValue::Float32(v) => {
            serde_json::Number::from_f64(*v as f64)
                .map(serde_json::Value::Number)
                .unwrap_or(serde_json::Value::Null)
        }
        _ => serde_json::Value::Null,
    }
}

async fn run_workshop(_args: WorkshopArgs) -> Result<(), CliError> {
    todo!()
}
