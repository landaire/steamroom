use super::DepotKey;
use crate::util::checksum::SteamAdler32;
use std::io::Read;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ChunkCompression {
    VZstd,
    VZlzma,
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
            [0x56, 0x53] => Self::VZstd,  // "VS" (VSZa header)
            [0x56, 0x5A] => Self::VZlzma, // "VZ" (VZa header)
            [0x5D, _] => Self::Lzma,
            [0x50, 0x4B] => Self::Zip, // "PK"
            _ => Self::None,
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
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

/// Decrypt and decompress a raw depot chunk, verifying size and checksum.
///
/// The chunk format is `ECB(IV, 16 bytes) || CBC(compressed_payload)`. After
/// decryption, the payload is decompressed based on its magic bytes (Valve zstd,
/// Valve LZMA, raw LZMA, zip, or uncompressed). The result is verified against
/// `expected_size` and `expected_checksum` (Steam's zero-seeded Adler-32).
pub fn process_chunk(
    data: &[u8],
    depot_key: &DepotKey,
    expected_size: u32,
    expected_checksum: u32,
) -> Result<Vec<u8>, ChunkError> {
    let _ = expected_checksum; // Used after decompress
    if data.len() < 4 {
        return Err(ChunkError::TooShort);
    }

    if data.len() < 32 {
        return Err(ChunkError::TooShort);
    }

    // Chunk format: ECB_encrypted_IV(16) + CBC_ciphertext(remaining)
    // 1. ECB decrypt first 16 bytes to get IV
    let iv = crate::crypto::symmetric_decrypt_ecb_nopad(&data[..16], &depot_key.0)?;
    // 2. CBC decrypt the rest using the decrypted IV
    let decrypted = crate::crypto::symmetric_decrypt_cbc(&data[16..], &depot_key.0, &iv)?;

    // Detect compression and decompress
    tracing::debug!(
        "chunk decrypted: {} bytes, first 20: {:02x?}, compression: {:?}",
        decrypted.len(),
        &decrypted[..decrypted.len().min(20)],
        ChunkCompression::detect(&decrypted)
    );
    let decompressed = decompress(&decrypted, expected_size)?;

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

fn decompress(data: &[u8], expected_size: u32) -> Result<Vec<u8>, ChunkError> {
    match ChunkCompression::detect(data) {
        ChunkCompression::VZstd => {
            // Valve zstd: "VSZa"(4) + CRC32(4) + zstd_data(N) + CRC32(4) + orig_size(8) + "zsv"(3)
            const HEADER: usize = 4 + 4; // "VSZa" + CRC32
            const FOOTER: usize = 4 + 8 + 3; // CRC32 + orig_size(u64) + "zsv"
            if data.len() < HEADER + FOOTER {
                return Err(ChunkError::TooShort);
            }
            let compressed = &data[HEADER..data.len() - FOOTER];
            let output = zstd::bulk::decompress(compressed, expected_size as usize)
                .map_err(|e| ChunkError::Io(std::io::Error::other(e)))?;
            Ok(output)
        }
        ChunkCompression::VZlzma => {
            // Valve LZMA: "VZa"(3) + CRC32(4) + LZMA_props(5) + LZMA_data(N) + CRC32(4) + orig_size(4) + "zv"(2)
            const HEADER: usize = 3 + 4; // "VZa" + CRC32
            const PROPS: usize = 5; // LZMA properties
            const FOOTER: usize = 4 + 4 + 2; // CRC32 + orig_size(u32) + "zv"
            if data.len() < HEADER + PROPS + FOOTER {
                return Err(ChunkError::TooShort);
            }
            let props = &data[HEADER..HEADER + PROPS];
            let lzma_data = &data[HEADER + PROPS..data.len() - FOOTER];

            // Build standard LZMA stream: props(5) + uncompressed_size(8 LE) + data
            let mut lzma_stream = Vec::with_capacity(13 + lzma_data.len());
            lzma_stream.extend_from_slice(props);
            lzma_stream.extend_from_slice(&(expected_size as u64).to_le_bytes());
            lzma_stream.extend_from_slice(lzma_data);

            let mut output = Vec::with_capacity(expected_size as usize);
            lzma_rs::lzma_decompress(&mut std::io::Cursor::new(&lzma_stream), &mut output)
                .map_err(|e| ChunkError::Io(std::io::Error::other(e)))?;
            Ok(output)
        }
        ChunkCompression::Lzma => {
            let mut output = Vec::new();
            lzma_rs::lzma_decompress(&mut std::io::Cursor::new(data), &mut output)
                .map_err(|e| ChunkError::Io(std::io::Error::other(e)))?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_vzstd() {
        assert_eq!(
            ChunkCompression::detect(b"VSZa\x00\x00\x00\x00rest"),
            ChunkCompression::VZstd
        );
    }

    #[test]
    fn detect_vzlzma() {
        assert_eq!(
            ChunkCompression::detect(b"VZa\x00\x00\x00\x00rest"),
            ChunkCompression::VZlzma
        );
    }

    #[test]
    fn detect_lzma() {
        assert_eq!(
            ChunkCompression::detect(&[0x5D, 0x00, 0x00]),
            ChunkCompression::Lzma
        );
    }

    #[test]
    fn detect_zip() {
        assert_eq!(
            ChunkCompression::detect(b"PK\x03\x04"),
            ChunkCompression::Zip
        );
    }

    #[test]
    fn detect_none_for_unknown() {
        assert_eq!(
            ChunkCompression::detect(b"\x00\x00\x00\x00"),
            ChunkCompression::None
        );
    }

    #[test]
    fn detect_none_for_short_input() {
        assert_eq!(ChunkCompression::detect(b""), ChunkCompression::None);
        assert_eq!(ChunkCompression::detect(b"V"), ChunkCompression::None);
    }

    #[test]
    fn process_chunk_too_short() {
        let key = DepotKey([0; 32]);
        assert!(matches!(
            process_chunk(b"short", &key, 0, 0),
            Err(ChunkError::TooShort)
        ));
    }

    #[test]
    fn process_chunk_uncompressed_roundtrip() {
        let key = DepotKey([0xAA; 32]);
        let plaintext = b"test chunk data!";
        let checksum = crate::util::checksum::SteamAdler32::compute(plaintext);

        let iv = [0x42u8; 16];
        let encrypted_iv = crate::crypto::symmetric_encrypt_ecb_nopad(&iv, &key.0).unwrap();
        let encrypted_body = crate::crypto::symmetric_encrypt_cbc(plaintext, &key.0, &iv).unwrap();

        let mut chunk_data = Vec::new();
        chunk_data.extend_from_slice(&encrypted_iv);
        chunk_data.extend_from_slice(&encrypted_body);

        let result = process_chunk(&chunk_data, &key, plaintext.len() as u32, checksum.0).unwrap();
        assert_eq!(result, plaintext);
    }

    #[test]
    fn process_chunk_bad_checksum() {
        let key = DepotKey([0xAA; 32]);
        let plaintext = b"test chunk data!";

        let iv = [0x42u8; 16];
        let encrypted_iv = crate::crypto::symmetric_encrypt_ecb_nopad(&iv, &key.0).unwrap();
        let encrypted_body = crate::crypto::symmetric_encrypt_cbc(plaintext, &key.0, &iv).unwrap();

        let mut chunk_data = Vec::new();
        chunk_data.extend_from_slice(&encrypted_iv);
        chunk_data.extend_from_slice(&encrypted_body);

        let result = process_chunk(&chunk_data, &key, plaintext.len() as u32, 0xDEADBEEF);
        assert!(matches!(result, Err(ChunkError::ChecksumMismatch { .. })));
    }

    #[test]
    fn process_chunk_wrong_key_fails() {
        let key = DepotKey([0xAA; 32]);
        let wrong_key = DepotKey([0xBB; 32]);
        let plaintext = b"test chunk data!";

        let iv = [0x42u8; 16];
        let encrypted_iv = crate::crypto::symmetric_encrypt_ecb_nopad(&iv, &key.0).unwrap();
        let encrypted_body = crate::crypto::symmetric_encrypt_cbc(plaintext, &key.0, &iv).unwrap();

        let mut chunk_data = Vec::new();
        chunk_data.extend_from_slice(&encrypted_iv);
        chunk_data.extend_from_slice(&encrypted_body);

        let result = process_chunk(&chunk_data, &wrong_key, plaintext.len() as u32, 0);
        assert!(result.is_err());
    }
}
