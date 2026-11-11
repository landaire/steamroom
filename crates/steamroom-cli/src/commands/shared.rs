//! Helpers shared by every `run_*` command handler.
//!
//! Most of these used to live in `main.rs`. They were lifted here so the
//! per-command modules (and, eventually, the daemon worker) can share the
//! same KV parsing, formatting, manifest decompression, and login glue.

use crate::cli::AuthOptions;
use crate::cli::FilesArgs;
use crate::errors::CliError;
use std::sync::OnceLock;
use steamroom::apps::AppDetails;
use steamroom::apps::KvDecodeError;
use steamroom::apps::UnlockedBranch;
use steamroom::cdn::CdnClient;
use steamroom::client::LoggedIn;
use steamroom::client::SteamClient;
use steamroom::depot::manifest::DepotManifest;
use steamroom::depot::*;
use steamroom::error::ConnectionError;
use steamroom::types::key_value::KeyValue;
use steamroom::types::key_value::KvValue;
use steamroom_client::credentials::StartupAuth;
use steamroom_client::credentials::TokenStore;
use steamroom_client::login::AuthTokens;
use steamroom_client::login::ConfirmationChallenge;
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

/// Crates whose tracing output is first-party. Everything else is
/// silenced by [`log_filter`] unless `RUST_LOG` opts it back in.
const FIRST_PARTY_CRATES: [&str; 4] = [
    "steamroom",
    "steamroom_client",
    "steamroom_ffi",
    "steamroom_cli",
];

