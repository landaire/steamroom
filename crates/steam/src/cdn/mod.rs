pub mod lancache;
pub mod server;

use bytes::Bytes;
use crate::depot::{ChunkId, DepotId, ManifestId};
use crate::error::Error;
pub use self::server::CdnServer;

pub struct CdnClient {
    client: reqwest::Client,
    lancache: bool,
}

impl CdnClient {
    pub fn new() -> Result<Self, Error> {
        todo!()
    }

    pub fn with_lancache(mut self) -> Self {
        self.lancache = true;
        self
    }

    pub fn is_lancache(&self) -> bool {
        self.lancache
    }

    pub async fn download_manifest(
        &self,
        server: &CdnServer,
        depot_id: DepotId,
        manifest_id: ManifestId,
        request_code: u64,
        cdn_auth_token: Option<&str>,
    ) -> Result<Bytes, Error> {
        todo!()
    }

    pub async fn download_chunk(
        &self,
        server: &CdnServer,
        depot_id: DepotId,
        chunk_id: &ChunkId,
        cdn_auth_token: Option<&str>,
    ) -> Result<Bytes, Error> {
        todo!()
    }
}
