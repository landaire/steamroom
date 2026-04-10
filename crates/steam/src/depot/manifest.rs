use super::{ChunkId, DepotId, DepotKey, ManifestId};
use crate::error::ManifestError;

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
        todo!()
    }

    pub fn decrypt_filenames(&mut self, key: &DepotKey) -> Result<(), ManifestError> {
        todo!()
    }
}
