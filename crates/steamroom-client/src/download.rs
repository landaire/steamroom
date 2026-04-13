use crate::event::DownloadEvent;
use bytes::Bytes;
use std::future::Future;
use std::io::Write;
use std::path::Path;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;
use steamroom::cdn::CdnClient;
use steamroom::cdn::pool::CdnServerPool;
use steamroom::depot::ChunkId;
use steamroom::depot::DepotId;
use steamroom::depot::DepotKey;
use steamroom::depot::chunk;
use steamroom::depot::manifest::DepotManifest;
use steamroom::depot::manifest::ManifestFile;
use steamroom::enums::DepotFileFlags;
use steamroom::error::Error as SteamError;
use tokio::sync::mpsc;

pub type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// Trait for fetching raw encrypted chunk bytes. Implement this to provide
/// a custom data source (CDN, local cache, LAN peer, etc.).
pub trait ChunkFetcher: Send + Sync {
    fn fetch_chunk(
        &self,
        depot_id: DepotId,
        chunk_id: &ChunkId,
    ) -> impl Future<Output = Result<Bytes, BoxError>> + Send;
}

/// CDN-backed chunk fetcher with server pool rotation and rate-limit handling.
#[non_exhaustive]
pub struct CdnChunkFetcher {
    pub cdn: CdnClient,
    pub pool: CdnServerPool,
    pub cdn_auth_token: Option<String>,
}

impl CdnChunkFetcher {
    pub fn new(cdn: CdnClient, pool: CdnServerPool, cdn_auth_token: Option<String>) -> Self {
        Self {
            cdn,
            pool,
            cdn_auth_token,
        }
    }
}

impl ChunkFetcher for CdnChunkFetcher {
    async fn fetch_chunk(&self, depot_id: DepotId, chunk_id: &ChunkId) -> Result<Bytes, BoxError> {
        let (server, wait) = self.pool.pick();
        if !wait.is_zero() {
            tracing::warn!(
                server = %server.host,
                wait_secs = wait.as_secs_f32(),
                "all CDN servers in cooldown, waiting"
            );
            tokio::time::sleep(wait).await;
        }
        match self
            .cdn
            .download_chunk(server, depot_id, chunk_id, self.cdn_auth_token.as_deref())
            .await
        {
            Ok(data) => {
                self.pool.report_success(server);
                Ok(data)
            }
            Err(SteamError::CdnStatus {
                status,
                retry_after,
            }) => {
                let ra = retry_after.map(Duration::from_secs);
                if status == reqwest::StatusCode::TOO_MANY_REQUESTS
                    || status == reqwest::StatusCode::SERVICE_UNAVAILABLE
                {
                    tracing::warn!(
                        server = %server.host,
                        status = status.as_u16(),
                        retry_after = retry_after.unwrap_or(0),
                        "CDN rate limited, backing off"
                    );
                } else {
                    tracing::debug!(
                        server = %server.host,
                        status = status.as_u16(),
                        "CDN error"
                    );
                }
                self.pool.report_failure(server, ra);
                Err(Box::new(SteamError::CdnStatus {
                    status,
                    retry_after,
                }))
            }
            Err(e) => {
                tracing::debug!(server = %server.host, error = %e, "CDN request failed");
                self.pool.report_failure(server, None);
                Err(Box::new(e))
            }
        }
    }
}

#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub struct RetryConfig {
    pub max_attempts: u32,
    pub initial_delay: Duration,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_attempts: 5,
            initial_delay: Duration::from_millis(500),
        }
    }
}

