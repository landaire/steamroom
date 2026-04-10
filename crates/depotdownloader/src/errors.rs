#[derive(Debug, thiserror::Error)]
pub enum CliError {
    #[error(transparent)]
    Steam(#[from] steam::error::Error),

    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("join: {0}")]
    Join(#[from] tokio::task::JoinError),

    #[error("manifest: {0}")]
    Manifest(#[from] steam::error::ManifestError),

    #[error("chunk: {0}")]
    Chunk(#[from] steam::depot::chunk::ChunkError),

    #[error("protobuf: {0}")]
    Protobuf(#[from] prost::DecodeError),

    #[error("regex: {0}")]
    Regex(#[from] regex::Error),

    #[error("json: {0}")]
    Json(#[from] serde_json::Error),

    #[error("http: {0}")]
    Reqwest(#[from] reqwest::Error),

    #[error("{0}")]
    Other(String),
}
