#[derive(Clone, Debug)]
pub struct CdnServer {
    pub host: String,
    pub port: u16,
    pub https: bool,
    pub vhost: String,
}

impl CdnServer {
    pub fn build_url(&self, path: &str, cdn_auth_token: Option<&str>) -> String {
        todo!()
    }
}
