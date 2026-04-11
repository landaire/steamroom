use std::collections::HashMap;
use std::path::{Path, PathBuf};
use steamroom::depot::{DepotId, ManifestId};

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct DepotConfig {
    pub installed_manifests: HashMap<u32, u64>,
}

impl DepotConfig {
    pub fn config_dir(install_dir: &Path) -> PathBuf {
        install_dir.join(".depotdownloader")
    }

    pub fn config_path(install_dir: &Path) -> PathBuf {
        Self::config_dir(install_dir).join("depot.json")
    }

    pub fn load(install_dir: &Path) -> Self {
        let path = Self::config_path(install_dir);
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|data| serde_json::from_str(&data).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, install_dir: &Path) -> Result<(), std::io::Error> {
        let dir = Self::config_dir(install_dir);
        std::fs::create_dir_all(&dir)?;
        let json = serde_json::to_string_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
        std::fs::write(Self::config_path(install_dir), json)
    }

    pub fn get_installed(&self, depot_id: DepotId) -> Option<ManifestId> {
        self.installed_manifests
            .get(&depot_id.0)
            .map(|&id| ManifestId(id))
    }

    pub fn set_installed(&mut self, depot_id: DepotId, manifest_id: ManifestId) {
        self.installed_manifests
            .insert(depot_id.0, manifest_id.0);
    }
}
