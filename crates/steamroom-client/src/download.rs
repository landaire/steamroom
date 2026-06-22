use crate::event::DownloadEvent;
use crate::event::ErrorChain;
use bytes::Bytes;
use rootcause::Report;
use rootcause::markers::Mutable;
use rootcause::markers::SendSync;
use std::fs::File;
use std::future::Future;
use std::io;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use steamroom::cdn::CdnClient;
use steamroom::cdn::pool::CdnServerPool;
use steamroom::depot::ChunkId;
use steamroom::depot::DepotId;
use steamroom::depot::DepotKey;
use steamroom::depot::chunk;
use steamroom::depot::manifest::DepotManifest;
use steamroom::depot::manifest::ManifestChunk;
use steamroom::depot::manifest::ManifestFile;
use steamroom::enums::DepotFileFlags;
use steamroom::error::Error as SteamError;
use tokio::sync::mpsc;

pub type BoxError = Box<dyn std::error::Error + Send + Sync>;

/// Owning, mutable, thread-safe `rootcause` report carrying a [`DownloadError`]
/// at its root.
pub type DownloadReport = Report<DownloadError, Mutable, SendSync>;

/// Root error variants for the download pipeline. Contextual data (depot id,
/// chunk id, filename, byte offset, retry chain) is attached to the surrounding
/// [`Report`] rather than baked into each variant, so the variants stay focused
/// on *what* failed and the report carries *where* and *while doing what*.
///
/// `Fetch` wraps the opaque `BoxError` returned by [`ChunkFetcher::fetch_chunk`]:
/// the trait is generic over user-supplied data sources, so the inner error
/// remains untyped at this layer. Every other failure mode is named.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum DownloadError {
    #[error("io: {0}")]
    Io(#[from] io::Error),

    #[error("chunk: {0}")]
    Chunk(#[from] chunk::ChunkError),

    #[error("fetch chunk")]
    Fetch {
        #[source]
        source: BoxError,
    },

    #[error("assembled file failed SHA-1 verification")]
    Sha1Mismatch {
        expected: [u8; 20],
        actual: [u8; 20],
    },

    #[error("background task: {0}")]
    Join(#[from] tokio::task::JoinError),

    #[error("semaphore closed: {0}")]
    Acquire(#[from] tokio::sync::AcquireError),
}

/// Wrap any error that converts into [`DownloadError`] as a fresh
/// [`DownloadReport`]. The orphan rule prevents blanket `From<io::Error> for
/// Report<DownloadError, ..>` impls (both sides are foreign), and `?` only
/// applies a single `From` step, so callers funnel through this helper -
/// typically `.map_err(report)?` or `.map_err(|e| report(e).attach(ctx))?`.
fn report<E: Into<DownloadError>>(e: E) -> DownloadReport {
    Report::new(e.into())
}

/// Failures from [`DepotJobBuilder::build`]. Each variant names a single
/// missing required input so callers do not have to inspect a stringified
/// message to react.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum BuildError {
    #[error("depot_id is required")]
    MissingDepotId,
    #[error("depot_key is required")]
    MissingDepotKey,
    #[error("install_dir is required")]
    MissingInstallDir,
}

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
                    let norm_file = filename.replace('\\', "/");
                    let norm_lit = lit.replace('\\', "/");
                    norm_file.eq_ignore_ascii_case(&norm_lit)
                }
                FileFilterEntry::Regex(re) => {
                    re.is_match(filename) || re.is_match(&filename.replace('\\', "/"))
                }
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
    ) -> Result<DownloadStats, DownloadReport> {
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

            let file_path = self.install_dir.join(file.normalized_path());
            let flags = DepotFileFlags::from_bits_retain(file.flags);

            let attach_file = || format!("file `{}` (size {} bytes)", file.filename, file.size);

            if flags.is_directory() {
                std::fs::create_dir_all(&file_path).map_err(|e| report(e).attach(attach_file()))?;
                continue;
            }

            if file.size == 0 && file.chunks.is_empty() {
                if self.verify && file_path.exists() {
                    self.emit(DownloadEvent::FileSkipped {
                        filename: filename.to_string(),
                    });
                    stats.files_skipped += 1;
                    continue;
                }

                if let Some(parent) = file_path.parent() {
                    std::fs::create_dir_all(parent).map_err(|e| report(e).attach(attach_file()))?;
                }
                
                std::fs::write(&file_path, []).map_err(|e| report(e).attach(attach_file()))?;
                stats.files_completed += 1;
                continue;
            }

            if file.link_target.is_some() {
                // Symlinks — skip for now
                continue;
            }

            if let Some(parent) = file_path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| report(e).attach(attach_file()))?;
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
                std::fs::create_dir_all(&staging_dir)
                    .map_err(|e| report(e).attach(attach_file()))?;
                let staging_path = staging_dir.join(filename.replace(['/', '\\'], "_"));

                let size = self
                    .download_file_streaming(file, &staging_path, &fetcher, &sem)
                    .await?;

                replace_file(&staging_path, &file_path)
                    .map_err(|e| report(e).attach(attach_file()))?;
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
            let new_files: std::collections::HashSet<String> =
                manifest.files.iter().map(|f| f.normalized_path()).collect();

            for old_name in old_files {
                if new_files.contains(old_name) {
                    continue;
                }
                let old_path = self.install_dir.join(old_name);
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
                .flat_map(|name| parent_dirs(&self.install_dir, name))
                .collect();
            candidate_dirs.sort_by_key(|d| std::cmp::Reverse(d.as_os_str().len()));
            candidate_dirs.dedup();
            let new_parents: std::collections::HashSet<PathBuf> = new_files
                .iter()
                .flat_map(|name| parent_dirs(&self.install_dir, name))
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
    /// Pre-allocates the output file via `set_len(file.size)`, then writes each
    /// chunk at its known offset as soon as it has been fetched, decrypted, and
    /// decompressed. Reusable chunks (existing bytes whose Adler-32 already
    /// matches the manifest) are left in place; only differing chunks hit the
    /// network. Out-of-order completions are fine because writes are positional
    /// (`pwrite` / `seek_write`).
    ///
    /// Memory is bounded by `max_downloads * (encrypted + decompressed chunk
    /// size)` plus one reusable scratch buffer. The full file is never resident.
    async fn download_file_streaming<F: ChunkFetcher + 'static>(
        &self,
        file: &ManifestFile,
        output_path: &Path,
        fetcher: &std::sync::Arc<F>,
        sem: &std::sync::Arc<tokio::sync::Semaphore>,
    ) -> Result<u64, DownloadReport> {
        let n = file.chunks.len();
        let attach_file_ctx = || {
            format!(
                "file `{}` (size {} bytes, {n} chunks)",
                file.filename, file.size
            )
        };
        if n == 0 {
            std::fs::write(output_path, []).map_err(|e| report(e).attach(attach_file_ctx()))?;
            return Ok(0);
        }

        // chunk.offset is the authoritative position from the protobuf; fall
        // back to a running cumulative sum for manifests that omit it.
        let mut offsets: Vec<u64> = Vec::with_capacity(n);
        {
            let mut running: u64 = 0;
            for chunk_meta in &file.chunks {
                let off = chunk_meta.offset.unwrap_or(running);
                offsets.push(off);
                running = off + u64::from(chunk_meta.uncompressed_size);
            }
        }

        let out = std::fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(output_path)
            .map_err(|e| report(e).attach(attach_file_ctx()))?;
        let original_len = out
            .metadata()
            .map_err(|e| report(e).attach(attach_file_ctx()))?
            .len();
        out.set_len(file.size)
            .map_err(|e| report(e).attach(attach_file_ctx()))?;
        let out = Arc::new(out);

        let reuse = compute_reuse_mask(&out, &file.chunks, &offsets, original_len)
            .map_err(|r| r.attach(attach_file_ctx()))?;

        let reused = reuse.iter().filter(|&&r| r).count();
        let to_fetch = n - reused;
        if reused > 0 {
            tracing::debug!(
                "{}: reusing {reused}/{n} chunks, fetching {to_fetch}",
                &file.filename,
            );
        }
        for (i, chunk_meta) in file.chunks.iter().enumerate() {
            if reuse[i] {
                self.emit(DownloadEvent::ChunkCompleted {
                    bytes: u64::from(chunk_meta.uncompressed_size),
                });
            }
        }

        let mut fetch_handles = Vec::with_capacity(to_fetch);
        for (i, chunk_meta) in file.chunks.iter().enumerate() {
            if reuse[i] {
                continue;
            }
            let chunk_id = chunk_meta.id.clone();
            let expected_size = chunk_meta.uncompressed_size;
            let checksum = chunk_meta.checksum;
            let chunk_offset = offsets[i];
            let depot_id = self.depot_id;
            let depot_key = self.depot_key.clone();
            let retry = self.retry.clone();
            let event_tx = self.event_tx.clone();
            let sem = sem.clone();
            let fetcher = fetcher.clone();
            let out = out.clone();

            fetch_handles.push(tokio::spawn(async move {
                let attach_chunk = || {
                    format!(
                        "chunk {chunk_id} at offset {chunk_offset} ({expected_size} bytes) of depot {}",
                        depot_id.0
                    )
                };
                let _permit = sem
                    .acquire()
                    .await
                    .map_err(|e| report(e).attach(attach_chunk()))?;

                let raw = fetch_with_retry(
                    fetcher.as_ref(),
                    depot_id,
                    &chunk_id,
                    &retry,
                    event_tx.as_ref(),
                )
                .await
                .map_err(|r| r.attach(attach_chunk()))?;

                let attach_chunk_for_blocking = attach_chunk();
                let attach_chunk_in_blocking = attach_chunk_for_blocking.clone();
                tokio::task::spawn_blocking(move || -> Result<(), DownloadReport> {
                    let processed = chunk::process_chunk(&raw, &depot_key, expected_size, checksum)
                        .map_err(|e| report(e).attach(attach_chunk_in_blocking.clone()))?;
                    let written = processed.len() as u64;
                    pwrite_all(out.as_ref(), &processed, chunk_offset)
                        .map_err(|e| report(e).attach(attach_chunk_in_blocking.clone()))?;
                    drop(processed);
                    if let Some(ref tx) = event_tx {
                        let _ = tx.send(DownloadEvent::ChunkCompleted { bytes: written });
                    }
                    Ok(())
                })
                .await
                .map_err(|e| report(e).attach(attach_chunk_for_blocking.clone()))?
                .map_err(|r| r.attach(attach_chunk_for_blocking))?;

                Ok::<(), DownloadReport>(())
            }));
        }

        let mut first_err: Option<DownloadReport> = None;
        for h in fetch_handles {
            match h.await {
                Ok(Ok(())) => {}
                Ok(Err(e)) => {
                    if first_err.is_none() {
                        first_err = Some(e);
                    }
                }
                Err(e) => {
                    if first_err.is_none() {
                        first_err = Some(report(e));
                    }
                }
            }
        }
        if let Some(e) = first_err {
            return Err(e.attach(attach_file_ctx()));
        }

        // Stream-verify SHA-1 against the manifest. Reused chunks pass only the
        // weak Adler-32 gate, so a torn or partial existing file can slip
        // corrupt bytes through; without the whole-file check the bad package
        // would be accepted silently and only surface at extraction time.
        if let Some(expected_sha) = file.sha_content.as_ref() {
            let file_size = file.size;
            let out_for_hash = out.clone();
            let expected = *expected_sha;
            let actual = tokio::task::spawn_blocking(move || -> Result<[u8; 20], io::Error> {
                use sha1::Digest;
                let mut hasher = sha1::Sha1::new();
                let mut buf = vec![0u8; HASH_READ_BUFFER];
                let mut pos: u64 = 0;
                while pos < file_size {
                    let want = (file_size - pos).min(buf.len() as u64) as usize;
                    pread_exact(&out_for_hash, &mut buf[..want], pos)?;
                    hasher.update(&buf[..want]);
                    pos += want as u64;
                }
                Ok(hasher.finalize().into())
            })
            .await
            .map_err(|e| report(e).attach(attach_file_ctx()))?
            .map_err(|e| report(e).attach(attach_file_ctx()))?;
            if actual != expected {
                // Drop our handle so Windows lets us unlink the bad output.
                drop(out);
                let _ = std::fs::remove_file(output_path);
                return Err(
                    Report::new(DownloadError::Sha1Mismatch { expected, actual })
                        .attach(attach_file_ctx()),
                );
            }
        }

        Ok(file.size)
    }
}

