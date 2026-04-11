use std::future::Future;
use std::time::Duration;
use bytes::Bytes;
use steam::cdn::CdnClient;
use steam::cdn::server::CdnServer;
use steam::depot::{ChunkId, DepotId, DepotKey};
use steam::depot::manifest::DepotManifest;
use crate::event::DownloadEvent;

pub type BoxError = Box<dyn std::error::Error + Send + Sync>;

pub trait ChunkFetcher: Send + Sync {
    fn fetch_chunk(
        &self,
        depot_id: DepotId,
        chunk_id: &ChunkId,
    ) -> impl Future<Output = Result<Bytes, BoxError>> + Send;
}

pub struct CdnChunkFetcher {
    pub cdn: CdnClient,
    pub server: CdnServer,
    pub cdn_auth_token: Option<String>,
}

impl ChunkFetcher for CdnChunkFetcher {
    async fn fetch_chunk(
        &self,
        _depot_id: DepotId,
        _chunk_id: &ChunkId,
    ) -> Result<Bytes, BoxError> {
        todo!()
    }
}

#[derive(Clone, Debug)]
pub struct RetryConfig {
    pub max_attempts: u32,
    pub initial_delay: Duration,
}

impl RetryConfig {
    pub fn none() -> Self {
        Self {
            max_attempts: 1,
            initial_delay: Duration::ZERO,
        }
    }
}

pub enum FileFilter {
    FileList(Vec<String>),
    Regex(regex::Regex),
    Mixed {
        files: Vec<String>,
        regex: regex::Regex,
    },
}

impl FileFilter {
    pub fn from_filelist(files: Vec<String>) -> Self {
        Self::FileList(files)
    }

    pub fn from_regex(pattern: &str) -> Result<Self, regex::Error> {
        Ok(Self::Regex(regex::Regex::new(pattern)?))
    }

    pub fn matches(&self, _filename: &str) -> bool {
        todo!()
    }
}

pub struct DepotJobBuilder {
    depot_id: Option<DepotId>,
    depot_key: Option<DepotKey>,
    install_dir: Option<std::path::PathBuf>,
    max_downloads: Option<usize>,
    verify: bool,
    file_filter: Option<FileFilter>,
    retry: RetryConfig,
}

impl DepotJobBuilder {
    pub fn new() -> Self {
        Self {
            depot_id: None,
            depot_key: None,
            install_dir: None,
            max_downloads: None,
            verify: false,
            file_filter: None,
            retry: RetryConfig::none(),
        }
    }

    pub fn depot_id(mut self, id: DepotId) -> Self {
        self.depot_id = Some(id);
        self
    }

    pub fn depot_key(mut self, key: DepotKey) -> Self {
        self.depot_key = Some(key);
        self
    }

    pub fn install_dir(mut self, dir: std::path::PathBuf) -> Self {
        self.install_dir = Some(dir);
        self
    }

    pub fn max_downloads(mut self, n: usize) -> Self {
        self.max_downloads = Some(n);
        self
    }

    pub fn verify(mut self, v: bool) -> Self {
        self.verify = v;
        self
    }

    pub fn file_filter(mut self, f: FileFilter) -> Self {
        self.file_filter = Some(f);
        self
    }

    pub fn retry(mut self, config: RetryConfig) -> Self {
        self.retry = config;
        self
    }

    pub fn event_sender(
        self,
        _tx: tokio::sync::mpsc::UnboundedSender<DownloadEvent>,
    ) -> Self {
        todo!()
    }

    pub fn previous_manifest(self, _manifest: DepotManifest) -> Self {
        todo!()
    }

    pub fn build(self) -> Result<DepotJob, BoxError> {
        todo!()
    }
}

pub struct DepotJob {
    // private
}

impl DepotJob {
    pub fn builder() -> DepotJobBuilder {
        DepotJobBuilder::new()
    }

    pub async fn download<F: ChunkFetcher>(
        &self,
        _manifest: &DepotManifest,
        _fetcher: &F,
    ) -> Result<(), BoxError> {
        todo!()
    }
}