/// Controls which manifest files are included in a download.
///
/// ```
/// use steamroom_client::download::FileFilter;
///
/// // Match only .dll files
/// let filter = FileFilter::Regex(regex::Regex::new(r"\.dll$").unwrap());
/// assert!(filter.matches("bin/server.dll"));
/// assert!(!filter.matches("bin/server.exe"));
///
/// // Parse a filelist with mixed literal and regex entries
/// let filter = FileFilter::from_filelist(&[
///     "game/bin/server.dll".into(),
///     "regex:^maps/.*\\.vpk$".into(),
/// ]).unwrap();
/// assert!(filter.matches("game/bin/server.dll"));
/// assert!(filter.matches("maps/de_dust2.vpk"));
/// ```
pub enum FileFilter {
    None,
    Combined(Vec<FileFilterEntry>),
    Regex(regex::Regex),
}

pub enum FileFilterEntry {
    Literal(String),
    Regex(regex::Regex),
}

impl FileFilter {
    /// Convert the filter back into filelist string format.
    /// Regex entries are prefixed with `regex:`.
    pub fn to_filelist(&self) -> Vec<String> {
        match self {
            Self::None => vec![],
            Self::Combined(entries) => entries
                .iter()
                .map(|e| match e {
                    FileFilterEntry::Literal(s) => s.clone(),
                    FileFilterEntry::Regex(re) => format!("regex:{}", re.as_str()),
                })
                .collect(),
            Self::Regex(re) => vec![format!("regex:{}", re.as_str())],
        }
    }

    /// Parse a filelist where lines can be literal paths or `regex:pattern` entries.
    /// This is compatible with the filelist format used by DepotDownloader.
    pub fn from_filelist(lines: &[String]) -> Result<Self, regex::Error> {
        let mut entries = Vec::with_capacity(lines.len());
        for line in lines {
            if let Some(pattern) = line.strip_prefix("regex:") {
                entries.push(FileFilterEntry::Regex(regex::Regex::new(pattern)?));
            } else {
                entries.push(FileFilterEntry::Literal(line.clone()));
            }
        }
        Ok(Self::Combined(entries))
    }

    /// Returns true if `filename` passes the filter.
    /// Literal comparisons are case-insensitive and normalize path separators.
    pub fn matches(&self, filename: &str) -> bool {
        match self {
            Self::None => true,
            Self::Combined(entries) => entries.iter().any(|entry| match entry {
                FileFilterEntry::Literal(lit) => {
                    filename.eq_ignore_ascii_case(lit)
                        || filename.replace('\\', "/").eq_ignore_ascii_case(lit)
                }
                FileFilterEntry::Regex(re) => re.is_match(filename),
            }),
            Self::Regex(re) => re.is_match(filename),
        }
    }
}

#[cfg(feature = "serde")]
impl serde::Serialize for FileFilter {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.to_filelist().serialize(serializer)
    }
}

#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for FileFilter {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let lines: Vec<String> = Vec::deserialize(deserializer)?;
        if lines.is_empty() {
            return Ok(Self::None);
        }
        Self::from_filelist(&lines).map_err(serde::de::Error::custom)
    }
}

/// A download job for a single depot. Handles chunk fetching, decryption,
/// decompression, file assembly, resume, and delta removal of stale files.
///
/// Create via [`DepotJob::builder()`].
pub struct DepotJob {
    depot_id: DepotId,
    depot_key: DepotKey,
    install_dir: PathBuf,
    max_downloads: usize,
    verify: bool,
    non_atomic: bool,
    file_filter: FileFilter,
    retry: RetryConfig,
    event_tx: Option<mpsc::UnboundedSender<DownloadEvent>>,
    old_manifest_files: Option<Vec<String>>,
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
        let (total_bytes, total_files) =
            manifest
                .files
                .iter()
                .fold((0u64, 0u64), |(bytes, count), f| {
                    if self.file_filter.matches(&f.filename) {
                        (bytes + f.size, count + 1)
                    } else {
                        (bytes, count)
                    }
                });
        let mut stats = DownloadStats::default();

        self.emit(DownloadEvent::DownloadStarted {
            total_bytes,
            total_files,
        });

        let sem = std::sync::Arc::new(tokio::sync::Semaphore::new(self.max_downloads));

