use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default)]
pub struct TokenStore {
    pub tokens: HashMap<String, String>,
}

impl TokenStore {
    pub fn default_path() -> PathBuf {
        todo!()
    }

    pub fn load(path: &Path) -> Result<Self, std::io::Error> {
        todo!()
    }

    pub fn save(&self, path: &Path) -> Result<(), std::io::Error> {
        todo!()
    }

    pub fn get(&self, account: &str) -> Option<&str> {
        self.tokens.get(account).map(|s| s.as_str())
    }

    pub fn set(&mut self, account: String, token: String) {
        self.tokens.insert(account, token);
    }
}
