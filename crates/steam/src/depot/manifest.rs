use byteorder::{LittleEndian, ReadBytesExt};
use prost::Message;
use std::io::Cursor;
use super::{ChunkId, DepotId, DepotKey, ManifestId};
use crate::enums::ManifestMagic;
use crate::error::ManifestError;
use crate::generated::{ContentManifestMetadata, ContentManifestPayload, ContentManifestSignature};

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
        let mut _signature = None;

        loop {
            let magic_val = match cursor.read_u32::<LittleEndian>() {
                Ok(v) => v,
                Err(_) => break,
            };

            let magic = match_magic(magic_val)?;

            if magic == ManifestMagic::EndOfManifest {
                break;
            }

            let section_len = cursor
                .read_u32::<LittleEndian>()
                .map_err(|_| ManifestError::MissingSection)? as usize;

            let pos = cursor.position() as usize;
            if pos + section_len > data.len() {
                return Err(ManifestError::MissingSection);
            }
            let section_data = &data[pos..pos + section_len];
            cursor.set_position((pos + section_len) as u64);

            match magic {
                ManifestMagic::PayloadV5 | ManifestMagic::V4 => {
                    payload = Some(
                        ContentManifestPayload::decode(section_data)
                            .map_err(|_| ManifestError::InvalidMagic)?,
                    );
                }
                ManifestMagic::Metadata => {
                    metadata = Some(
                        ContentManifestMetadata::decode(section_data)
                            .map_err(|_| ManifestError::InvalidMagic)?,
                    );
                }
                ManifestMagic::Signature => {
                    _signature = Some(
                        ContentManifestSignature::decode(section_data)
                            .map_err(|_| ManifestError::InvalidMagic)?,
                    );
                }
                ManifestMagic::EndOfManifest => break,
            }
        }

        let payload = payload.ok_or(ManifestError::MissingSection)?;
        let meta = metadata.ok_or(ManifestError::MissingSection)?;

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
                let decoded = base64::Engine::decode(
                    &base64::engine::general_purpose::STANDARD,
                    encrypted_name,
                )
                .map_err(|_| ManifestError::InvalidMagic)?;

                let decrypted = crate::crypto::symmetric_decrypt_ecb(&decoded, &key.0)
                    .map_err(|_| ManifestError::InvalidMagic)?;

                // Decrypted filename is null-terminated UTF-8
                let name = decrypted
                    .split(|&b| b == 0)
                    .next()
                    .unwrap_or(&decrypted);
                file.filename = Some(
                    String::from_utf8(name.to_vec())
                        .map_err(|_| ManifestError::InvalidMagic)?,
                );
            }
        }
        self.filenames_encrypted = false;
        Ok(())
    }
}

fn match_magic(val: u32) -> Result<ManifestMagic, ManifestError> {
    match val {
        0x1B81_B817 => Ok(ManifestMagic::PayloadV5),
        0x1F4D_B10B => Ok(ManifestMagic::Metadata),
        0x1B81_B813 => Ok(ManifestMagic::Signature),
        0xD64B_F064 => Ok(ManifestMagic::EndOfManifest),
        0x71_710712 => Ok(ManifestMagic::V4),
        _ => Err(ManifestError::InvalidMagic),
    }
}
