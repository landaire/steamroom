use std::future::Future;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;
use bytes::Bytes;
use tokio::sync::mpsc;
use steamroom::cdn::CdnClient;
use steamroom::cdn::server::CdnServer;
use steamroom::depot::chunk::{self, ChunkError};
use steamroom::depot::manifest::{DepotManifest, ManifestFile};
use steamroom::depot::{ChunkId, DepotId, DepotKey};
use steamroom::enums::DepotFileFlags;
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
        depot_id: DepotId,
        chunk_id: &ChunkId,
    ) -> Result<Bytes, BoxError> {
        Ok(self
            .cdn
            .download_chunk(&self.server, depot_id, chunk_id, self.cdn_auth_token.as_deref())
            .await?)
    }
}

#[derive(Clone, Debug)]
pub struct RetryConfig {
    pub max_attempts: u32,
    pub initial_delay: Duration,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_attempts: 3,
            initial_delay: Duration::from_secs(1),
        }
    }
}

pub enum FileFilter {
    None,
    FileList(Vec<String>),
    Regex(regex::Regex),
}

impl FileFilter {
    pub fn matches(&self, filename: &str) -> bool {
        match self {
            Self::None => true,
            Self::FileList(list) => list.iter().any(|f| {
                filename.eq_ignore_ascii_case(f)
                    || filename.replace('\\', "/").eq_ignore_ascii_case(f)
            }),
            Self::Regex(re) => re.is_match(filename),
        }
    }
}

pub struct DepotJob {
    depot_id: DepotId,
    depot_key: DepotKey,
    install_dir: PathBuf,
    max_downloads: usize,
    verify: bool,
    file_filter: FileFilter,
    retry: RetryConfig,
    event_tx: Option<mpsc::UnboundedSender<DownloadEvent>>,
}

impl DepotJob {
    pub fn builder() -> DepotJobBuilder {
        DepotJobBuilder::default()
    }

    fn emit(&self, event: DownloadEvent) {
        if let Some(ref tx) = self.event_tx {
            let _ = tx.send(event);
        }
    }

