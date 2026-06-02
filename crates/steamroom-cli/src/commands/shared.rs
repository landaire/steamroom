//! Helpers shared by every `run_*` command handler.
//!
//! Most of these used to live in `main.rs`. They were lifted here so the
//! per-command modules (and, eventually, the daemon worker) can share the
//! same KV parsing, formatting, manifest decompression, and login glue.

use crate::cli::AuthOptions;
use crate::cli::FilesArgs;
use crate::errors::CliError;
use std::sync::OnceLock;
use steamroom::apps::AccessToken;
use steamroom::cdn::CdnClient;
use steamroom::client::LoggedIn;
use steamroom::client::SteamClient;
use steamroom::depot::manifest::DepotManifest;
use steamroom::depot::*;
use steamroom::types::key_value;
use steamroom::types::key_value::KeyValue;
use steamroom::types::key_value::KvValue;
use steamroom_client::login::CredentialsLoginFlow;
use steamroom_client::login::GuardType;
use steamroom_client::login::LoginBuilder;
use steamroom_client::login::LoginError;
use tracing::info;
use tracing::warn;

/// Set once in `main`: true iff the user did not pass `--non-interactive`
/// and stdin is a TTY. Read via [`is_interactive`].
pub static INTERACTIVE: OnceLock<bool> = OnceLock::new();

/// Initialize the interactive flag. Safe to call once from `main`.
pub fn init_interactive(v: bool) {
    let _ = INTERACTIVE.set(v);
}

pub fn is_interactive() -> bool {
    INTERACTIVE.get().copied().unwrap_or(false)
}

pub fn parse_app_kv(data: &[u8]) -> Result<KeyValue, CliError> {
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

pub fn parse_package_kv(data: &[u8]) -> Result<KeyValue, CliError> {
    // Package PICS data has a 4-byte header before the binary KV blob.
    let kv_data = if data.len() > 4 && data[0] != 0x00 {
        &data[4..]
    } else {
        data
    };
    parse_app_kv(kv_data)
}

pub fn kv_to_json(kv: &KeyValue) -> serde_json::Value {
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

pub fn find_first_depot(depots_kv: &KeyValue) -> Result<DepotId, CliError> {
    if let KvValue::Children(ref map) = depots_kv.value {
        for key in map.keys() {
            if let Ok(id) = key.parse::<u32>()
                && id > 0
            {
                return Ok(DepotId(id));
            }
        }
    }
    Err(CliError::NoDepots)
}

pub fn find_manifest_for_depot(
    depots_kv: &KeyValue,
    depot_id: DepotId,
    branch: &str,
) -> Result<ManifestId, CliError> {
    let depot_key = depot_id.0.to_string();
    let depot = depots_kv
        .get(&depot_key)
        .ok_or(CliError::DepotNotFound(depot_id.0))?;

    // Look in depots -> {depot_id} -> manifests -> {branch} -> gid
    if let Some(manifests) = depot.get("manifests")
        && let Some(branch_kv) = manifests.get(branch)
    {
        if let Some(gid) = branch_kv.get("gid")
            && let Some(gid_str) = gid.as_str()
        {
            let id: u64 = gid_str.parse().map_err(|_| CliError::InvalidManifestId)?;
            return Ok(ManifestId(id));
        }
        // Maybe branch_kv itself is a string (manifest ID directly)
        if let Some(gid_str) = branch_kv.as_str() {
            let id: u64 = gid_str.parse().map_err(|_| CliError::InvalidManifestId)?;
            return Ok(ManifestId(id));
        }
    }

    Err(CliError::ManifestNotFound {
        depot: depot_id.0,
        branch: branch.to_string(),
    })
}

pub fn resolve_depot_key(args: &FilesArgs) -> Result<DepotKey, CliError> {
    if let Some(ref hex) = args.depot_key {
        let bytes: Vec<u8> = (0..hex.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&hex[i..i + 2], 16))
            .collect::<Result<_, _>>()
            .map_err(|_| {
                CliError::Io(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "invalid hex in --depot-key",
                ))
            })?;
        if bytes.len() != 32 {
            return Err(CliError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("depot key must be 32 bytes, got {}", bytes.len()),
            )));
        }
        let mut key = [0u8; 32];
        key.copy_from_slice(&bytes);
        return Ok(DepotKey(key));
    }
    // Try auto-detect from depot.json next to the manifest file
    if let Some(ref manifest_path) = args.manifest_file
        && let Some(parent) = manifest_path.parent()
    {
        // Check sibling depot.json (manifest might be in .depotdownloader/manifests/)
        for dir in [parent, &parent.join("../.."), &parent.join("..")] {
            let config = steamroom_client::depot_config::DepotConfig::load(dir);
            if let Some(depot_id) = args.depot
                && let Some((_, key)) = config.get_installed(DepotId(depot_id))
            {
                return Ok(key);
            }
            // Try any depot in the config
            for info in config.depots.values() {
                let bytes: Vec<u8> = (0..info.depot_key.len())
                    .step_by(2)
                    .filter_map(|i| u8::from_str_radix(&info.depot_key[i..i + 2], 16).ok())
                    .collect();
                if bytes.len() == 32 {
                    let mut key = [0u8; 32];
                    key.copy_from_slice(&bytes);
                    return Ok(DepotKey(key));
                }
            }
        }
    }
    Err(CliError::Io(std::io::Error::new(
        std::io::ErrorKind::NotFound,
        "no depot key available (pass --depot-key <hex> or --raw for encrypted names)",
    )))
}

