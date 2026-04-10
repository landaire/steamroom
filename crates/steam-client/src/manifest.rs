use std::path::{Path, PathBuf};
use steam::depot::manifest::DepotManifest;
use steam::depot::{DepotId, ManifestId};
use steam::error::ManifestError;

pub struct DepotConfig {
    // private
}

impl DepotConfig {
    pub fn path_for(base: &Path, depot_id: DepotId) -> PathBuf {
        todo!()
    }

    pub fn load(path: &Path) -> Result<Self, std::io::Error> {
        todo!()
    }

    pub fn save(&self, path: &Path) -> Result<(), std::io::Error> {
        todo!()
    }

    pub fn get_installed(&self, depot_id: DepotId) -> Option<ManifestId> {
        todo!()
    }

    pub fn set_installed(&mut self, depot_id: DepotId, manifest_id: ManifestId) {
        todo!()
    }
}

pub struct ManifestCache {
    // private
}

impl ManifestCache {
    pub fn new(base: PathBuf) -> Self {
        todo!()
    }

    pub fn default_for(depot_id: DepotId) -> PathBuf {
        todo!()
    }

    pub fn load(
        &self,
        depot_id: DepotId,
        manifest_id: ManifestId,
    ) -> Result<Option<DepotManifest>, ManifestError> {
        todo!()
    }

    pub fn save(
        &self,
        depot_id: DepotId,
        manifest_id: ManifestId,
        data: &[u8],
    ) -> Result<(), std::io::Error> {
        todo!()
    }
}

pub fn extract_and_parse(data: &[u8]) -> Result<DepotManifest, ManifestError> {
    todo!()
}