    pub async fn download<F: ChunkFetcher + 'static>(
        &self,
        manifest: &DepotManifest,
        fetcher: std::sync::Arc<F>,
    ) -> Result<DownloadStats, BoxError> {
        let total_bytes: u64 = manifest.files.iter().filter_map(|f| f.size).sum();
        let mut stats = DownloadStats::default();

        let sem = std::sync::Arc::new(tokio::sync::Semaphore::new(self.max_downloads));

        for file in &manifest.files {
            let filename = file.filename.as_deref().unwrap_or("(unknown)");

            if !self.file_filter.matches(filename) {
                self.emit(DownloadEvent::FileSkipped {
                    filename: filename.to_string(),
                });
                stats.files_skipped += 1;
                continue;
            }

            let file_path = self.install_dir.join(filename);
            // proto2 optional: absent flags means normal file (no special attributes)
            let flags = DepotFileFlags(file.flags.unwrap_or(0));

            if flags.is_directory() {
                std::fs::create_dir_all(&file_path)?;
                continue;
            }

            if file.size.unwrap_or(0) == 0 && file.chunks.is_empty() {
                if let Some(parent) = file_path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(&file_path, &[])?;
                stats.files_completed += 1;
                continue;
            }

            if file.link_target.is_some() {
                // Symlinks — skip for now
                continue;
            }

            if let Some(parent) = file_path.parent() {
                std::fs::create_dir_all(parent)?;
            }

            // Check if file already matches the manifest (skip if up-to-date)
            let expected_size = file.size.unwrap_or(0);
            if self.verify && file_matches(&file_path, expected_size, file.sha_content.as_ref()) {
                self.emit(DownloadEvent::FileSkipped {
                    filename: filename.to_string(),
                });
                stats.files_skipped += 1;
                stats.bytes_downloaded += expected_size;
                continue;
            }

            self.emit(DownloadEvent::FileStarted {
                filename: filename.to_string(),
            });

            // Download to staging, then move to final path
            let staging_dir = self.install_dir.join(".depotdownloader").join("staging");
            std::fs::create_dir_all(&staging_dir)?;
            let staging_path = staging_dir.join(filename.replace(['/', '\\'], "_"));

            let file_data =
                self.download_file_chunks_with_resume(file, &fetcher, &sem, &staging_path).await?;

            std::fs::write(&file_path, &file_data)?;
            let _ = std::fs::remove_file(&staging_path);
            stats.bytes_downloaded += file_data.len() as u64;
            stats.files_completed += 1;

            self.emit(DownloadEvent::FileCompleted {
                filename: filename.to_string(),
            });
            self.emit(DownloadEvent::DepotProgress {
                completed_bytes: stats.bytes_downloaded,
                total_bytes,
            });
        }

        Ok(stats)
    }

    /// Pipelined chunk download: network fetch and decrypt/decompress overlap.
    ///
    /// Stage 1 (async IO, bounded by semaphore): fetch raw bytes from CDN
    /// Stage 2 (blocking thread pool): decrypt + decompress + checksum verify
    ///
    /// Fetchers push raw bytes into a bounded channel. A processor task drains
    /// the channel and dispatches each chunk to spawn_blocking. Results land in
    /// ordered slots. The bounded channel provides backpressure: if the CPU pool
    /// falls behind, fetchers block on send instead of buffering unbounded memory.
    async fn download_file_chunks<F: ChunkFetcher + 'static>(
        &self,
        file: &ManifestFile,
        fetcher: &std::sync::Arc<F>,
        sem: &std::sync::Arc<tokio::sync::Semaphore>,
    ) -> Result<Vec<u8>, BoxError> {
        let n = file.chunks.len();
        if n == 0 {
            return Ok(Vec::new());
        }

        // Bounded channel: fetch stage → process stage.
        // Capacity = max_downloads so we buffer at most that many fetched-but-unprocessed chunks.
        let (fetch_tx, mut fetch_rx) =
            tokio::sync::mpsc::channel::<(usize, Bytes, u32, u32)>(self.max_downloads);

        let slots: std::sync::Arc<std::sync::Mutex<Vec<Option<Vec<u8>>>>> =
            std::sync::Arc::new(std::sync::Mutex::new(vec![None; n]));

        // Stage 1: spawn fetcher tasks
        let mut fetch_handles = Vec::with_capacity(n);
        for (i, chunk_meta) in file.chunks.iter().enumerate() {
            let chunk_id = chunk_meta.id.clone().ok_or("chunk missing chunk ID")?;
            let expected_size = chunk_meta.uncompressed_size.ok_or("chunk missing uncompressed_size")?;
            let checksum = chunk_meta.checksum.ok_or("chunk missing checksum")?;
            let depot_id = self.depot_id;
            let retry = self.retry.clone();
            let event_tx = self.event_tx.clone();
            let sem = sem.clone();
            let fetcher = fetcher.clone();
            let fetch_tx = fetch_tx.clone();

            fetch_handles.push(tokio::spawn(async move {
                let _permit = sem.acquire().await.map_err(|e| -> BoxError { Box::new(e) })?;

                let mut delay = retry.initial_delay;
                let mut result = Err::<Bytes, BoxError>("never attempted".into());
                for attempt in 0..retry.max_attempts {
                    match fetcher.fetch_chunk(depot_id, &chunk_id).await {
                        Ok(data) => { result = Ok(data); break; }
                        Err(e) if attempt + 1 < retry.max_attempts => {
                            if let Some(ref tx) = event_tx {
                                let _ = tx.send(DownloadEvent::ChunkFailed { error: e.to_string() });
                            }
                            tokio::time::sleep(delay).await;
                            delay *= 2;
                        }
                        Err(e) => { result = Err(e); break; }
                    }
                }

                // Backpressure: if process stage is full, this blocks the fetcher
                // (which releases the semaphore permit, letting other fetchers proceed)
                fetch_tx.send((i, result?, expected_size, checksum)).await
                    .map_err(|_| -> BoxError { "process channel closed".into() })?;
                Ok::<(), BoxError>(())
            }));
        }
        drop(fetch_tx); // close so process loop terminates when all fetchers done

        // Stage 2: drain fetch results → spawn_blocking for decrypt+decompress
        let slots_ref = slots.clone();
        let depot_key = self.depot_key.clone();
        let event_tx = self.event_tx.clone();

        let process_handle = tokio::spawn(async move {
            let mut block_handles = Vec::new();

            while let Some((i, raw, expected_size, checksum)) = fetch_rx.recv().await {
                let key = depot_key.clone();
                let slots = slots_ref.clone();
                let tx = event_tx.clone();

                block_handles.push(tokio::task::spawn_blocking(move || {
                    let processed = chunk::process_chunk(&raw, &key, expected_size, checksum)?;
                    if let Some(ref tx) = tx {
                        let _ = tx.send(DownloadEvent::ChunkCompleted { bytes: processed.len() as u64 });
                    }
                    slots.lock().unwrap()[i] = Some(processed);
                    Ok::<(), BoxError>(())
                }));
            }

            for h in block_handles {
                h.await??;
            }
            Ok::<(), BoxError>(())
        });

        // Wait for both stages
        for h in fetch_handles {
            h.await??;
        }
        process_handle.await??;

        // Assemble in order
        let slots = std::sync::Arc::try_unwrap(slots)
            .map_err(|_| "slots arc still shared")?
            .into_inner()
            .unwrap();
        // size hint only — Vec grows if absent, no correctness impact
        let mut file_data = Vec::with_capacity(file.size.unwrap_or(0) as usize);
        for slot in slots {
            file_data.extend_from_slice(&slot.ok_or("chunk slot empty after pipeline")?);
        }
        Ok(file_data)
    }

    async fn fetch_with_retry<F: ChunkFetcher>(
        &self,
        fetcher: &F,
        chunk_id: &ChunkId,
    ) -> Result<Bytes, BoxError> {
        let mut delay = self.retry.initial_delay;
        for attempt in 0..self.retry.max_attempts {
            match fetcher.fetch_chunk(self.depot_id, chunk_id).await {
                Ok(data) => return Ok(data),
                Err(e) if attempt + 1 < self.retry.max_attempts => {
                    self.emit(DownloadEvent::ChunkFailed {
                        error: e.to_string(),
                    });
                    tokio::time::sleep(delay).await;
                    delay *= 2;
                }
                Err(e) => return Err(e),
            }
        }
        unreachable!()
    }

    async fn download_file_chunks_with_resume<F: ChunkFetcher + 'static>(
        &self,
        file: &ManifestFile,
        fetcher: &std::sync::Arc<F>,
        sem: &std::sync::Arc<tokio::sync::Semaphore>,
        staging_path: &Path,
    ) -> Result<Vec<u8>, BoxError> {
        let existing_bytes = std::fs::metadata(staging_path)
            .map(|m| m.len())
            .unwrap_or(0);

        // Count complete chunks already staged
        let mut staged_offset: u64 = 0;
        let mut skip_count = 0;
        if existing_bytes > 0 {
            for chunk_meta in &file.chunks {
                let chunk_size = chunk_meta.uncompressed_size.unwrap_or(0) as u64;
                if staged_offset + chunk_size <= existing_bytes {
                    staged_offset += chunk_size;
                    skip_count += 1;
                } else {
                    break;
                }
            }
        }

        if skip_count == file.chunks.len() {
            return Ok(std::fs::read(staging_path)?);
        }

        if skip_count > 0 {
            tracing::debug!(
                "resuming {}: skipping {skip_count}/{} chunks ({staged_offset} bytes staged)",
                file.filename.as_deref().unwrap_or("?"),
                file.chunks.len(),
            );
        } else {
            let _ = std::fs::remove_file(staging_path);
        }

        // Build a trimmed file with only remaining chunks, pipeline-download them
        let remaining = ManifestFile {
            filename: file.filename.clone(),
            size: file.size.map(|s| s - staged_offset),
            flags: file.flags,
            sha_content: file.sha_content,
            chunks: file.chunks[skip_count..].to_vec(),
            link_target: None,
        };

        let new_data = self.download_file_chunks(&remaining, fetcher, sem).await?;

        // Append to staging for crash safety
        {
            let mut f = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(staging_path)?;
            f.write_all(&new_data)?;
        }

        // Assemble: existing staged data + newly downloaded
        if skip_count > 0 {
            let mut full = std::fs::read(staging_path)?;
            full.truncate((staged_offset as usize) + new_data.len());
            Ok(full)
        } else {
            Ok(new_data)
        }
    }
}

