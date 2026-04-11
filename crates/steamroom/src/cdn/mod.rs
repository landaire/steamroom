pub mod lancache;
pub mod server;

use bytes::Bytes;
pub use self::server::CdnServer;
use crate::depot::{ChunkId, DepotId, ManifestId};
use crate::error::Error;

pub struct CdnClient {
    client: reqwest::Client,
    lancache_ip: Option<std::net::IpAddr>,
}

impl CdnClient {
    pub fn new() -> Result<Self, Error> {
        let client = reqwest::Client::builder()
            .build()
            .map_err(Error::Http)?;
        Ok(Self {
            client,
            lancache_ip: None,
        })
    }

    pub fn with_lancache(mut self) -> Self {
        self.lancache_ip = lancache::detect();
        self
    }

    pub fn is_lancache(&self) -> bool {
        self.lancache_ip.is_some()
    }

    pub async fn download_manifest(
        &self,
        server: &CdnServer,
        depot_id: DepotId,
        manifest_id: ManifestId,
        request_code: u64,
        cdn_auth_token: Option<&str>,
    ) -> Result<Bytes, Error> {
        let path = format!(
            "/depot/{}/manifest/{}/5/{}",
            depot_id.0, manifest_id.0, request_code
        );
        let url = self.build_url(server, &path, cdn_auth_token);
        let resp = self.build_request(server, &url).send().await?;
        Ok(resp.bytes().await?)
    }

    pub async fn download_chunk(
        &self,
        server: &CdnServer,
        depot_id: DepotId,
        chunk_id: &ChunkId,
        cdn_auth_token: Option<&str>,
    ) -> Result<Bytes, Error> {
        let path = format!("/depot/{}/chunk/{}", depot_id.0, chunk_id);
        let url = self.build_url(server, &path, cdn_auth_token);
        let resp = self.build_request(server, &url).send().await?;
        Ok(resp.bytes().await?)
    }

    fn build_url(&self, server: &CdnServer, path: &str, cdn_auth_token: Option<&str>) -> String {
        if let Some(ip) = self.lancache_ip {
            lancache::build_url(ip, server, path)
        } else {
            server.build_url(path, cdn_auth_token)
        }
    }

    fn build_request(&self, server: &CdnServer, url: &str) -> reqwest::RequestBuilder {
        let mut req = self.client.get(url);
        if self.lancache_ip.is_some() {
            req = req.header("Host", lancache::host_header(server));
        }
        req
    }
}
