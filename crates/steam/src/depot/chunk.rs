use super::DepotKey;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ChunkCompression {
    VZstd,
    Lzma,
    Zip,
    None,
}

impl ChunkCompression {
    pub fn detect(data: &[u8]) -> Self {
        todo!()
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ChunkError {
    #[error("chunk data too short")]
    TooShort,

    #[error("size mismatch")]
    SizeMismatch,

    #[error("checksum mismatch")]
    ChecksumMismatch,

    #[error("empty archive")]
    EmptyArchive,

    #[error("crypto: {0}")]
    Crypto(#[from] crate::error::CryptoError),

    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error("zip: {0}")]
    Zip(String),
}

pub fn process_chunk(
    data: &[u8],
    depot_key: &DepotKey,
    expected_size: u32,
    expected_checksum: u32,
) -> Result<Vec<u8>, ChunkError> {
    todo!()
}