/// Build the tracing filter layer. When `RUST_LOG` is set it is honored
/// verbatim; otherwise logging is restricted to the first-party crates at
/// `level` and every other crate (h2, hyper, reqwest, tokio, ...) is
/// silenced. The default branch is a typed [`Targets`] filter rather than
/// a parsed directive string. Boxed so both branches share one type at
/// the call site.
///
/// [`Targets`]: tracing_subscriber::filter::Targets
pub fn log_filter<S>(
    level: tracing_subscriber::filter::LevelFilter,
) -> Box<dyn tracing_subscriber::Layer<S> + Send + Sync>
where
    S: tracing::Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
{
    use tracing_subscriber::Layer;
    if let Ok(env) = tracing_subscriber::EnvFilter::try_from_default_env() {
        return env.boxed();
    }
    let mut targets = tracing_subscriber::filter::Targets::new();
    for krate in FIRST_PARTY_CRATES {
        targets = targets.with_target(krate, level);
    }
    targets.boxed()
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
        // Check sibling depot.json (manifest might be in .DepotDownloader/manifests/)
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

/// Map a library app lookup failure to a friendlier CLI error, translating
/// the "no KV payload" case into [`CliError::NoProductInfo`].
fn map_app_lookup<T>(res: Result<T, steamroom::Error>, app_id: AppId) -> Result<T, CliError> {
    res.map_err(|e| match e {
        steamroom::Error::Kv(KvDecodeError::Missing) => CliError::NoProductInfo(app_id.0),
        other => CliError::Steam(other),
    })
}

/// Password-protected branches to merge into app details.
#[derive(Debug, Default)]
pub struct BranchUnlocks {
    /// Unlocked by `--branch-password`. Failing to read one is an error.
    pub from_password: Vec<UnlockedBranch>,
    /// Cached by the local Steam client. Steam may have removed or re-keyed
    /// these since, so a denied one is skipped with a warning.
    pub cached: Vec<UnlockedBranch>,
}

impl BranchUnlocks {
    /// Keep only `branch`. A password that unlocks other branches but not
    /// this one is an error rather than a silent fall back to public data.
    pub fn only(mut self, branch: &str) -> Result<Self, CliError> {
        if !self.from_password.is_empty() && !self.from_password.iter().any(|b| b.name == branch) {
            return Err(CliError::BranchNotUnlocked {
                branch: branch.to_string(),
                unlocked: self.from_password.into_iter().map(|b| b.name).collect(),
            });
        }

        self.from_password.retain(|b| b.name == branch);
        self.cached.retain(|b| b.name == branch);

        Ok(self)
    }
}

/// Collect the branches `password` unlocks and, when `local_keys` is set,
/// the branches whose passwords the local Steam client has cached.
pub async fn unlock_branches(
    client: &SteamClient<LoggedIn>,
    app_id: AppId,
    password: Option<&str>,
    local_keys: bool,
) -> Result<BranchUnlocks, CliError> {
    let from_password = match password {
        Some(password) => client.check_app_beta_password(app_id, password).await?,
        None => Vec::new(),
    };

    let cached = if local_keys {
        let steam_dir =
            steamroom_client::steam_creds::steam_dir().ok_or(CliError::SteamNotFound)?;
        let config = steamroom_client::steam_creds::read_config(&steam_dir)
            .ok_or(CliError::SteamNotFound)?;
        steamroom_client::steam_creds::unlocked_branches(&config, app_id)
            .into_iter()
            .filter(|cached| !from_password.iter().any(|b| b.name == cached.name))
            .collect()
    } else {
        Vec::new()
    };

    Ok(BranchUnlocks {
        from_password,
        cached,
    })
}

/// Look up an app's structured [`AppDetails`] via PICS, with the depot
/// sections of unlocked password-protected branches merged in.
pub async fn fetch_app_details(
    client: &SteamClient<LoggedIn>,
    app_id: AppId,
    unlocks: &BranchUnlocks,
) -> Result<AppDetails, CliError> {
    let mut details = map_app_lookup(
        client
            .app_details_with_branches(app_id, &unlocks.from_password)
            .await,
        app_id,
    )?;

    // A cached password is only needed while Steam still hides the branch.
    let pending: Vec<&UnlockedBranch> = unlocks
        .cached
        .iter()
        .filter(|cached| details.branch(&cached.name).is_none_or(|b| b.pwd_required))
        .collect();
    if pending.is_empty() {
        return Ok(details);
    }

    let token = client.app_access_token(app_id).await?;
    for branch in pending {
        match client.private_depots(&token, branch).await {
            Ok(section) => details = details.with_private_depots(section),
            Err(steamroom::Error::Connection(ConnectionError::PrivateBranchDenied {
                result,
                ..
            })) => {
                warn!(
                    "skipping cached password for branch \"{}\": Steam denied it ({result:?})",
                    branch.name
                );
            }
            Err(e) => return Err(e.into()),
        }
    }

    Ok(details)
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
    if cdn_servers.is_empty() {
        return Err(CliError::NoCdnServers);
    }
    let cdn_pool = steamroom::cdn::CdnServerPool::new(cdn_servers);
    let cdn = CdnClient::new().map_err(CliError::Steam)?;
    let manifest_data = cdn
        .download_manifest_pooled(&cdn_pool, depot_id, manifest_id, request_code, None)
        .await?;
    let manifest_bytes = decompress_manifest(&manifest_data)?;
    let mut manifest = DepotManifest::parse(&manifest_bytes)?;
    if manifest.filenames_encrypted {
        let _ = manifest.decrypt_filenames(&depot_key);
    }
    Ok(manifest)
}

pub async fn connect_and_login(
    auth: &AuthOptions,
    recorder: Option<&steamroom::transport::recording::Recorder>,
) -> Result<SteamClient<LoggedIn>, CliError> {
    let make_builder = || {
        let b = LoginBuilder::new().device_name(auth.device_name.as_deref().unwrap_or("steamroom"));
        match recorder {
            Some(r) => b.record(r.clone()),
            None => b,
        }
    };
    let builder = make_builder();

    if auth.use_steam_token {
        let (client, _) = login_with_steam_token(builder, auth.username.as_deref()).await?;
        return Ok(client);
    }

    // -u/--username given. --qr forces a fresh QR session; otherwise try a
    // saved refresh token (with fallback to interactive auth if it's stale),
    // then password.
    if let Some(ref username) = auth.username {
        // Try the saved refresh token first regardless of `--qr` /
        // password. The flags describe how to RECOVER if the token is
        // missing or stale; a valid token is always the cheap path.
        if let Some(token) = load_saved_token(username)? {
            info!("using saved refresh token for {username}");
            let attempt = make_builder()
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
                    forget_saved_token(username)?;
                }
                Err(e) => return Err(e.into()),
            }
        }
        // No token (or it was rejected). Fall through to the requested
        // interactive flow.
        if auth.qr {
            if !is_interactive() {
                return Err(CliError::InteractiveAuthRequired);
            }
            return drive_qr_flow(builder, username, auth.remember_password).await;
        }
        if !is_interactive() && auth.password.is_none() {
            return Err(CliError::InteractiveAuthRequired);
        }
        return drive_credentials_flow(builder, username, auth).await;
    }

    // Auto-detect Steam user with a saved token.
    if let Some((username, token)) = detect_steam_user()? {
        info!("auto-detected Steam user: {username}");
        return Ok(builder.with_refresh_token(username, token).login().await?);
    }

    // Last resort: anonymous.
    Ok(builder.anonymous().login().await?)
}

