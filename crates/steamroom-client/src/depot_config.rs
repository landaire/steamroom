use std::collections::HashMap;
use std::path::{Path, PathBuf};
use steamroom::depot::{DepotId, DepotKey, ManifestId};

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct DepotInfo {
    pub manifest_id: u64,
    pub depot_key: String,
}

#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct DepotConfig {
    pub depots: HashMap<u32, DepotInfo>,
}

impl DepotConfig {
    pub fn config_dir(install_dir: &Path) -> PathBuf {
        install_dir.join(".depotdownloader")
    }

    pub fn config_path(install_dir: &Path) -> PathBuf {
        Self::config_dir(install_dir).join("depot.json")
    }

    fn manifests_dir(install_dir: &Path) -> PathBuf {
        Self::config_dir(install_dir).join("manifests")
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

    pub fn get_installed(&self, depot_id: DepotId) -> Option<(ManifestId, DepotKey)> {
        let info = self.depots.get(&depot_id.0)?;
        let key_bytes = decode_hex(&info.depot_key)?;
        if key_bytes.len() != 32 {
            return None;
        }
        let mut key = [0u8; 32];
        key.copy_from_slice(&key_bytes);
        Some((ManifestId(info.manifest_id), DepotKey(key)))
    }

    pub fn set_installed(&mut self, depot_id: DepotId, manifest_id: ManifestId, depot_key: &DepotKey) {
        self.depots.insert(depot_id.0, DepotInfo {
            manifest_id: manifest_id.0,
            depot_key: encode_hex(&depot_key.0),
        });
    }

    /// Save the raw CDN manifest response (zip-compressed, filenames still encrypted).
    pub fn save_manifest_raw(
        install_dir: &Path,
        depot_id: DepotId,
        manifest_id: ManifestId,
        data: &[u8],
    ) -> Result<(), std::io::Error> {
        let dir = Self::manifests_dir(install_dir);
        std::fs::create_dir_all(&dir)?;
        std::fs::write(
            dir.join(format!("{}_{}.zip", depot_id.0, manifest_id.0)),
            data,
        )
    }

    /// Save the decompressed manifest bytes (protobuf sections, pre-filename-decryption).
    pub fn save_manifest_decompressed(
        install_dir: &Path,
        depot_id: DepotId,
        manifest_id: ManifestId,
        data: &[u8],
    ) -> Result<(), std::io::Error> {
        let dir = Self::manifests_dir(install_dir);
        std::fs::create_dir_all(&dir)?;
        std::fs::write(
            dir.join(format!("{}_{}.manifest", depot_id.0, manifest_id.0)),
            data,
        )
    }

    /// Load a previously saved decompressed manifest.
    pub fn load_manifest_decompressed(
        install_dir: &Path,
        depot_id: DepotId,
        manifest_id: ManifestId,
    ) -> Option<Vec<u8>> {
        let path = Self::manifests_dir(install_dir)
            .join(format!("{}_{}.manifest", depot_id.0, manifest_id.0));
        std::fs::read(&path).ok()
    }
}

fn encode_hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        let _ = write!(s, "{b:02x}");
    }
    s
}

fn decode_hex(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}