        for file in &manifest.files {
            let filename = &file.filename;

            if !self.file_filter.matches(filename) {
                self.emit(DownloadEvent::FileSkipped {
                    filename: filename.clone(),
                });
                stats.files_skipped += 1;
                continue;
            }

            let file_path = self.install_dir.join(filename);
            let flags = DepotFileFlags(file.flags);

            if flags.is_directory() {
                std::fs::create_dir_all(&file_path)?;
                continue;
            }

            if file.size == 0 && file.chunks.is_empty() {
                if let Some(parent) = file_path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(&file_path, [])?;
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
            let expected_size = file.size;
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

            let file_size = if self.non_atomic {
                self.download_file_streaming(file, &file_path, &fetcher, &sem)
                    .await?
            } else {
                let staging_dir = self.install_dir.join(".depotdownloader").join("staging");
                std::fs::create_dir_all(&staging_dir)?;
                let staging_path = staging_dir.join(filename.replace(['/', '\\'], "_"));

                let size = self
                    .download_file_streaming(file, &staging_path, &fetcher, &sem)
                    .await?;

                replace_file(&staging_path, &file_path)?;
                size
            };
            stats.bytes_downloaded += file_size;
            stats.files_completed += 1;

            self.emit(DownloadEvent::FileCompleted {
                filename: filename.to_string(),
            });
            self.emit(DownloadEvent::DepotProgress {
                completed_bytes: stats.bytes_downloaded,
                total_bytes,
            });
        }

        // Remove files from the old manifest that are absent in the new one
        if let Some(ref old_files) = self.old_manifest_files {
            let new_files: std::collections::HashSet<&str> =
                manifest.files.iter().map(|f| f.filename.as_str()).collect();

            for old_name in old_files {
                if new_files.contains(old_name.as_str()) {
                    continue;
                }
                let old_path = self.install_dir.join(old_name.replace('\\', "/"));
                if old_path.exists() {
                    let is_dir = old_path.is_dir();
                    let removed = if is_dir {
                        std::fs::remove_dir(&old_path).is_ok()
                    } else {
                        std::fs::remove_file(&old_path).is_ok()
                    };
                    if removed {
                        self.emit(DownloadEvent::FileRemoved {
                            filename: old_name.clone(),
                        });
                        stats.files_removed += 1;
                    }
                }
            }

            // Collect parent dirs of removed files, then prune empty ones bottom-up
            let mut candidate_dirs: Vec<PathBuf> = old_files
                .iter()
                .filter(|name| !new_files.contains(name.as_str()))
                .flat_map(|name| {
                    let mut dirs = vec![];
                    let mut p = Path::new(name).parent();
                    while let Some(d) = p {
                        if d.as_os_str().is_empty() {
                            break;
                        }
                        dirs.push(
                            self.install_dir
                                .join(d.to_str().unwrap_or("").replace('\\', "/")),
                        );
                        p = d.parent();
                    }
                    dirs
                })
                .collect();
            // Sort longest path first so we remove children before parents
            candidate_dirs.sort_by_key(|d| std::cmp::Reverse(d.as_os_str().len()));
            candidate_dirs.dedup();
            let new_parents: std::collections::HashSet<PathBuf> = new_files
                .iter()
                .flat_map(|name| {
                    let mut dirs = vec![];
                    let mut p = Path::new(name).parent();
                    while let Some(d) = p {
                        if d.as_os_str().is_empty() {
                            break;
                        }
                        dirs.push(
                            self.install_dir
                                .join(d.to_str().unwrap_or("").replace('\\', "/")),
                        );
                        p = d.parent();
                    }
                    dirs
                })
                .collect();
            for dir in &candidate_dirs {
                if std::fs::remove_dir(dir).is_err() && !new_parents.contains(dir) {
                    tracing::info!(
                        "kept non-empty directory {} (contains files not in the manifest)",
                        dir.strip_prefix(&self.install_dir).unwrap_or(dir).display()
                    );
                }
            }
        }

        Ok(stats)
    }

