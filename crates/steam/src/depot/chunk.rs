use std::io::Read;
use super::DepotKey;
use crate::util::checksum::SteamAdler32;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ChunkCompression {
    VZstd,
    Lzma,
    Zip,
    None,
}

impl ChunkCompression {
    pub fn detect(data: &[u8]) -> Self {
        if data.len() < 2 {
            return Self::None;
        }
        match &data[..2] {
            [0x56, 0x5A] => Self::VZstd, // "VZ"
            [0x5D, _] => Self::Lzma,
            [0x50, 0x4B] => Self::Zip, // "PK"
            _ => Self::None,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ChunkError {
    #[error("chunk data too short")]
    TooShort,

    #[error("size mismatch: expected {expected}, got {actual}")]
    SizeMismatch { expected: u32, actual: u32 },

    #[error("checksum mismatch: expected {expected:#010x}, got {actual:#010x}")]
    ChecksumMismatch { expected: u32, actual: u32 },

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
    if data.len() < 4 {
        return Err(ChunkError::TooShort);
    }

    // Decrypt with AES-256-ECB
    let decrypted = crate::crypto::symmetric_decrypt_ecb(data, &depot_key.0)?;

    // Detect compression and decompress
    let decompressed = decompress(&decrypted)?;

    // Verify size
    if decompressed.len() != expected_size as usize {
        return Err(ChunkError::SizeMismatch {
            expected: expected_size,
            actual: decompressed.len() as u32,
        });
    }

    // Verify checksum (Steam uses non-standard Adler32 with zero seed)
    let checksum = SteamAdler32::compute(&decompressed);
    if checksum.0 != expected_checksum {
        return Err(ChunkError::ChecksumMismatch {
            expected: expected_checksum,
            actual: checksum.0,
        });
    }

    Ok(decompressed)
}

fn decompress(data: &[u8]) -> Result<Vec<u8>, ChunkError> {
    match ChunkCompression::detect(data) {
        ChunkCompression::VZstd => {
            // "VZ" header (2 bytes) + original_size (4 bytes LE) + zstd-compressed data
            if data.len() < 6 {
                return Err(ChunkError::TooShort);
            }
            let original_size =
                u32::from_le_bytes(data[2..6].try_into().unwrap()) as usize;
            let compressed = &data[6..];
            let mut output = Vec::with_capacity(original_size);
            let mut decoder = zstd::stream::read::Decoder::new(compressed)?;
            decoder.read_to_end(&mut output)?;
            Ok(output)
        }
        ChunkCompression::Lzma => {
            let mut output = Vec::new();
            lzma_rs::lzma_decompress(&mut std::io::Cursor::new(data), &mut output)
                .map_err(|e| ChunkError::Io(std::io::Error::new(std::io::ErrorKind::Other, e)))?;
            Ok(output)
        }
        ChunkCompression::Zip => {
            let cursor = std::io::Cursor::new(data);
            let mut archive =
                zip::ZipArchive::new(cursor).map_err(|e| ChunkError::Zip(e.to_string()))?;
            if archive.is_empty() {
                return Err(ChunkError::EmptyArchive);
            }
            let mut file = archive
                .by_index(0)
                .map_err(|e| ChunkError::Zip(e.to_string()))?;
            let mut output = Vec::new();
            file.read_to_end(&mut output)?;
            Ok(output)
        }
        ChunkCompression::None => Ok(data.to_vec()),
    }
}
