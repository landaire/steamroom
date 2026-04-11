#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct CdnServer {
    pub host: String,
    pub port: u16,
    pub https: bool,
    pub vhost: String,
}

impl CdnServer {
    pub fn build_url(&self, path: &str, cdn_auth_token: Option<&str>) -> String {
        let scheme = if self.https { "https" } else { "http" };
        let base = format!("{scheme}://{}:{}{path}", self.host, self.port);
        match cdn_auth_token {
            Some(token) if !token.is_empty() => format!("{base}{token}"),
            _ => base,
        }
    }
}
