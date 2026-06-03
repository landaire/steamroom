#[derive(Debug, thiserror::Error)]
pub enum CliError {
    #[error("{}", display_steam_error(.0))]
    Steam(#[from] steamroom::error::Error),

    #[error("{0}")]
    Io(#[from] std::io::Error),

    #[error("cryptography error: {0}")]
    Crypto(#[from] steamroom::error::CryptoError),

    #[error("internal task error: {0}")]
    Join(#[from] tokio::task::JoinError),

    #[error("{}", display_manifest_error(.0))]
    Manifest(#[from] steamroom::error::ManifestError),

    #[error("chunk processing failed: {0}")]
    Chunk(#[from] steamroom::depot::chunk::ChunkError),

    #[error("failed to decode server response: {0}")]
    Protobuf(#[from] prost::DecodeError),

    #[error("invalid regex pattern: {0}")]
    Regex(#[from] regex::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("{}", display_http_error(.0))]
    Http(#[from] reqwest::Error),

    #[error("failed to parse KeyValue data: {0}")]
    Kv(#[from] steamroom::types::key_value::TextKvError),

    #[error("Steam returned no product info for app {0} (does the app exist?)")]
    NoProductInfo(u32),

    #[error("app {0} has no metadata")]
    NoKvData(u32),

    #[error("no depots found in app info")]
    NoDepots,

    #[error("depot {0} was not found in the app info")]
    DepotNotFound(u32),

    #[error("no manifest found for depot {depot} on branch \"{branch}\"")]
    ManifestNotFound { depot: u32, branch: String },

    #[error("the manifest ID is not a valid number")]
    InvalidManifestId,

    #[error("no cached decryption key for depot {0} in config.vdf")]
    NoLocalKey(u32),

    #[error("Steam installation not found")]
    SteamNotFound,

    #[error("{}", display_login_error(.0))]
    Login(#[from] steamroom_client::login::LoginError),

    #[error(
        "authentication requires an interactive terminal. Re-run on a TTY, \
         supply --password / STEAM_PASS for credentials login, or use a \
         valid saved refresh token."
    )]
    InteractiveAuthRequired,

    #[error("Steam returned no CDN servers")]
    NoCdnServers,

    #[error(
        "--use-daemon: {0} are not supported via the daemon; pass them to --daemon at launch instead"
    )]
    DaemonRejectedFlag(&'static str),

    #[error("--priority is only valid with --use-daemon")]
    PriorityWithoutDaemon,

    #[error("--detach is only valid with --use-daemon")]
    DetachWithoutDaemon,

    #[error("--daemon and --use-daemon are mutually exclusive")]
    DaemonModeConflict,

    #[error(
        "daemon RPC: incompatible wire-protocol version (peer={peer}, ours={ours}); restart the daemon"
    )]
    ProtocolVersionMismatch { peer: u16, ours: u16 },

    #[error("daemon RPC: frame exceeds {limit_bytes} byte cap (got {len_bytes})")]
    FrameTooLarge { len_bytes: u64, limit_bytes: u64 },

    #[error("daemon RPC: malformed frame: {0}")]
    MalformedFrame(String),

    #[error("daemon RPC: socket closed before frame complete")]
    SocketClosed,

    #[error("operation cancelled")]
    Cancelled,

    #[error("a steamroom daemon is already running on this socket")]
    DaemonAlreadyRunning,

    #[error("no daemon running on this socket; start one with `steamroom daemon start`")]
    NoDaemonRunning,

    #[error("daemon returned error: {0}")]
    DaemonError(String),
}

impl From<steamroom::error::ConnectionError> for CliError {
    fn from(e: steamroom::error::ConnectionError) -> Self {
        Self::Steam(steamroom::error::Error::Connection(e))
    }
}

fn display_login_error(e: &steamroom_client::login::LoginError) -> String {
    use steamroom_client::login::LoginError;
    match e {
        LoginError::InvalidPassword => "invalid password".into(),
        LoginError::InvalidGuardCode => "two-factor code rejected".into(),
        LoginError::LogonFailed(r) => format!("login failed: {}", eresult_message(r)),
        LoginError::Transport(inner) => display_steam_error(inner),
        LoginError::MissingField(f) => format!("Steam response missing field: {f}"),
        LoginError::NoCmServers => "could not find any Steam CM servers to connect to".into(),
        _ => e.to_string(),
    }
}

fn display_steam_error(e: &steamroom::error::Error) -> String {
    use steamroom::error::ConnectionError;
    use steamroom::error::Error;

    match e {
        Error::Connection(ConnectionError::LogonFailed(r)) => {
            format!("login failed: {}", eresult_message(r))
        }
        Error::Connection(ConnectionError::ServiceMethodFailed(r)) => {
            format!("Steam API call failed: {}", eresult_message(r))
        }
        Error::Connection(ConnectionError::DepotAccessDenied(depot)) => {
            format!("access denied for depot {depot} (do you own this app? try logging in with -u)")
        }
        Error::Connection(ConnectionError::Disconnected) => "disconnected from Steam".into(),
        Error::Connection(ConnectionError::DnsResolutionFailed) => {
            "DNS resolution failed (check your network connection)".into()
        }
        Error::Connection(ConnectionError::EncryptionFailed) => {
            "failed to establish encrypted connection to Steam".into()
        }
        Error::Connection(ConnectionError::MissingField(field)) => {
            format!("Steam response is missing required field: {field}")
        }
        Error::CdnStatus { status, .. } => {
            use reqwest::StatusCode;
            let code = status.as_u16();
            if *status == StatusCode::UNAUTHORIZED || *status == StatusCode::FORBIDDEN {
                format!("CDN access denied (HTTP {code})")
            } else if *status == StatusCode::NOT_FOUND {
                "content not found on CDN (HTTP 404)".into()
            } else if *status == StatusCode::TOO_MANY_REQUESTS {
                "rate limited by CDN (HTTP 429), retries exhausted".into()
            } else if status.is_server_error() {
                format!("CDN server error (HTTP {code})")
            } else {
                format!("CDN returned HTTP {code}")
            }
        }
        other => other.to_string(),
    }
}

fn display_manifest_error(e: &steamroom::error::ManifestError) -> String {
    use steamroom::error::ManifestError;
    match e {
        ManifestError::MissingSection => "manifest is missing required data sections".into(),
        ManifestError::ChecksumMismatch { .. } => {
            "manifest checksum mismatch (corrupt download?)".into()
        }
        ManifestError::DecryptFailed(_) => {
            "failed to decrypt manifest filenames (wrong depot key?)".into()
        }
        other => format!("manifest error: {other}"),
    }
}

fn display_http_error(e: &reqwest::Error) -> String {
    if e.is_connect() {
        format!("connection failed (check your network): {e}")
    } else if e.is_timeout() {
        format!("request timed out: {e}")
    } else {
        format!("HTTP error: {e}")
    }
}

fn eresult_message(r: &steamroom::enums::EResultError) -> &'static str {
    use steamroom::enums::EResultError;
    match r {
        EResultError::InvalidPassword => "invalid password",
        EResultError::AccessDenied => "access denied",
        EResultError::Banned => "account is banned",
        EResultError::AccountNotFound => "account not found",
        EResultError::InvalidSteamID => "invalid Steam ID",
        EResultError::ServiceUnavailable => "Steam service is temporarily unavailable",
        EResultError::Timeout => "request timed out",
        EResultError::LimitExceeded => "rate limit exceeded, try again later",
        EResultError::Expired => "session expired, please log in again",
        EResultError::InsufficientPrivilege => "insufficient privileges",
        EResultError::NotLoggedOn => "not logged in",
        EResultError::Busy => "Steam is busy, try again later",
        EResultError::Revoked => "access has been revoked",
        EResultError::NoConnection => "no connection to Steam",
        _ => "unknown error",
    }
}