pub fn tokens_path() -> std::path::PathBuf {
    TokenStore::default_path()
}

pub fn startup_auth() -> Result<Option<StartupAuth>, CliError> {
    let store = TokenStore::load_or_default(&tokens_path())?;
    Ok(store.startup_auth)
}

pub fn set_startup_auth(startup_auth: Option<StartupAuth>) -> Result<(), CliError> {
    let path = tokens_path();
    let mut store = TokenStore::load_or_default(&path)?;
    store.startup_auth = startup_auth;
    store.save(&path)?;
    Ok(())
}

pub fn apply_startup_auth(auth: &mut AuthOptions) -> Result<(), CliError> {
    if auth.has_explicit_method() {
        return Ok(());
    }

    let Some(StartupAuth::SteamToken { account_name }) = startup_auth()? else {
        return Ok(());
    };

    auth.use_steam_token = true;
    auth.username = account_name;
    Ok(())
}

/// Try to detect the active Steam user and find a saved refresh token.
pub fn detect_steam_user() -> Result<Option<(String, String)>, CliError> {
    let Some(dir) = steamroom_client::steam_creds::steam_dir() else {
        return Ok(None);
    };
    let Some(username) = steamroom_client::steam_creds::detect_username(&dir) else {
        return Ok(None);
    };
    let Some(token) = load_saved_token(&username)? else {
        return Ok(None);
    };

    Ok(Some((username, token)))
}

pub fn load_saved_token(username: &str) -> Result<Option<String>, CliError> {
    let store = TokenStore::load_or_default(&tokens_path())?;
    Ok(store.get(username).map(ToOwned::to_owned))
}

pub fn save_token(username: &str, refresh_token: &str) -> Result<(), CliError> {
    let path = tokens_path();
    let mut store = TokenStore::load_or_default(&path)?;
    store.set(username.to_string(), refresh_token.to_string());
    store.save(&path)?;
    info!("saved refresh token for {username}");
    Ok(())
}

pub fn forget_saved_token(username: &str) -> Result<(), CliError> {
    let path = tokens_path();
    let mut store = match TokenStore::load(&path) {
        Ok(store) => store,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e.into()),
    };
    store.remove(username);
    store.save(&path)?;
    Ok(())
}

pub async fn login_with_steam_token(
    builder: LoginBuilder,
    username: Option<&str>,
) -> Result<(SteamClient<LoggedIn>, String), CliError> {
    let username = match username {
        Some(username) => username.to_string(),
        None => {
            let directory =
                steamroom_client::steam_creds::steam_dir().ok_or(CliError::SteamTokenUnavailable)?;
            steamroom_client::steam_creds::detect_username(&directory)
                .ok_or(CliError::SteamTokenUnavailable)?
        }
    };
    info!("extracting cached Steam token for {username}...");
    let credential = steamroom_client::steam_creds::extract_token(&username)
        .ok_or(CliError::SteamTokenUnavailable)?;
    let account_name = credential.account_name;
    info!("using cached Steam token for {account_name}");
    let client = builder
        .with_refresh_token(account_name.clone(), credential.refresh_token)
        .login()
        .await?;

    Ok((client, account_name))
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
                (Some(password), _) => password,
                (None, true) => rpassword::prompt_password(format!("Password for {username}: "))?,
                (None, false) => return Err(CliError::InteractiveAuthRequired),
            }
        } else if !is_interactive() {
            return Err(CliError::InteractiveAuthRequired);
        } else {
            eprintln!("Invalid password, try again ({}/3)", attempt + 1);
            rpassword::prompt_password(format!("Password for {username}: "))?
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
            CredentialsLoginFlow::NeedsConfirmation(challenge) => {
                if !is_interactive() {
                    return Err(CliError::InteractiveAuthRequired);
                }
                drive_confirmation(challenge).await?
            }
            // CredentialsLoginFlow is #[non_exhaustive]; a variant added by a
            // future library version is unsupported by this build rather than a
            // panic.
            _ => return Err(CliError::Login(LoginError::NoSupportedConfirmation)),
        };

        let tokens = approved.tokens();
        let account_name = tokens.account_name.as_deref().unwrap_or(username).to_string();
        let refresh_token = tokens.refresh_token.clone();
        let client = approved.finish().await?;
        if auth.remember_password {
            save_token(&account_name, &refresh_token)?;
        }
        return Ok(client);
    }
    // Three attempts exhausted.
    Err(CliError::Login(LoginError::InvalidPassword))
}

