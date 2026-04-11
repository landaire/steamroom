#[derive(Debug, thiserror::Error)]
pub enum CliError {
    #[error(transparent)]
    Steam(#[from] steamroom::error::Error),

    #[error(transparent)]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Crypto(#[from] steamroom::error::CryptoError),

    #[error(transparent)]
    Join(#[from] tokio::task::JoinError),

    #[error(transparent)]
    Manifest(#[from] steamroom::error::ManifestError),

    #[error(transparent)]
    Chunk(#[from] steamroom::depot::chunk::ChunkError),

    #[error(transparent)]
    Protobuf(#[from] prost::DecodeError),

    #[error(transparent)]
    Regex(#[from] regex::Error),

    #[error(transparent)]
    Json(#[from] serde_json::Error),

    #[error(transparent)]
    Http(#[from] reqwest::Error),

    #[error(transparent)]
    Kv(#[from] steamroom::types::key_value::TextKvError),

    #[error("no CM servers available")]
    NoCmServers,

    #[error("no product info returned for app {0}")]
    NoProductInfo(u32),

    #[error("app {0} has no KV data")]
    NoKvData(u32),

    #[error("no depots found in app info")]
    NoDepots,

    #[error("depot {0} not found in app info")]
    DepotNotFound(u32),

    #[error("manifest not found for depot {depot} branch {branch}")]
    ManifestNotFound { depot: u32, branch: String },

    #[error("invalid manifest ID")]
    InvalidManifestId,

    #[error("no CDN servers available")]
    NoCdnServers,

    #[error("chunk is missing chunk ID")]
    ChunkMissingId,
}
