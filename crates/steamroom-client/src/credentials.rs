use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct TokenStore {
    pub tokens: HashMap<String, String>,
}

impl TokenStore {
    pub fn default_path() -> PathBuf {
        dirs_next::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".depotdownloader")
            .join("tokens.json")
    }

    pub fn load(path: &Path) -> Result<Self, std::io::Error> {
        let data = std::fs::read_to_string(path)?;
        serde_json::from_str(&data)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
    }

    pub fn save(&self, path: &Path) -> Result<(), std::io::Error> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self).map_err(std::io::Error::other)?;
        std::fs::write(path, json)
    }

    pub fn get(&self, account: &str) -> Option<&str> {
        self.tokens.get(account).map(|s| s.as_str())
    }

    pub fn set(&mut self, account: String, token: String) {
        self.tokens.insert(account, token);
    }
}
