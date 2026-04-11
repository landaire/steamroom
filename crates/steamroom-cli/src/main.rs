mod cli;
mod download;
mod errors;

use clap::Parser;
use cli::*;
use errors::CliError;
use prost::Message;
use std::path::PathBuf;
use steamroom::apps::AccessToken;
use steamroom::cdn::CdnClient;
use steamroom::client::msg::ClientMsg;
use steamroom::client::LoggedIn;
use steamroom::client::SteamClient;
use steamroom::client::PROTOCOL_VERSION;
use steamroom::connection;
use steamroom::depot::manifest::DepotManifest;
use steamroom::depot::*;
use steamroom::messages::EMsg;
use steamroom::transport::websocket::WebSocketTransport;
use steamroom::types::key_value;
use steamroom::types::key_value::KeyValue;
use steamroom::types::key_value::KvValue;

use tracing::debug;
use tracing::info;
use tracing::warn;

fn main() {
    let cli = if std::env::var("DD_COMPAT").as_deref() == Ok("1") {
        cli::CompatCli::parse().into_cli()
    } else {
        Cli::parse()
    };
    let default_filter = if cli.quiet {
        "off"
    } else if cli.debug {
        "debug"
    } else if cfg!(debug_assertions) {
        "warn,steamroom=debug,steamroom_client=debug,steamroom_ffi=debug,steamroom_cli=debug"
    } else {
        "warn"
    };
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| default_filter.into()),
        )
        .init();

    let cpus = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .max_blocking_threads(cpus)
        .build()
        .expect("failed to build tokio runtime");

    let raw_errors = cli.raw_errors;
    if let Err(err) = rt.block_on(async_main(cli)) {
        if raw_errors {
            // Wrap in rootcause Report for full context chain
            let report: rootcause::Report<CliError> = rootcause::report!(err);
            eprintln!("Error: {report:?}");
        } else {
            eprintln!("Error: {err}");
        }
        std::process::exit(1);
    }
}

async fn async_main(cli: Cli) -> Result<(), CliError> {
    let show_progress = !cli.no_progress;
    match cli.command {
        Command::Info(args) => run_info(args, &cli.auth).await,
        Command::Manifests(args) => run_manifests(args, &cli.auth).await,
        Command::Files(args) => run_files(args, &cli.auth).await,
        Command::Download(args) => run_download(args, &cli.auth, show_progress).await,
        Command::Workshop(args) => run_workshop(args, &cli.auth, show_progress).await,
    }
}

async fn connect_and_login(auth: &AuthOptions) -> Result<SteamClient<LoggedIn>, CliError> {
    info!("discovering CM servers...");
    let servers = connection::CmServer::fetch().await.unwrap_or_else(|_| {
        warn!("failed to fetch CM servers, using defaults");
        connection::CmServer::defaults()
    });

    // Try TCP first if available, fall back to WebSocket
    let tcp_server = servers
        .iter()
        .find(|s| s.protocol == connection::Protocol::Tcp);

    let client = if let Some(server) = tcp_server {
        info!("connecting via TCP to {:?}", server.addr);
        let transport = steamroom::transport::tcp::TcpTransport::connect(server).await?;
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
            save_token(
                tokens.account_name.as_deref().unwrap_or(username),
                &tokens.refresh_token,
            );
            build_token_logon(
                tokens.account_name.as_deref().unwrap_or(username),
                &tokens.access_token,
            )
        } else {
            let mut last_err = None;
            let mut tokens = None;
            for attempt in 0..3 {
                let password = if attempt == 0 {
                    auth.password.clone().unwrap_or_else(|| {
                        rpassword::prompt_password(format!("Password for {username}: "))
                            .unwrap_or_default()
                    })
                } else {
                    eprintln!("Invalid password, try again ({}/3)", attempt + 1);
                    rpassword::prompt_password(format!("Password for {username}: "))
                        .unwrap_or_default()
                };
                match authenticate_credentials(
                    &client,
                    username,
                    &password,
                    auth.device_name.as_deref(),
                )
                .await
                {
                    Ok(t) => {
                        tokens = Some(t);
                        break;
                    }
                    Err(CliError::Steam(steamroom::error::Error::Connection(
                        steamroom::error::ConnectionError::LogonFailed(
                            steamroom::enums::EResultError::InvalidPassword,
                        ),
                    ))) => {
                        last_err = Some(CliError::Steam(steamroom::error::Error::Connection(
                            steamroom::error::ConnectionError::LogonFailed(
                                steamroom::enums::EResultError::InvalidPassword,
                            ),
                        )));
                        continue;
                    }
                    Err(e) => return Err(e),
                }
            }
            let tokens = tokens.ok_or_else(|| last_err.unwrap())?;
            save_token(
                tokens.account_name.as_deref().unwrap_or(username),
                &tokens.refresh_token,
            );
            build_token_logon(
                tokens.account_name.as_deref().unwrap_or(username),
                &tokens.access_token,
            )
        }
    } else {
        build_anon_logon()
    };

    let logon_bytes = logon.encode_to_vec();
    let mut msg = ClientMsg::with_body(EMsg::CLIENT_LOGON, &logon_bytes);
    msg.header.steamid = Some(steam_id);
    msg.header.client_sessionid = Some(0);

    info!("logging in...");
    let (client, _resp) = client.login(msg).await?;

    info!("logged in successfully");
    Ok(client)
}