/// Buffer size for the streaming SHA-1 verification pass. Sized to amortize
/// syscall cost without holding meaningful memory.
const HASH_READ_BUFFER: usize = 1 << 20;

/// Buffer size cap for the Adler-32 reuse check. Real Steam chunks are ~1 MiB;
/// the cap exists only as a guard against pathologically large manifests.
const REUSE_BUFFER_CAP: usize = 4 << 20;

/// Decide which chunks can be reused from the existing on-disk bytes.
///
/// `original_len` is the file size before `set_len`, so chunks that fall in
/// the zero-extended tail are never considered reusable.
fn compute_reuse_mask(
    file: &Arc<File>,
    chunks: &[ManifestChunk],
    offsets: &[u64],
    original_len: u64,
) -> Result<Vec<bool>, DownloadReport> {
    use steamroom::util::checksum::SteamAdler32;

    let mut reuse = Vec::with_capacity(chunks.len());
    let cap = chunks
        .iter()
        .map(|c| c.uncompressed_size as usize)
        .max()
        .unwrap_or(0)
        .min(REUSE_BUFFER_CAP);
    let mut buf = vec![0u8; cap];

    for (chunk_meta, &offset) in chunks.iter().zip(offsets.iter()) {
        let size = chunk_meta.uncompressed_size as usize;
        let end = offset.saturating_add(size as u64);
        if size == 0 || end > original_len || size > buf.len() {
            reuse.push(false);
            continue;
        }
        match pread_exact(file, &mut buf[..size], offset) {
            Ok(()) => {
                let matches = SteamAdler32::compute(&buf[..size]).0 == chunk_meta.checksum;
                reuse.push(matches);
            }
            Err(_) => reuse.push(false),
        }
    }
    Ok(reuse)
}

