use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct TokenStore {
    #[serde(default)]
    pub tokens: HashMap<String, String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub startup_auth: Option<StartupAuth>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case", tag = "method")]
pub enum StartupAuth {
    SteamToken {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        account_name: Option<String>,
    },
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

    pub fn load_or_default(path: &Path) -> Result<Self, std::io::Error> {
        match Self::load(path) {
            Ok(store) => Ok(store),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e),
        }
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

    pub fn remove(&mut self, account: &str) {
        self.tokens.remove(account);
    }

    pub fn set(&mut self, account: String, token: String) {
        self.tokens.insert(account, token);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_legacy_tokens_with_startup_auth() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("tokens.json");
        std::fs::write(&path, r#"{"tokens":{"account":"refresh-token"}}"#).unwrap();

        let mut store = TokenStore::load_or_default(&path).unwrap();
        assert_eq!(store.get("account"), Some("refresh-token"));
        assert_eq!(store.startup_auth, None);

        store.startup_auth = Some(StartupAuth::SteamToken {
            account_name: Some("account".to_string()),
        });
        store.save(&path).unwrap();

        let reloaded = TokenStore::load(&path).unwrap();
        assert_eq!(reloaded.get("account"), Some("refresh-token"));
        assert_eq!(reloaded.startup_auth, store.startup_auth);
    }
}