fn tokens_path() -> Option<std::path::PathBuf> {
    Some(
        dirs_next::home_dir()?
            .join(".depotdownloader")
            .join("tokens.json"),
    )
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
    let _ = std::fs::write(
        &path,
        serde_json::to_string_pretty(&root).unwrap_or_default(),
    );
    info!("saved refresh token for {username}");
}

async fn authenticate_credentials(
    client: &SteamClient<steamroom::client::Encrypted>,
    username: &str,
    password: &str,
    device_name: Option<&str>,
) -> Result<steamroom::auth::AuthTokens, CliError> {
    info!("getting RSA public key for {username}...");
    let rsa = client.get_password_rsa_public_key(username).await?;
    let modulus = rsa
        .publickey_mod
        .ok_or(CliError::Steam(steamroom::error::Error::Connection(
            steamroom::error::ConnectionError::EncryptionFailed,
        )))?;
    let exponent =
        rsa.publickey_exp
            .ok_or(CliError::Steam(steamroom::error::Error::Connection(
                steamroom::error::ConnectionError::EncryptionFailed,
            )))?;
    let timestamp = rsa.timestamp.unwrap_or(0);

    let encrypted_password = steamroom::crypto::rsa::encrypt_with_rsa_public_key(
        password.as_bytes(),
        &modulus,
        &exponent,
    )?;
    let encoded_password = base64::Engine::encode(
        &base64::engine::general_purpose::STANDARD,
        &encrypted_password,
    );

    info!("beginning auth session...");
    let req = steamroom::generated::CAuthenticationBeginAuthSessionViaCredentialsRequest {
        account_name: Some(username.to_string()),
        encrypted_password: Some(encoded_password),
        encryption_timestamp: Some(timestamp),
        remember_login: Some(true),
        persistence: Some(1),
        device_friendly_name: device_name.map(|s| s.to_string()),
        ..Default::default()
    };
    let session = client.begin_auth_session_via_credentials(req).await?;

    // Handle 2FA if required
    for guard in &session.allowed_confirmations {
        match guard {
            steamroom::auth::GuardType::DeviceCode | steamroom::auth::GuardType::EmailCode => {
                let prompt = match guard {
                    steamroom::auth::GuardType::DeviceCode => {
                        "Steam Guard code (from authenticator app): "
                    }
                    steamroom::auth::GuardType::EmailCode => "Steam Guard code (from email): ",
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
            steamroom::auth::GuardType::DeviceConfirmation => {
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
    client: &SteamClient<steamroom::client::Encrypted>,
) -> Result<steamroom::auth::AuthTokens, CliError> {
    info!("generating QR code...");
    let req = steamroom::generated::CAuthenticationBeginAuthSessionViaQrRequest {
        device_friendly_name: Some("steamroom".to_string()),
        ..Default::default()
    };
    let session = client.begin_auth_session_via_qr(req).await?;

    if let Some(ref url) = session.challenge_url {
        // Print QR code to terminal
        let qr = qrcode::QrCode::new(url.as_bytes())
            .map_err(|e| CliError::Io(std::io::Error::other(e)))?;
        let rendered = qr.render::<qrcode::render::unicode::Dense1x2>().build();
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

fn build_token_logon(username: &str, token: &str) -> (steamroom::generated::CMsgClientLogon, u64) {
    let logon = steamroom::generated::CMsgClientLogon {
        protocol_version: Some(PROTOCOL_VERSION),
        cell_id: Some(0),
        client_os_type: Some(20),
        account_name: Some(username.to_string()),
        access_token: Some(token.to_string()),
        ..Default::default()
    };
    let steam_id = steamroom::types::SteamId::from_parts(1, 1, 1, 0);
    (logon, steam_id.raw())
}

fn build_anon_logon() -> (steamroom::generated::CMsgClientLogon, u64) {
    let logon = steamroom::generated::CMsgClientLogon {
        protocol_version: Some(PROTOCOL_VERSION),
        cell_id: Some(0),
        client_os_type: Some(20),
        ..Default::default()
    };
    let steam_id = steamroom::types::SteamId::from_parts(1, 10, 0, 0);
    (logon, steam_id.raw())
}

async fn run_download(
    args: DownloadArgs,
    auth: &AuthOptions,
    show_progress: bool,
) -> Result<(), CliError> {
    let client = connect_and_login(auth).await?;
    let app_id = AppId(args.app);

    // Get access tokens
    info!("getting PICS access tokens for app {}", app_id);
    let tokens = client.pics_get_access_tokens(&[app_id]).await?;
    let token = tokens
        .into_iter()
        .next()
        .unwrap_or(AccessToken { app_id, token: 0 });

    // Get product info
    info!("getting product info...");
    let infos = client.pics_get_product_info(&[token]).await?;
    let app_info = infos
        .into_iter()
        .next()
        .ok_or_else(|| CliError::NoProductInfo(app_id.0))?;

    // Parse KV data
    let kv_data = app_info
        .kv_data
        .ok_or_else(|| CliError::NoKvData(app_id.0))?;
    let kv = parse_app_kv(&kv_data)?;

    // Find depots
    let depot_id = if let Some(d) = args.depot {
        DepotId(d)
    } else {
        // Find first depot from the KV data
        let depots_kv = kv.get("depots").ok_or_else(|| CliError::NoDepots)?;
        find_first_depot(depots_kv)?
    };

    let branch = args.branch.as_deref().unwrap_or("public");

    // Find manifest ID
    let manifest_id = if let Some(m) = args.manifest {
        ManifestId(m)
    } else {
        let depots_kv = kv.get("depots").ok_or_else(|| CliError::NoDepots)?;
        find_manifest_for_depot(depots_kv, depot_id, branch)?
    };

    info!(
        "depot={}, manifest={}, branch={}",
        depot_id, manifest_id, branch
    );

    // Get depot decryption key
    info!("getting depot key for depot {depot_id} app {app_id}...");
    let depot_key = client.get_depot_decryption_key(depot_id, app_id).await?;
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

    // Get manifest request code
    info!("getting manifest request code...");
    let request_code = client
        .get_manifest_request_code(app_id, depot_id, manifest_id, Some(branch), None)
        .await?
        .unwrap_or(0);

    // Download manifest (with cache)
    let cdn = CdnClient::new().map_err(CliError::Steam)?;
    let manifest_cache = steamroom_client::manifest::ManifestCache::new(
        steamroom_client::manifest::ManifestCache::default_path(),
    );

    let (manifest_bytes, cdn_raw) = if let Some(cached) = manifest_cache.load(depot_id, manifest_id)
    {
        debug!("using cached manifest for {depot_id}_{manifest_id}");
        (cached, None)
    } else {
        info!("downloading manifest...");
        let manifest_data = cdn
            .download_manifest(cdn_server, depot_id, manifest_id, request_code, None)
            .await?;
        let decompressed = decompress_manifest(&manifest_data)?;
        let _ = manifest_cache.save(depot_id, manifest_id, &decompressed);
        (decompressed, Some(manifest_data))
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

    // Load old manifest for delta file removal
    let depot_config = steamroom_client::depot_config::DepotConfig::load(&output_dir);
    let old_manifest_files = match depot_config.get_installed(depot_id) {
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
                Some(
                    old.files
                        .iter()
                        .map(|f| f.filename.clone())
                        .collect::<Vec<_>>(),
                )
            })
        }
        _ => None,
    };

    // Set up download orchestration
    let (event_tx, event_rx) = tokio::sync::mpsc::unbounded_channel();

    let fetcher = steamroom_client::download::CdnChunkFetcher::new(cdn, cdn_pool, None);

    let mut builder = steamroom_client::download::DepotJob::builder()
        .depot_id(depot_id)
        .depot_key(depot_key.clone())
        .install_dir(output_dir.clone())
        .verify(args.verify)
        .event_sender(event_tx);

    if let Some(old_files) = old_manifest_files {
        info!("delta update: will remove files not in new manifest");
        builder = builder.old_manifest_files(old_files);
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

    let job = builder.build().map_err(|e| {
        CliError::Steam(steamroom::error::Error::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            e,
        )))
    })?;

    info!("downloading to {}", output_dir.display());

    let progress_handle = download::spawn_progress_renderer(event_rx, show_progress);

    let stats = job
        .download(&manifest, std::sync::Arc::new(fetcher))
        .await
        .map_err(|e| CliError::Io(std::io::Error::other(e)))?;

    drop(job);
    let _ = progress_handle.await;

    // Save manifest and config for future delta downloads / preservation
    if let Some(raw) = cdn_raw {
        let _ = steamroom_client::depot_config::DepotConfig::save_manifest_raw(
            &output_dir,
            depot_id,
            manifest_id,
            &raw,
        );
    }
    let _ = steamroom_client::depot_config::DepotConfig::save_manifest_decompressed(
        &output_dir,
        depot_id,
        manifest_id,
        &manifest_bytes,
    );
    let mut depot_config = steamroom_client::depot_config::DepotConfig::load(&output_dir);
    depot_config.set_installed(depot_id, manifest_id, &depot_key);
    let _ = depot_config.save(&output_dir);

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
        for key in map.keys() {
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
                    let id: u64 = gid_str.parse().map_err(|_| CliError::InvalidManifestId)?;
                    return Ok(ManifestId(id));
                }
            }
            // Maybe branch_kv itself is a string (manifest ID directly)
            if let Some(gid_str) = branch_kv.as_str() {
                let id: u64 = gid_str.parse().map_err(|_| CliError::InvalidManifestId)?;
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
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                "empty manifest archive",
            )
            .into());
        }
        let mut file = archive
            .by_index(0)
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
    Ok((client, kv))
}

async fn run_info(args: InfoArgs, auth: &AuthOptions) -> Result<(), CliError> {
    let app_id = AppId(args.app);
    let (_client, kv) = fetch_app_kv(auth, app_id).await?;

    if args.format == Some(OutputFormat::Json) {
        println!("{}", serde_json::to_string_pretty(&kv_to_json(&kv))?);
        return Ok(());
    }

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
            let depot_ids: Vec<&String> = map.keys().filter(|k| k.parse::<u32>().is_ok()).collect();
            println!("\nDepots ({}):", depot_ids.len());
            for id in &depot_ids {
                let depot = &map[*id];
                let dname = depot.get("name").and_then(|n| n.as_str()).unwrap_or("");
                let encrypted = depot.get("encryptedmanifests").is_some();
                let enc_tag = if encrypted { " [encrypted]" } else { "" };
                println!("  {id}: {dname}{enc_tag}");
            }

            println!("\nBranches:");
            if let Some(branches) = depots.get("branches") {
                if let KvValue::Children(ref bmap) = branches.value {
                    for (bname, branch) in bmap {
                        let build_id = branch
                            .get("buildid")
                            .and_then(|b| b.as_str())
                            .unwrap_or("-");
                        let time_updated = branch
                            .get("timeupdated")
                            .and_then(|t| t.as_str())
                            .unwrap_or("");
                        let pwd = branch.get("pwdrequired").and_then(|p| p.as_str()) == Some("1");
                        let desc = branch
                            .get("description")
                            .and_then(|d| d.as_str())
                            .unwrap_or("");
                        let mut flags = Vec::new();
                        if pwd {
                            flags.push("password");
                        }
                        if !desc.is_empty() {
                            flags.push(desc);
                        }
                        let extra = if flags.is_empty() {
                            String::new()
                        } else {
                            format!(" ({})", flags.join(", "))
                        };
                        let ts = if time_updated.is_empty() {
                            String::new()
                        } else {
                            format!(" updated {time_updated}")
                        };
                        println!("  {bname}: build {build_id}{ts}{extra}");
                    }
                }
            }
        }
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
                        let dname = depot.get("name").and_then(|n| n.as_str()).unwrap_or("");
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
        .or_else(|| kv.get("depots").and_then(|d| find_first_depot(d).ok()))
        .ok_or(CliError::NoDepots)?;

    let manifest_id = args
        .manifest
        .map(ManifestId)
        .or_else(|| {
            kv.get("depots")
                .and_then(|d| find_manifest_for_depot(d, depot_id, branch).ok())
        })
        .ok_or(CliError::ManifestNotFound {
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
    if manifest.filenames_encrypted && !args.raw {
        manifest.decrypt_filenames(&depot_key)?;
    }

    if args.format == Some(OutputFormat::Json) {
        let entries: Vec<serde_json::Value> = manifest
            .files
            .iter()
            .map(|f| {
                serde_json::json!({
                    "filename": &f.filename,
                    "size": f.size,
                    "flags": f.flags,
                    "chunks": f.chunks.len(),
                })
            })
            .collect();
        println!("{}", serde_json::to_string_pretty(&entries)?);
        return Ok(());
    }

    for file in &manifest.files {
        let name = &file.filename;
        let is_dir = steamroom::enums::DepotFileFlags(file.flags).is_directory();
        if args.format == Some(OutputFormat::Plain) {
            println!("{name}");
        } else if is_dir {
            println!("{name}/");
        } else {
            println!("{name}\t{}", fmt_size(file.size));
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
        KvValue::Float32(v) => serde_json::Number::from_f64(*v as f64)
            .map(serde_json::Value::Number)
            .unwrap_or(serde_json::Value::Null),
        _ => serde_json::Value::Null,
    }
}

async fn run_workshop(
    args: WorkshopArgs,
    auth: &AuthOptions,
    show_progress: bool,
) -> Result<(), CliError> {
    let client = connect_and_login(auth).await?;

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
    let cdn_server = &cdn_servers[0];
    let cdn_pool = steamroom::cdn::CdnServerPool::new(cdn_servers.clone());
    let cdn = CdnClient::new().map_err(CliError::Steam)?;

    let request_code = client
        .get_manifest_request_code(app_id, depot_id, manifest_id, None, None)
        .await?
        .unwrap_or(0);

    let manifest_data = cdn
        .download_manifest(cdn_server, depot_id, manifest_id, request_code, None)
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
        .build()
        .map_err(|e| CliError::Io(std::io::Error::other(e)))?;

    info!("downloading to {}", output_dir.display());

    let progress_handle = download::spawn_progress_renderer(event_rx, show_progress);

    let stats = job
        .download(&manifest, std::sync::Arc::new(fetcher))
        .await
        .map_err(|e| CliError::Io(std::io::Error::other(e)))?;
    drop(job);
    let _ = progress_handle.await;

    info!(
        "workshop download complete: {} files, {}",
        stats.files_completed,
        fmt_size(stats.bytes_downloaded)
    );
    Ok(())
}