async fn fetch_with_retry<F: ChunkFetcher>(
    fetcher: &F,
    depot_id: DepotId,
    chunk_id: &ChunkId,
    retry: &RetryConfig,
    event_tx: Option<&mpsc::UnboundedSender<DownloadEvent>>,
) -> Result<Bytes, DownloadReport> {
    // Guarantee at least one attempt; a zero-budget config is a misconfiguration
    // but here it would degenerate to "never tried", which is hard to debug.
    let attempts = retry.max_attempts.max(1);
    let mut delay = retry.initial_delay;
    let mut prior: Vec<DownloadReport> = Vec::new();
    for attempt in 1..=attempts {
        match fetcher.fetch_chunk(depot_id, chunk_id).await {
            Ok(data) => return Ok(data),
            Err(source) if attempt < attempts => {
                let wait = retry_delay_for_error(&source, delay);
                if let Some(tx) = event_tx {
                    let _ = tx.send(DownloadEvent::ChunkFailed {
                        error: ErrorChain::from_error(&*source),
                    });
                }
                let attempt_report = Report::new(DownloadError::Fetch { source })
                    .attach(format!("attempt {attempt} of {attempts}"));
                prior.push(attempt_report);
                tokio::time::sleep(wait).await;
                delay = (wait * 2).min(Duration::from_secs(30));
            }
            Err(source) => {
                let mut final_report = Report::new(DownloadError::Fetch { source })
                    .attach(format!("attempt {attempt} of {attempts}"));
                let children = final_report.children_mut();
                for p in prior {
                    children.push(p.into_dynamic().into_cloneable());
                }
                return Err(final_report);
            }
        }
    }
    // Loop above returns on `attempt == attempts` either Ok or via the
    // final `Err` arm; reaching here would mean attempts == 0, which the
    // `.max(1)` above forbids.
    unreachable!("retry loop must terminate within attempts iterations")
}