fn file_matches(path: &Path, expected_size: u64, sha_content: Option<&[u8; 20]>) -> bool {
    let meta = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(_) => return false,
    };
    if meta.len() != expected_size {
        return false;
    }
    if let Some(expected_sha) = sha_content {
        if let Ok(data) = std::fs::read(path) {
            let actual = steamroom::util::checksum::Sha1Hash::compute(&data);
            return actual.0 == *expected_sha;
        }
        return false;
    }
    // No SHA to verify — size match is good enough
    true
}

#[derive(Default, Debug)]
pub struct DownloadStats {
    pub files_completed: u64,
    pub files_skipped: u64,
    pub bytes_downloaded: u64,
}

#[derive(Default)]
pub struct DepotJobBuilder {
    depot_id: Option<DepotId>,
    depot_key: Option<DepotKey>,
    install_dir: Option<PathBuf>,
    max_downloads: Option<usize>,
    verify: bool,
    file_filter: Option<FileFilter>,
    retry: Option<RetryConfig>,
    event_tx: Option<mpsc::UnboundedSender<DownloadEvent>>,
}

impl DepotJobBuilder {
    pub fn depot_id(mut self, id: DepotId) -> Self {
        self.depot_id = Some(id);
        self
    }

    pub fn depot_key(mut self, key: DepotKey) -> Self {
        self.depot_key = Some(key);
        self
    }

    pub fn install_dir(mut self, dir: PathBuf) -> Self {
        self.install_dir = Some(dir);
        self
    }

    pub fn max_downloads(mut self, n: usize) -> Self {
        self.max_downloads = Some(n);
        self
    }

    pub fn file_filter(mut self, f: FileFilter) -> Self {
        self.file_filter = Some(f);
        self
    }

    pub fn verify(mut self, v: bool) -> Self {
        self.verify = v;
        self
    }

    pub fn retry(mut self, config: RetryConfig) -> Self {
        self.retry = Some(config);
        self
    }

    pub fn event_sender(mut self, tx: mpsc::UnboundedSender<DownloadEvent>) -> Self {
        self.event_tx = Some(tx);
        self
    }

    pub fn build(self) -> Result<DepotJob, BoxError> {
        Ok(DepotJob {
            depot_id: self.depot_id.ok_or("depot_id required")?,
            depot_key: self.depot_key.ok_or("depot_key required")?,
            install_dir: self.install_dir.ok_or("install_dir required")?,
            max_downloads: self.max_downloads.unwrap_or(8),
            verify: self.verify,
            file_filter: self.file_filter.unwrap_or(FileFilter::None),
            retry: self.retry.unwrap_or_default(),
            event_tx: self.event_tx,
        })
    }
}
