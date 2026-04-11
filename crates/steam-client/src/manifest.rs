use std::path::{Path, PathBuf};
use steam::depot::manifest::DepotManifest;
use steam::depot::{DepotId, ManifestId};
use steam::error::ManifestError;

pub struct DepotConfig {
    // private
}

impl DepotConfig {
    pub fn path_for(_base: &Path, _depot_id: DepotId) -> PathBuf {
        todo!()
    }

    pub fn load(_path: &Path) -> Result<Self, std::io::Error> {
        todo!()
    }

    pub fn save(&self, _path: &Path) -> Result<(), std::io::Error> {
        todo!()
    }

    pub fn get_installed(&self, _depot_id: DepotId) -> Option<ManifestId> {
        todo!()
    }

    pub fn set_installed(&mut self, _depot_id: DepotId, _manifest_id: ManifestId) {
        todo!()
    }
}

pub struct ManifestCache {
    // private
}

impl ManifestCache {
    pub fn new(_base: PathBuf) -> Self {
        todo!()
    }

    pub fn default_for(_depot_id: DepotId) -> PathBuf {
        todo!()
    }

    pub fn load(
        &self,
        _depot_id: DepotId,
        _manifest_id: ManifestId,
    ) -> Result<Option<DepotManifest>, ManifestError> {
        todo!()
    }

    pub fn save(
        &self,
        _depot_id: DepotId,
        _manifest_id: ManifestId,
        _data: &[u8],
    ) -> Result<(), std::io::Error> {
        todo!()
    }
}

pub fn extract_and_parse(_data: &[u8]) -> Result<DepotManifest, ManifestError> {
    todo!()
}