fn pread_exact(file: &File, buf: &mut [u8], offset: u64) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::FileExt;
        file.read_exact_at(buf, offset)
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::FileExt;
        let mut total = 0;
        while total < buf.len() {
            let n = file.seek_read(&mut buf[total..], offset + total as u64)?;
            if n == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "failed to fill buffer",
                ));
            }
            total += n;
        }
        Ok(())
    }
    #[cfg(not(any(unix, windows)))]
    compile_error!("pread_exact requires unix or windows");
}

fn pwrite_all(file: &File, buf: &[u8], offset: u64) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::FileExt;
        file.write_all_at(buf, offset)
    }
    #[cfg(windows)]
    {
        use std::os::windows::fs::FileExt;
        let mut total = 0;
        while total < buf.len() {
            let n = file.seek_write(&buf[total..], offset + total as u64)?;
            if n == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "failed to write whole buffer",
                ));
            }
            total += n;
        }
        Ok(())
    }
    #[cfg(not(any(unix, windows)))]
    compile_error!("pwrite_all requires unix or windows");
}

/// Collect all parent directories of a normalized path, joined to install_dir.
fn parent_dirs(install_dir: &Path, name: &str) -> Vec<PathBuf> {
    let mut dirs = vec![];
    let mut p = Path::new(name).parent();
    while let Some(d) = p {
        if d.as_os_str().is_empty() {
            break;
        }
        dirs.push(install_dir.join(d));
        p = d.parent();
    }
    dirs
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

    pub fn build(self) -> Result<DepotJob, BuildError> {
        Ok(DepotJob {
            depot_id: self.depot_id.ok_or(BuildError::MissingDepotId)?,
            depot_key: self.depot_key.ok_or(BuildError::MissingDepotKey)?,
            install_dir: self.install_dir.ok_or(BuildError::MissingInstallDir)?,
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