pub fn decompress_manifest(data: &[u8]) -> Result<Vec<u8>, CliError> {
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

pub fn fmt_size(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{} B", bytes)
    } else if bytes < 1024 * 1024 {
        format!("{:.2} KiB", bytes as f64 / 1024.0)
    } else if bytes < 1024 * 1024 * 1024 {
        format!("{:.2} MiB", bytes as f64 / (1024.0 * 1024.0))
    } else {
        format!("{:.2} GiB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    }
}

pub fn fmt_timestamp(epoch: u64) -> String {
    jiff::Timestamp::from_second(epoch as i64)
        .map(|ts| ts.strftime("%Y-%m-%d %H:%M:%S UTC").to_string())
        .unwrap_or_else(|_| epoch.to_string())
}

pub fn fmt_relative(epoch: u64) -> String {
    let Ok(ts) = jiff::Timestamp::from_second(epoch as i64) else {
        return epoch.to_string();
    };
    let now = jiff::Timestamp::now();
    let span = now.duration_since(ts);
    let hours = span.as_hours();
    if hours < 1 {
        "just now".to_string()
    } else if hours < 24 {
        format!("{hours}h ago")
    } else {
        let days = hours / 24;
        if days >= 365 {
            let years = days / 365;
            let rem_months = (days % 365) / 30;
            if rem_months > 0 {
                format!("{years}y {rem_months}mo ago")
            } else {
                format!("{years}y ago")
            }
        } else if days >= 30 {
            let months = days / 30;
            let rem_days = days % 30;
            if rem_days > 0 {
                format!("{months}mo {rem_days}d ago")
            } else {
                format!("{months}mo ago")
            }
        } else {
            format!("{days}d ago")
        }
    }
}

/// Look up an app's KV data via PICS using an already-authenticated client.
///
/// Previously this helper also built the client (took an `AuthOptions`);
/// the refactor moved the connect/login step up to `async_main` so it can
/// be reused across daemon requests, leaving this helper as a pure
/// PICS-fetch + parse.
pub async fn fetch_app_kv(
    client: &SteamClient<LoggedIn>,
    app_id: AppId,
) -> Result<KeyValue, CliError> {
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
    Ok(kv)
}

pub async fn fetch_manifest(
    client: &SteamClient<LoggedIn>,
    app_id: AppId,
    depot_id: DepotId,
    manifest_id: ManifestId,
    branch: Option<&str>,
) -> Result<DepotManifest, CliError> {
    let depot_key = client.get_depot_decryption_key(depot_id, app_id).await?;
    let request_code = client
        .get_manifest_request_code(app_id, depot_id, manifest_id, branch, None)
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
        let _ = manifest.decrypt_filenames(&depot_key);
    }
    Ok(manifest)
}

pub async fn connect_and_login(auth: &AuthOptions) -> Result<SteamClient<LoggedIn>, CliError> {
    let builder =
        LoginBuilder::new().device_name(auth.device_name.as_deref().unwrap_or("steamroom"));

    // --use-steam-token: prefer local Steam install's cached token.
    if auth.use_steam_token {
        let username = auth.username.clone().or_else(|| {
            let dir = steamroom_client::steam_creds::steam_dir()?;
            steamroom_client::steam_creds::detect_username(&dir)
        });
        let cached = username.as_deref().and_then(|u| {
            info!("extracting cached Steam token for {u}...");
            steamroom_client::steam_creds::extract_token(u)
        });
        if let Some(cred) = cached {
            info!("using cached Steam token for {}", cred.account_name);
            return Ok(builder
                .with_refresh_token(cred.account_name, cred.refresh_token)
                .login()
                .await?);
        }
        warn!("failed to extract Steam token, falling back to normal auth");
        if let Some(u) = username
            && let Some(token) = load_saved_token(&u)
        {
            info!("using saved refresh token for {u}");
            return Ok(builder.with_refresh_token(u, token).login().await?);
        }
        return Ok(builder.anonymous().login().await?);
    }

    // -u/--username given. --qr forces a fresh QR session; otherwise try a
    // saved refresh token (with fallback to interactive auth if it's stale),
    // then password.
    if let Some(ref username) = auth.username {
        if auth.qr {
            if !is_interactive() {
                return Err(CliError::InteractiveAuthRequired);
            }
            return drive_qr_flow(builder, username).await;
        }
        if let Some(token) = load_saved_token(username) {
            info!("using saved refresh token for {username}");
            let attempt = LoginBuilder::new()
                .device_name(auth.device_name.as_deref().unwrap_or("steamroom"))
                .with_refresh_token(username, token)
                .login()
                .await;
            match attempt {
                Ok(client) => return Ok(client),
                Err(LoginError::LogonFailed(
                    steamroom::enums::EResultError::InvalidPassword
                    | steamroom::enums::EResultError::AccessDenied
                    | steamroom::enums::EResultError::Expired,
                ))
                | Err(LoginError::InvalidPassword) => {
                    warn!("saved refresh token rejected; re-authenticating");
                    forget_saved_token(username);
                }
                Err(e) => return Err(e.into()),
            }
        }
        if !is_interactive() && auth.password.is_none() {
            return Err(CliError::InteractiveAuthRequired);
        }
        return drive_credentials_flow(builder, username, auth).await;
    }

    // Auto-detect Steam user with a saved token.
    if let Some((username, token)) = detect_steam_user() {
        info!("auto-detected Steam user: {username}");
        return Ok(builder.with_refresh_token(username, token).login().await?);
    }

    // Last resort: anonymous.
    Ok(builder.anonymous().login().await?)
}

pub fn tokens_path() -> Option<std::path::PathBuf> {
    Some(
        dirs_next::home_dir()?
            .join(".depotdownloader")
            .join("tokens.json"),
    )
}

/// Try to detect the active Steam user and find a saved refresh token.
pub fn detect_steam_user() -> Option<(String, String)> {
    let dir = steamroom_client::steam_creds::steam_dir()?;
    let username = steamroom_client::steam_creds::detect_username(&dir)?;
    let token = load_saved_token(&username)?;
    Some((username, token))
}

pub fn load_saved_token(username: &str) -> Option<String> {
    let data = std::fs::read_to_string(tokens_path()?).ok()?;
    let parsed: serde_json::Value = serde_json::from_str(&data).ok()?;
    parsed["tokens"][username].as_str().map(|s| s.to_string())
}

pub fn save_token(username: &str, refresh_token: &str) {
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

pub fn forget_saved_token(username: &str) {
    let Some(path) = tokens_path() else { return };
    let Ok(data) = std::fs::read_to_string(&path) else {
        return;
    };
    let Ok(mut root) = serde_json::from_str::<serde_json::Value>(&data) else {
        return;
    };
    if let Some(tokens) = root.get_mut("tokens").and_then(|v| v.as_object_mut()) {
        tokens.remove(username);
    }
    let _ = std::fs::write(
        &path,
        serde_json::to_string_pretty(&root).unwrap_or_default(),
    );
}

pub async fn drive_credentials_flow(
    builder: LoginBuilder,
    username: &str,
    auth: &AuthOptions,
) -> Result<SteamClient<LoggedIn>, CliError> {
    let _ = builder; // Dropped here, the loop creates a fresh LoginBuilder per attempt (each attempt reconnects).
    for attempt in 0..3u32 {
        let password = if attempt == 0 {
            match (auth.password.clone(), is_interactive()) {
                (Some(p), _) => p,
                (None, true) => rpassword::prompt_password(format!("Password for {username}: "))
                    .unwrap_or_default(),
                (None, false) => return Err(CliError::InteractiveAuthRequired),
            }
        } else if !is_interactive() {
            return Err(CliError::InteractiveAuthRequired);
        } else {
            eprintln!("Invalid password, try again ({}/3)", attempt + 1);
            rpassword::prompt_password(format!("Password for {username}: ")).unwrap_or_default()
        };

        let credentials = LoginBuilder::new()
            .device_name(auth.device_name.as_deref().unwrap_or("steamroom"))
            .with_credentials(username, password);
        let flow = match credentials.begin().await {
            Ok(f) => f,
            Err(LoginError::InvalidPassword) => continue,
            Err(e) => return Err(e.into()),
        };

        let approved = match flow {
            CredentialsLoginFlow::Approved(a) => a,
            CredentialsLoginFlow::NeedsGuardCode(mut challenge) => {
                if !is_interactive() {
                    return Err(CliError::InteractiveAuthRequired);
                }
                loop {
                    let prompt = guard_prompt(challenge.allowed_kinds());
                    let kind = preferred_kind(challenge.allowed_kinds());
                    let code = rpassword::prompt_password(prompt).unwrap_or_default();
                    match challenge.submit_code(&code, kind).await {
                        Ok(a) => break a,
                        Err((c, LoginError::InvalidGuardCode)) => {
                            eprintln!("Invalid Steam Guard code, try again.");
                            challenge = c;
                        }
                        Err((_, e)) => return Err(e.into()),
                    }
                }
            }
            CredentialsLoginFlow::NeedsMobileConfirm(mobile) => {
                if !is_interactive() {
                    return Err(CliError::InteractiveAuthRequired);
                }
                info!("confirm login on your Steam mobile app...");
                mobile.wait_for_confirmation().await?
            }
            _ => unreachable!("unexpected CredentialsLoginFlow variant"),
        };

        let tokens = approved.tokens();
        save_token(
            tokens.account_name.as_deref().unwrap_or(username),
            &tokens.refresh_token,
        );
        return Ok(approved.finish().await?);
    }
    // Three attempts exhausted.
    Err(CliError::Login(LoginError::InvalidPassword))
}

pub fn guard_prompt(kinds: &[GuardType]) -> &'static str {
    if kinds.contains(&GuardType::DeviceCode) {
        "Steam Guard code (from authenticator app): "
    } else if kinds.contains(&GuardType::EmailCode) {
        "Steam Guard code (from email): "
    } else {
        "Steam Guard code: "
    }
}

pub fn preferred_kind(kinds: &[GuardType]) -> GuardType {
    if kinds.contains(&GuardType::DeviceCode) {
        GuardType::DeviceCode
    } else if kinds.contains(&GuardType::EmailCode) {
        GuardType::EmailCode
    } else {
        kinds.first().copied().unwrap_or(GuardType::DeviceCode)
    }
}

pub async fn drive_qr_flow(
    builder: LoginBuilder,
    username: &str,
) -> Result<SteamClient<LoggedIn>, CliError> {
    info!("generating QR code...");
    let flow = builder.with_qr().begin().await?;

    let url = flow.challenge_url();
    let qr =
        qrcode::QrCode::new(url.as_bytes()).map_err(|e| CliError::Io(std::io::Error::other(e)))?;
    let rendered = qr.render::<qrcode::render::unicode::Dense1x2>().build();
    eprintln!("{rendered}");
    eprintln!("Scan this QR code with the Steam mobile app");
    eprintln!("Or open: {url}");

    let approved = flow.wait_for_scan().await?;
    let tokens = approved.tokens();
    save_token(
        tokens.account_name.as_deref().unwrap_or(username),
        &tokens.refresh_token,
    );
    Ok(approved.finish().await?)
}