    /// Streaming chunk download with delta reuse.
    ///
    /// For each chunk, check if the existing file already has correct data at
    /// that offset (Adler-32 match). Reusable chunks are copied from the existing
    /// file; changed chunks are fetched from CDN, decrypted, and decompressed.
    /// Chunks are written to the output file in order as they complete.
    ///
    /// Memory usage is bounded by `max_downloads × chunk_size` (~16 MB typical).
    async fn download_file_streaming<F: ChunkFetcher + 'static>(
        &self,
        file: &ManifestFile,
        output_path: &Path,
        fetcher: &std::sync::Arc<F>,
        sem: &std::sync::Arc<tokio::sync::Semaphore>,
    ) -> Result<u64, BoxError> {
        use steamroom::util::checksum::SteamAdler32;

        let n = file.chunks.len();
        if n == 0 {
            std::fs::write(output_path, [])?;
            return Ok(0);
        }

        // Read existing file to find reusable chunks (delta optimization)
        let existing = std::fs::read(output_path).unwrap_or_default();
        let mut reuse: Vec<bool> = Vec::with_capacity(n);
        let mut offset: usize = 0;
        for chunk_meta in &file.chunks {
            let size = chunk_meta.uncompressed_size as usize;
            let end = offset + size;
            let ok = end <= existing.len()
                && SteamAdler32::compute(&existing[offset..end]).0 == chunk_meta.checksum;
            reuse.push(ok);
            offset += size;
        }

        let reused = reuse.iter().filter(|&&r| r).count();
        let to_fetch = n - reused;
        if reused > 0 {
            tracing::debug!(
                "{}: reusing {reused}/{n} chunks, fetching {to_fetch}",
                &file.filename,
            );
        }

        // Only fetch chunks that differ
        let slots: std::sync::Arc<Vec<OnceLock<Vec<u8>>>> =
            std::sync::Arc::new((0..n).map(|_| OnceLock::new()).collect());

        let (fetch_tx, mut fetch_rx) =
            tokio::sync::mpsc::channel::<(usize, Bytes, u32, u32)>(self.max_downloads);

        let mut fetch_handles = Vec::with_capacity(to_fetch);
        for (i, chunk_meta) in file.chunks.iter().enumerate() {
            if reuse[i] {
                continue;
            }
            let chunk_id = chunk_meta.id.clone();
            let expected_size = chunk_meta.uncompressed_size;
            let checksum = chunk_meta.checksum;
            let depot_id = self.depot_id;
            let retry = self.retry.clone();
            let event_tx = self.event_tx.clone();
            let sem = sem.clone();
            let fetcher = fetcher.clone();
            let fetch_tx = fetch_tx.clone();

            fetch_handles.push(tokio::spawn(async move {
                let _permit = sem
                    .acquire()
                    .await
                    .map_err(|e| -> BoxError { Box::new(e) })?;

                let mut delay = retry.initial_delay;
                let mut result = Err::<Bytes, BoxError>("never attempted".into());
                for attempt in 0..retry.max_attempts {
                    match fetcher.fetch_chunk(depot_id, &chunk_id).await {
                        Ok(data) => {
                            result = Ok(data);
                            break;
                        }
                        Err(e) if attempt + 1 < retry.max_attempts => {
                            let wait = retry_delay_for_error(&e, delay);
                            if let Some(ref tx) = event_tx {
                                let _ = tx.send(DownloadEvent::ChunkFailed {
                                    error: e.to_string(),
                                });
                            }
                            tokio::time::sleep(wait).await;
                            delay = (wait * 2).min(Duration::from_secs(30));
                        }
                        Err(e) => {
                            result = Err(e);
                            break;
                        }
                    }
                }

                fetch_tx
                    .send((i, result?, expected_size, checksum))
                    .await
                    .map_err(|_| -> BoxError { "process channel closed".into() })?;
                Ok::<(), BoxError>(())
            }));
        }
        drop(fetch_tx);

        // Decrypt + decompress fetched chunks into slots
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
                        let _ = tx.send(DownloadEvent::ChunkCompleted {
                            bytes: processed.len() as u64,
                        });
                    }
                    let _ = slots[i].set(processed);
                    Ok::<(), BoxError>(())
                }));
            }
            for h in block_handles {
                h.await??;
            }
            Ok::<(), BoxError>(())
        });

        for h in fetch_handles {
            h.await??;
        }
        process_handle.await??;

        // Write output: reused chunks from existing data, fetched chunks from slots
        let slots = std::sync::Arc::try_unwrap(slots).map_err(|_| "slots arc still shared")?;
        let mut out = std::fs::File::create(output_path)?;
        let mut written: u64 = 0;
        let mut read_offset: usize = 0;
        for (i, chunk_meta) in file.chunks.iter().enumerate() {
            let size = chunk_meta.uncompressed_size as usize;
            if reuse[i] {
                out.write_all(&existing[read_offset..read_offset + size])?;
                self.emit(DownloadEvent::ChunkCompleted { bytes: size as u64 });
            } else {
                let data = slots[i].get().ok_or("chunk slot empty after pipeline")?;
                out.write_all(data)?;
            }
            read_offset += size;
            written += size as u64;
        }

        Ok(written)
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