/// Complete a 2FA challenge. When Steam accepts both a code and an out-of-band
/// confirmation, the two are driven concurrently so the user may either type a
/// code or approve on their phone, whichever they do first.
async fn drive_confirmation(
    challenge: ConfirmationChallenge,
) -> Result<steamroom_client::login::ApprovedAuth, CliError> {
    match (challenge.accepts_code(), challenge.accepts_confirmation()) {
        (true, true) => {
            eprintln!(
                "Approve the sign-in in your Steam mobile app, or type your Steam Guard code below."
            );
            let kind = preferred_kind(challenge.code_kinds());
            let tokens = race_code_or_confirm(&challenge, kind).await?;
            Ok(challenge.into_approved(tokens))
        }
        (true, false) => {
            let kind = preferred_kind(challenge.code_kinds());
            loop {
                let prompt = guard_prompt(challenge.code_kinds());
                let code = rpassword::prompt_password(prompt).unwrap_or_default();
                match challenge.submit_code(code.trim(), kind).await {
                    Ok(()) => break,
                    Err(LoginError::InvalidGuardCode) => {
                        eprintln!("Invalid Steam Guard code, try again.")
                    }
                    Err(e) => return Err(e.into()),
                }
            }
            let tokens = challenge.wait_for_tokens().await?;
            Ok(challenge.into_approved(tokens))
        }
        (false, true) => {
            info!("confirm the sign-in in your Steam mobile app...");
            Ok(challenge.wait_for_confirmation().await?)
        }
        // begin() rejects a challenge with no supported method before we get here.
        (false, false) => Err(CliError::Login(LoginError::NoSupportedConfirmation)),
    }
}

/// Poll for out-of-band confirmation while concurrently reading a Steam Guard
/// code from stdin. Returns as soon as either path yields tokens. Codes are
/// read in plaintext (they are not secret like a password) so the read stays
/// async and cancellable, letting a mobile approval win the race.
async fn race_code_or_confirm(
    challenge: &ConfirmationChallenge,
    kind: GuardType,
) -> Result<AuthTokens, CliError> {
    use std::io::Write;
    use tokio::io::AsyncBufReadExt;

    let prompt = guard_prompt(challenge.code_kinds());
    let reprompt = || {
        eprint!("{prompt}");
        let _ = std::io::stderr().flush();
    };

    let mut lines = tokio::io::BufReader::new(tokio::io::stdin()).lines();
    reprompt();

    // One long-lived confirmation poll, kept alive across code attempts so its
    // interval timer is never reset by keystrokes (which would starve the
    // mobile-approval path). `biased` checks it first each iteration so a ready
    // token is never dropped in favor of a simultaneously-ready stdin line. A
    // submitted-and-accepted code surfaces through this same poll.
    let poll = challenge.wait_for_tokens();
    tokio::pin!(poll);

    loop {
        tokio::select! {
            biased;
            res = &mut poll => return res.map_err(Into::into),

            line = lines.next_line() => {
                let code = match line {
                    Ok(Some(l)) => l.trim().to_string(),
                    // stdin closed or errored: wait out the confirmation poll.
                    Ok(None) | Err(_) => return (&mut poll).await.map_err(Into::into),
                };
                if code.is_empty() {
                    reprompt();
                    continue;
                }
                match challenge.submit_code(&code, kind).await {
                    // Accepted: the live poll picks up the tokens on its next tick.
                    Ok(()) => {}
                    Err(LoginError::InvalidGuardCode) => {
                        eprintln!("Invalid Steam Guard code, try again (or approve on your phone).");
                        reprompt();
                    }
                    Err(e) => return Err(e.into()),
                }
            }
        }
    }
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
    remember_password: bool,
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
    let account_name = tokens.account_name.as_deref().unwrap_or(username).to_string();
    let refresh_token = tokens.refresh_token.clone();
    let client = approved.finish().await?;
    if remember_password {
        save_token(&account_name, &refresh_token)?;
    }
    Ok(client)
}
