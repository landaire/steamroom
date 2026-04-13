use std::path::PathBuf;
use steamroom::depot::DepotId;
use steamroom::depot::ManifestId;
use steamroom::depot::manifest::DepotManifest;
use steamroom::error::ManifestError;

pub struct ManifestCache {
    base: PathBuf,
}

impl ManifestCache {
    pub fn new(base: PathBuf) -> Self {
        Self { base }
    }

    pub fn default_path() -> PathBuf {
        dirs_next::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".depotdownloader")
            .join("manifests")
    }

    fn cache_path(&self, depot_id: DepotId, manifest_id: ManifestId) -> PathBuf {
        self.base
            .join(format!("{}_{}.manifest", depot_id.0, manifest_id.0))
    }

    fn raw_cache_path(&self, depot_id: DepotId, manifest_id: ManifestId) -> PathBuf {
        self.base
            .join(format!("{}_{}.zip", depot_id.0, manifest_id.0))
    }

    pub fn load(&self, depot_id: DepotId, manifest_id: ManifestId) -> Option<Vec<u8>> {
        std::fs::read(self.cache_path(depot_id, manifest_id)).ok()
    }

    pub fn load_raw(&self, depot_id: DepotId, manifest_id: ManifestId) -> Option<Vec<u8>> {
        std::fs::read(self.raw_cache_path(depot_id, manifest_id)).ok()
    }

    pub fn save(
        &self,
        depot_id: DepotId,
        manifest_id: ManifestId,
        decompressed: &[u8],
        raw: &[u8],
    ) -> Result<(), std::io::Error> {
        if let Some(parent) = self.base.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::create_dir_all(&self.base)?;
        std::fs::write(self.cache_path(depot_id, manifest_id), decompressed)?;
        std::fs::write(self.raw_cache_path(depot_id, manifest_id), raw)?;
        Ok(())
    }
}

pub fn parse_cdn_manifest(data: &[u8]) -> Result<DepotManifest, ManifestError> {
    // Manifest data may be zip-compressed
    let bytes = if data.len() > 2 && data[0] == 0x50 && data[1] == 0x4B {
        let cursor = std::io::Cursor::new(data);
        let mut archive =
            zip::ZipArchive::new(cursor).map_err(|_| ManifestError::MissingSection)?;
        if archive.is_empty() {
            return Err(ManifestError::MissingSection);
        }
        let mut file = archive
            .by_index(0)
            .map_err(|_| ManifestError::MissingSection)?;
        let mut buf = Vec::new();
        std::io::Read::read_to_end(&mut file, &mut buf)
            .map_err(|_| ManifestError::MissingSection)?;
        buf
    } else {
        data.to_vec()
    };
    DepotManifest::parse(&bytes)
}