fn replace_file(src: &Path, dst: &Path) -> Result<(), std::io::Error> {
    if dst.exists() {
        #[cfg(windows)]
        {
            let mut perms = std::fs::metadata(dst)?.permissions();
            #[allow(clippy::permissions_set_readonly_false)]
            if perms.readonly() {
                perms.set_readonly(false);
                let _ = std::fs::set_permissions(dst, perms);
            }
        }
        std::fs::remove_file(dst)?;
    }
    std::fs::rename(src, dst)
}

#[derive(Default, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[non_exhaustive]
pub struct DownloadStats {
    pub files_completed: u64,
    pub files_skipped: u64,
    pub files_removed: u64,
    pub bytes_downloaded: u64,
}

#[derive(Default)]
pub struct DepotJobBuilder {
    depot_id: Option<DepotId>,
    depot_key: Option<DepotKey>,
    install_dir: Option<PathBuf>,
    max_downloads: Option<usize>,
    verify: bool,
    non_atomic: bool,
    file_filter: Option<FileFilter>,
    retry: Option<RetryConfig>,
    event_tx: Option<mpsc::UnboundedSender<DownloadEvent>>,
    old_manifest_files: Option<Vec<String>>,
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

    pub fn non_atomic(mut self, v: bool) -> Self {
        self.non_atomic = v;
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

    pub fn old_manifest_files(mut self, files: Vec<String>) -> Self {
        self.old_manifest_files = Some(files);
        self
    }

    pub fn build(self) -> Result<DepotJob, BoxError> {
        Ok(DepotJob {
            depot_id: self.depot_id.ok_or("depot_id required")?,
            depot_key: self.depot_key.ok_or("depot_key required")?,
            install_dir: self.install_dir.ok_or("install_dir required")?,
            max_downloads: self.max_downloads.unwrap_or(16),
            verify: self.verify,
            non_atomic: self.non_atomic,
            file_filter: self.file_filter.unwrap_or(FileFilter::None),
            retry: self.retry.unwrap_or_default(),
            event_tx: self.event_tx,
            old_manifest_files: self.old_manifest_files,
        })
    }
}

/// Compute retry delay, respecting `Retry-After` from 429/503 responses.
fn retry_delay_for_error(err: &BoxError, default: Duration) -> Duration {
    if let Some(SteamError::CdnStatus {
        status,
        retry_after,
    }) = err.downcast_ref::<SteamError>()
        && (*status == reqwest::StatusCode::TOO_MANY_REQUESTS
            || *status == reqwest::StatusCode::SERVICE_UNAVAILABLE)
    {
        if let Some(secs) = retry_after {
            return Duration::from_secs((*secs).min(60));
        }
        return default.max(Duration::from_secs(5));
    }
    default
}
