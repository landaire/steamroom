use byteorder::{LittleEndian, ReadBytesExt};
use prost::Message;
use std::io::Cursor;
use super::{ChunkId, DepotId, DepotKey, ManifestId};
use crate::enums::ManifestMagic;
use crate::error::ManifestError;
use crate::generated::{ContentManifestMetadata, ContentManifestPayload};

#[derive(Clone, Debug)]
pub struct DepotManifest {
    pub depot_id: Option<DepotId>,
    pub manifest_id: Option<ManifestId>,
    pub creation_time: Option<u32>,
    pub filenames_encrypted: bool,
    pub total_uncompressed_size: Option<u64>,
    pub total_compressed_size: Option<u64>,
    pub files: Vec<ManifestFile>,
}

#[derive(Clone, Debug)]
pub struct ManifestFile {
    pub filename: Option<String>,
    pub size: Option<u64>,
    pub flags: Option<u32>,
    pub sha_content: Option<[u8; 20]>,
    pub chunks: Vec<ManifestChunk>,
    pub link_target: Option<String>,
}

#[derive(Clone, Debug)]
pub struct ManifestChunk {
    pub id: Option<ChunkId>,
    pub checksum: Option<u32>,
    pub offset: Option<u64>,
    pub compressed_size: Option<u32>,
    pub uncompressed_size: Option<u32>,
}

impl DepotManifest {
    pub fn parse(data: &[u8]) -> Result<Self, ManifestError> {
        let mut cursor = Cursor::new(data);

        let mut payload = None;
        let mut metadata = None;

        loop {
            let magic_val = match cursor.read_u32::<LittleEndian>() {
                Ok(v) => v,
                Err(_) => break,
            };

            let magic = match ManifestMagic::from_u32(magic_val) {
                Ok(m) => m,
                Err(_) => {
                    // Unknown section — try to skip, break on EOF
                    match cursor.read_u32::<LittleEndian>() {
                        Ok(section_len) => {
                            let pos = cursor.position() as usize;
                            cursor.set_position((pos + section_len as usize) as u64);
                            continue;
                        }
                        Err(_) => break,
                    }
                }
            };

            tracing::trace!("manifest section: {magic:?} at offset {}", cursor.position() as usize - 4);
            if magic == ManifestMagic::EndOfManifest {
                break;
            }

            let section_len = cursor
                .read_u32::<LittleEndian>()
                .map_err(|_| ManifestError::MissingSection)? as usize;

            let pos = cursor.position() as usize;
            if pos + section_len > data.len() {
                break;
            }
            let section_data = &data[pos..pos + section_len];
            cursor.set_position((pos + section_len) as u64);

            match magic {
                ManifestMagic::PayloadV5 | ManifestMagic::V4 => {
                    if payload.is_none() {
                        match ContentManifestPayload::decode(section_data) {
                            Ok(p) => payload = Some(p),
                            Err(e) => tracing::warn!("failed to decode payload ({} bytes): {e}", section_data.len()),
                        }
                    }
                }
                ManifestMagic::Metadata => {
                    if metadata.is_none() {
                        if let Ok(m) = ContentManifestMetadata::decode(section_data) {
                            metadata = Some(m);
                        }
                    }
                }
                ManifestMagic::Signature => {
                    // Signature — we don't validate it, just skip
                }
                ManifestMagic::EndOfManifest => break,
            }
        }

        tracing::debug!("manifest parse: payload={}, metadata={}", payload.is_some(), metadata.is_some());
        let payload = payload.ok_or(ManifestError::MissingSection)?;
        // V4 manifests may have metadata in a different magic section that we
        // parse into metadata. If absent, use defaults — the payload is still valid.
        let meta = metadata.unwrap_or_default();

        let files = payload
            .mappings
            .into_iter()
            .map(|m| {
                let sha_content = m.sha_content.as_deref().and_then(|b| {
                    if b.len() == 20 {
                        let mut arr = [0u8; 20];
                        arr.copy_from_slice(b);
                        Some(arr)
                    } else {
                        None
                    }
                });

                let chunks = m
                    .chunks
                    .into_iter()
                    .map(|c| {
                        let id = c.sha.as_deref().and_then(|b| {
                            if b.len() == 20 {
                                let mut arr = [0u8; 20];
                                arr.copy_from_slice(b);
                                Some(ChunkId(arr))
                            } else {
                                None
                            }
                        });
                        ManifestChunk {
                            id,
                            checksum: c.crc,
                            offset: c.offset,
                            compressed_size: c.cb_compressed,
                            uncompressed_size: c.cb_original,
                        }
                    })
                    .collect();

                ManifestFile {
                    filename: m.filename,
                    size: m.size,
                    flags: m.flags,
                    sha_content,
                    chunks,
                    link_target: m.linktarget,
                }
            })
            .collect();

        Ok(DepotManifest {
            depot_id: meta.depot_id.map(DepotId),
            manifest_id: meta.gid_manifest.map(ManifestId),
            creation_time: meta.creation_time,
            // proto2 optional bool: absent means not encrypted
            filenames_encrypted: meta.filenames_encrypted.unwrap_or(false),
            total_uncompressed_size: meta.cb_disk_original,
            total_compressed_size: meta.cb_disk_compressed,
            files,
        })
    }

    pub fn decrypt_filenames(&mut self, key: &DepotKey) -> Result<(), ManifestError> {
        if !self.filenames_encrypted {
            return Ok(());
        }
        for file in &mut self.files {
            if let Some(ref encrypted_name) = file.filename {
                let clean_b64: String =
                    encrypted_name.chars().filter(|c| !c.is_whitespace()).collect();
                let decoded = base64::Engine::decode(
                    &base64::engine::general_purpose::STANDARD,
                    &clean_b64,
                )?;

                // ECB(IV) + CBC(data) — same format as chunk encryption
                if decoded.len() < 32 {
                    return Err(ManifestError::DecryptFailed(
                        crate::error::CryptoError::DecryptionFailed,
                    ));
                }
                let iv = crate::crypto::symmetric_decrypt_ecb_nopad(&decoded[..16], &key.0)?;
                let decrypted = crate::crypto::symmetric_decrypt_cbc(&decoded[16..], &key.0, &iv)?;

                let name = decrypted.split(|&b| b == 0).next().unwrap_or(&decrypted);
                file.filename = Some(
                    String::from_utf8(name.to_vec()).map_err(|_| ManifestError::InvalidFilename)?,
                );
            }
        }
        self.filenames_encrypted = false;
        Ok(())
    }
}

impl ManifestMagic {
    fn from_u32(val: u32) -> Result<Self, ManifestError> {
        match val {
            0x1B81_B817 => Ok(Self::PayloadV5),
            0x1F4D_B10B => Ok(Self::Metadata),
            0x1B81_B813 => Ok(Self::Signature),
            0xD64B_F064 => Ok(Self::EndOfManifest),
            0x71F6_17D0 => Ok(Self::V4),
            0x1F48_12BE => Ok(Self::Metadata), // V4 metadata
            _ => Err(ManifestError::InvalidMagic(val)),
        }
    }
}
