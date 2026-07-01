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

    #[error("chunk failed SHA-1 identity check")]
    ChunkSha1Mismatch {
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
    old_file_layouts: Option<std::collections::HashMap<String, Vec<OldChunkLoc>>>,

    /// Per-chunk reuse decisions tallied during a run. Test-only: it exists so
    /// tests can assert *how* a file was assembled (fetched vs. reused vs.
    /// copied off disk), which the resulting bytes alone cannot prove.
    #[cfg(test)]
    checkpoints: Arc<ReuseCheckpoints>,
}

/// Test-only tally proving which reuse branch each chunk took. See
/// [`ChunkSource`]. Counts accumulate across every file in a single job.
#[cfg(test)]
#[derive(Default)]
pub(crate) struct ReuseCheckpoints {
    pub in_place: std::sync::atomic::AtomicU64,
    pub copy_reuse: std::sync::atomic::AtomicU64,
    pub fetch: std::sync::atomic::AtomicU64,
}

#[cfg(test)]
impl ReuseCheckpoints {
    fn record(&self, plan: &[ChunkSource]) {
        use std::sync::atomic::Ordering::Relaxed;
        for source in plan {
            match source {
                ChunkSource::InPlace => &self.in_place,
                ChunkSource::CopyFrom { .. } => &self.copy_reuse,
                ChunkSource::Fetch => &self.fetch,
            }
            .fetch_add(1, Relaxed);
        }
    }

    pub fn snapshot(&self) -> (u64, u64, u64) {
        use std::sync::atomic::Ordering::Relaxed;
        (
            self.in_place.load(Relaxed),
            self.copy_reuse.load(Relaxed),
            self.fetch.load(Relaxed),
        )
    }
}

impl DepotJob {
    pub fn builder() -> DepotJobBuilder {
        DepotJobBuilder::default()
    }

    /// Handle to the test-only reuse tally. Grab it before `download` and read
    /// [`ReuseCheckpoints::snapshot`] after to assert the exercised path.
    #[cfg(test)]
    pub(crate) fn checkpoints(&self) -> Arc<ReuseCheckpoints> {
        self.checkpoints.clone()
    }

    fn emit(&self, event: DownloadEvent) {
        if let Some(ref tx) = self.event_tx {
            let _ = tx.send(event);
        }
    }

    /// Make the on-disk file's attributes match the manifest flags. Currently
    /// this is the unix executable bit (the only permission flag DepotDownloader
    /// reflects); it runs on both the write and verify-skip paths so a repair
    /// fixes a content-correct file whose flags drifted.
    fn reconcile_flags(
        &self,
        flags: DepotFileFlags,
        path: &Path,
        filename: &str,
        attach: impl Fn() -> String,
    ) -> Result<(), DownloadReport> {
        if sync_executable(path, flags.is_executable()).map_err(|e| report(e).attach(attach()))? {
            tracing::debug!("reconciled executable bit on `{filename}`");
        }
        Ok(())
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

        // Content-addressed store over the previously-installed files, so an
        // unchanged chunk is copied off disk from whichever old file holds it
        // rather than refetched. Empty when there is no prior install.
        // No prior install means an empty store: every chunk is then fetched or
        // reused positionally, which is the correct behavior.
        let cas = self
            .old_file_layouts
            .as_ref()
            .map(build_cas)
            .unwrap_or_default();

        // Directories are created lazily on first use and remembered, so a
        // manifest with thousands of files sharing a handful of directories
        // issues one `create_dir_all` per directory rather than per file.
        let mut dir_cache: std::collections::HashSet<PathBuf> = std::collections::HashSet::new();

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
                // Explicit directory entries (including empty ones) are always
                // materialized so the tree matches the manifest exactly.
                ensure_dir(&mut dir_cache, &file_path)
                    .map_err(|e| report(e).attach(attach_file()))?;
                continue;
            }

            // Symlinks carry `size == 0` and no chunks, so they must be handled
            // before the empty-file branch below -- otherwise they would be
            // written out as empty regular files instead of links.
            if flags.is_symlink() || file.link_target.is_some() {
                if let Some(parent) = file_path.parent() {
                    ensure_dir(&mut dir_cache, parent)
                        .map_err(|e| report(e).attach(attach_file()))?;
                }
                match create_symlink(&file_path, file.link_target.as_deref())
                    .map_err(|e| report(e).attach(attach_file()))?
                {
                    true => {
                        stats.files_completed += 1;
                        self.emit(DownloadEvent::FileCompleted {
                            filename: filename.to_string(),
                        });
                    }
                    false => {
                        tracing::debug!(
                            "skipping symlink `{filename}` (no target or unsupported platform)"
                        );
                        stats.files_skipped += 1;
                        self.emit(DownloadEvent::FileSkipped {
                            filename: filename.to_string(),
                        });
                    }
                }
                continue;
            }

            if file.size == 0 && file.chunks.is_empty() {
                // Skip only when the target is already a regular, empty file.
                // `symlink_metadata` does not follow links, so a stale symlink
                // here is not mistaken for the empty file it may point at. A
                // non-empty file, a symlink, or a directory in its place all
                // fall through to be rewritten.
                if self.verify
                    && let Ok(meta) = std::fs::symlink_metadata(&file_path)
                    && meta.is_file()
                    && meta.len() == 0
                {
                    // Content is correct; still reconcile the executable bit so
                    // verify repairs a file whose flags drifted.
                    self.reconcile_flags(flags, &file_path, filename, attach_file)?;
                    self.emit(DownloadEvent::FileSkipped {
                        filename: filename.to_string(),
                    });
                    stats.files_skipped += 1;
                    continue;
                }

                if let Some(parent) = file_path.parent() {
                    ensure_dir(&mut dir_cache, parent)
                        .map_err(|e| report(e).attach(attach_file()))?;
                }

                // Drop a stale symlink first; otherwise `write` would follow it
                // and truncate the link target instead of replacing the entry.
                remove_stale_symlink(&file_path).map_err(|e| report(e).attach(attach_file()))?;
                // `write` with an empty slice creates the file or truncates an
                // existing one to zero length.
                std::fs::write(&file_path, []).map_err(|e| report(e).attach(attach_file()))?;
                self.reconcile_flags(flags, &file_path, filename, attach_file)?;
                stats.files_completed += 1;
                continue;
            }

            if let Some(parent) = file_path.parent() {
                ensure_dir(&mut dir_cache, parent).map_err(|e| report(e).attach(attach_file()))?;
            }

            // Check if file already matches the manifest (skip if up-to-date)
            let expected_size = file.size;
            if self.verify && file_matches(&file_path, expected_size, file.sha_content.as_ref()) {
                // Content matches; reconcile the executable bit so verify also
                // repairs a file whose flags no longer match the manifest.
                self.reconcile_flags(flags, &file_path, filename, attach_file)?;
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
                // Written in place: the file itself is both output and reuse
                // source, so unchanged chunks stay put and are never refetched.
                // A stale symlink here must go first, or the open would follow
                // it and write through to the link target. A regular file is
                // kept so its chunks can be reused in place.
                remove_stale_symlink(&file_path).map_err(|e| report(e).attach(attach_file()))?;
                self.download_file_streaming(file, &file_path, None, &cas, &fetcher, &sem)
                    .await?
            } else {
                let staging_dir = self.install_dir.join(".DepotDownloader").join("staging");
                ensure_dir(&mut dir_cache, &staging_dir)
                    .map_err(|e| report(e).attach(attach_file()))?;
                // Staging names flatten separators; two source paths could in
                // principle collide here, but files are processed sequentially
                // and each staging file is renamed away before the next starts,
                // so only one is ever live at a time.
                let staging_path = staging_dir.join(filename.replace(['/', '\\'], "_"));

                // Stage a fresh copy, but seed unchanged chunks from the
                // currently-installed file so only changed chunks are fetched.
                let reuse_from = file_path.exists().then_some(file_path.as_path());
                let size = self
                    .download_file_streaming(file, &staging_path, reuse_from, &cas, &fetcher, &sem)
                    .await?;

                replace_file(&staging_path, &file_path)
                    .map_err(|e| report(e).attach(attach_file()))?;
                size
            };
            self.reconcile_flags(flags, &file_path, filename, attach_file)?;
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
    /// decompressed. Out-of-order completions are fine because writes are
    /// positional (`pwrite` / `seek_write`).
    ///
    /// A chunk avoids the network when a copy of its bytes already exists on
    /// disk (see [`plan_reuse`]): in the output itself, in this file's installed
    /// copy, or anywhere in the old install via the content-addressed store.
    /// Only genuinely-changed chunks are fetched.
    ///
    /// Memory is bounded by `max_downloads * (encrypted + decompressed chunk
    /// size)` plus one reusable scratch buffer. The full file is never resident.
    async fn download_file_streaming<F: ChunkFetcher + 'static>(
        &self,
        file: &ManifestFile,
        output_path: &Path,
        reuse_from: Option<&Path>,
        cas: &Cas,
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

        let (plan, sources) = plan_reuse(
            &out,
            original_len,
            &ReuseSources {
                reuse_from,
                output_path,
                install_dir: &self.install_dir,
                cas,
            },
            &file.chunks,
            &offsets,
        )
        .map_err(|r| r.attach(attach_file_ctx()))?;

        #[cfg(test)]
        self.checkpoints.record(&plan);

        let copy_from_disk = plan
            .iter()
            .filter(|s| matches!(s, ChunkSource::CopyFrom { .. }))
            .count();
        let reused = plan.iter().filter(|s| **s != ChunkSource::Fetch).count();
        let to_fetch = n - reused;
        if reused > 0 {
            tracing::debug!(
                "{}: reusing {reused}/{n} chunks ({copy_from_disk} copied from disk), fetching {to_fetch}",
                &file.filename,
            );
        }
        // In-place chunks are already correct; surface their bytes to progress
        // now. Copied chunks report from the copy task so the byte counter
        // tracks the actual work as it happens.
        for (i, chunk_meta) in file.chunks.iter().enumerate() {
            if plan[i] == ChunkSource::InPlace {
                self.emit(DownloadEvent::ChunkCompleted {
                    bytes: u64::from(chunk_meta.uncompressed_size),
                });
            }
        }

        let mut fetch_handles = Vec::with_capacity(to_fetch + usize::from(copy_from_disk > 0));

        // Copy reusable chunks off local disk into the output in one blocking
        // pass, concurrently with the network fetches below.
        if copy_from_disk > 0 {
            // (source index into `sources`, read offset, write offset, size).
            // Read and write offsets differ when an update moved the chunk.
            let ops: Vec<(usize, u64, u64, usize)> = file
                .chunks
                .iter()
                .enumerate()
                .filter_map(|(i, c)| match plan[i] {
                    ChunkSource::CopyFrom { source, src_offset } => {
                        Some((source, src_offset, offsets[i], c.uncompressed_size as usize))
                    }
                    _ => None,
                })
                .collect();
            let sources = sources.clone();
            let out = out.clone();
            let event_tx = self.event_tx.clone();
            let ctx = attach_file_ctx();
            fetch_handles.push(tokio::spawn(async move {
                tokio::task::spawn_blocking(move || -> Result<(), DownloadReport> {
                    // `copy_from_disk > 0` gates this task, so `ops` is non-empty.
                    let cap = ops
                        .iter()
                        .map(|(_, _, _, s)| *s)
                        .max()
                        .expect("copy_from_disk > 0 implies at least one op");
                    let mut buf = vec![0u8; cap];
                    for (source, src_off, dst_off, size) in ops {
                        pread_exact(&sources[source], &mut buf[..size], src_off)
                            .map_err(|e| report(e).attach(ctx.clone()))?;
                        pwrite_all(&out, &buf[..size], dst_off)
                            .map_err(|e| report(e).attach(ctx.clone()))?;
                        if let Some(ref tx) = event_tx {
                            let _ = tx.send(DownloadEvent::ChunkCompleted { bytes: size as u64 });
                        }
                    }
                    Ok(())
                })
                .await
                .map_err(report)?
            }));
        }
        // Sources not consumed by the copy task are closed here.
        drop(sources);

        for (i, chunk_meta) in file.chunks.iter().enumerate() {
            if plan[i] != ChunkSource::Fetch {
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
                    // The chunk id is the SHA-1 of the uncompressed bytes, so
                    // verify identity instead of trusting only the Adler-32 gate
                    // in `process_chunk`. An all-zero id carries no identity to
                    // check against; `process_chunk`'s size + checksum is then
                    // all we have.
                    if chunk_id != ChunkId([0u8; 20]) {
                        let actual = sha1_of(&processed);
                        if actual != chunk_id.0 {
                            return Err(Report::new(DownloadError::ChunkSha1Mismatch {
                                expected: chunk_id.0,
                                actual,
                            })
                            .attach(attach_chunk_in_blocking.clone()));
                        }
                    }
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

        // Stream-verify the whole assembled file against the manifest SHA-1.
        // Every individual chunk is already SHA-1-verified (reused chunks in
        // `plan_reuse`, fetched chunks after decode, except chunks that carry no
        // id and fall back to the Adler-32 gate). This end-to-end pass is the
        // backstop for those id-less chunks and for assembly mistakes (a chunk
        // written at the wrong offset) that per-chunk checks cannot see.
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

/// Where a chunk's bytes come from, decided by [`plan_reuse`].
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum ChunkSource {
    /// Already correct at the chunk's own offset in the output file; leave it.
    InPlace,
    /// Correct in an already-opened on-disk source (`source` indexes the pool
    /// returned alongside the plan) at `src_offset`, which need not equal the
    /// chunk's new offset. Copy those bytes into the output.
    CopyFrom { source: usize, src_offset: u64 },
    /// Must be downloaded.
    Fetch,
}

/// A chunk of the previously-installed version of a file: its content identity
/// and where those bytes live in that installed file. Used to reuse a chunk by
/// content even when an update has shifted it to a different offset.
#[derive(Clone, Debug)]
pub struct OldChunkLoc {
    pub id: ChunkId,
    pub offset: u64,
    pub size: u32,
}

/// Location of a chunk's bytes within the previously-installed depot: a path
/// relative to the install dir, plus the offset and size in that file.
#[derive(Clone, Debug)]
struct CasLoc {
    rel_path: String,
    offset: u64,
    size: u32,
}

/// Content-addressed store over the previously-installed files: chunk SHA-1 ->
/// where a copy of those bytes lives on disk. Built once per download and
/// consulted for every chunk, so an unchanged chunk is copied off local disk
/// from whichever old file holds it, even a different file than the one being
/// written. Every hit is SHA-verified before use, so a since-overwritten
/// ("evicted") source simply fails verification and falls back to a fetch.
type Cas = std::collections::HashMap<[u8; 20], CasLoc>;

fn build_cas(layouts: &std::collections::HashMap<String, Vec<OldChunkLoc>>) -> Cas {
    let mut cas = Cas::with_capacity(layouts.values().map(Vec::len).sum());
    for (rel_path, locs) in layouts {
        for loc in locs {
            if loc.id == ChunkId([0u8; 20]) {
                continue;
            }
            // First occurrence wins; any location holding the bytes is equally
            // valid since reuse is verified by content, not position.
            cas.entry(loc.id.0).or_insert_with(|| CasLoc {
                rel_path: rel_path.clone(),
                offset: loc.offset,
                size: loc.size,
            });
        }
    }
    cas
}

/// Decide, per chunk, whether it can be sourced from local disk instead of the
/// network. `out_len` is the output size before `set_len`, so chunks in the
/// zero-extended tail are never reused in place; `reuse_src_len` bounds reads
/// from the installed file.
///
/// Reuse is gated on the chunk's SHA-1 identity, not the weak Adler-32: the
/// chunk id *is* the SHA-1 of its uncompressed bytes, so recomputing it over
/// the candidate bytes is an exact content match with no collision risk. A
/// chunk whose id is absent (all-zero) carries no verifiable identity and is
/// always fetched rather than trusting a weaker signal.
/// `size` bytes at `off` in `f` hash to `id`. Reads into `buf` (which must be
/// at least `size` long). A read failure (past EOF, source gone) is a non-match.
fn chunk_matches(f: &File, buf: &mut [u8], off: u64, id: &[u8; 20]) -> bool {
    pread_exact(f, buf, off).is_ok() && &sha1_of(buf) == id
}

/// Decide, per chunk, whether it can be sourced from local disk, and open the
/// source files needed to do so. Returns the plan plus a pool of opened files
/// that [`ChunkSource::CopyFrom::source`] indexes into.
///
/// Reuse is gated on the chunk's SHA-1 identity, never a weak checksum: the
/// chunk id is the SHA-1 of its uncompressed bytes, so recomputing it over the
/// candidate bytes is an exact content match. A chunk with no identity
/// (all-zero id) is always fetched.
///
/// Only the source files this one output needs are opened, then dropped when
/// the returned pool is dropped; handles for the whole depot are never held at
/// once. `reuse_from` is a positional fallback (the installed copy of this same
/// file, for when no CAS layout exists); `cas` covers content-addressed reuse
/// from anywhere in the old install, including other files and shifted offsets.
/// The on-disk places [`plan_reuse`] may source chunk bytes from, besides the
/// output itself.
struct ReuseSources<'a> {
    /// The installed copy of the file being written (positional fallback).
    reuse_from: Option<&'a Path>,
    /// The file being written; never read as a source (unsafe mid-write).
    output_path: &'a Path,
    /// Root the CAS's relative paths resolve against.
    install_dir: &'a Path,
    /// Content-addressed store over the whole previous install.
    cas: &'a Cas,
}

fn plan_reuse(
    out: &Arc<File>,
    out_len: u64,
    src: &ReuseSources<'_>,
    chunks: &[ManifestChunk],
    offsets: &[u64],
) -> Result<(Vec<ChunkSource>, Vec<Arc<File>>), DownloadReport> {
    let ReuseSources {
        reuse_from,
        output_path,
        install_dir,
        cas,
    } = *src;
    let cap = chunks
        .iter()
        .map(|c| c.uncompressed_size as usize)
        .max()
        // Empty chunk list yields no buffer; the loop below is then a no-op.
        .unwrap_or(0)
        .min(REUSE_BUFFER_CAP);
    let mut buf = vec![0u8; cap];
    let unidentified = [0u8; 20];

    let mut plan = Vec::with_capacity(chunks.len());
    let mut sources: Vec<Arc<File>> = Vec::new();
    let mut by_path: std::collections::HashMap<PathBuf, usize> = std::collections::HashMap::new();

    // Pre-open the installed copy of this file (positional fallback source),
    // unless it is the file we are writing in place.
    let target_src = reuse_from
        .filter(|p| *p != output_path)
        .and_then(|p| std::fs::File::open(p).ok())
        .map(|f| {
            let idx = sources.len();
            sources.push(Arc::new(f));
            idx
        });

    for (chunk_meta, &offset) in chunks.iter().zip(offsets.iter()) {
        let size = chunk_meta.uncompressed_size as usize;
        // No bytes, no verifiable identity, or too large to buffer: fetch it.
        if size == 0 || size > cap || chunk_meta.id.0 == unidentified {
            plan.push(ChunkSource::Fetch);
            continue;
        }
        let id = &chunk_meta.id.0;
        let end = offset.saturating_add(size as u64);

        // 1. Already correct at its own offset in the output (resume, or a
        //    non-atomic in-place update). `out_len` (length before `set_len`)
        //    excludes the zero-extended tail of a grown file; on a shrunk file
        //    the read is the guard, failing past the new EOF.
        if end <= out_len && chunk_matches(out, &mut buf[..size], offset, id) {
            plan.push(ChunkSource::InPlace);
            continue;
        }

        // 2. Positional fallback: same offset in this file's installed copy.
        if let Some(ti) = target_src
            && chunk_matches(&sources[ti], &mut buf[..size], offset, id)
        {
            plan.push(ChunkSource::CopyFrom {
                source: ti,
                src_offset: offset,
            });
            continue;
        }

        // 3. Content-addressed: the chunk lives somewhere in the old install,
        //    possibly a different file or a shifted offset.
        if let Some(loc) = cas.get(id)
            && loc.size as usize == size
        {
            let abs = install_dir.join(&loc.rel_path);
            // Never read the file we are writing in place; step 1 already
            // covered its same-offset bytes and a mid-write read is unsafe.
            if abs != output_path
                && let Some(idx) = open_source(&abs, &mut sources, &mut by_path)
                && chunk_matches(&sources[idx], &mut buf[..size], loc.offset, id)
            {
                plan.push(ChunkSource::CopyFrom {
                    source: idx,
                    src_offset: loc.offset,
                });
                continue;
            }
        }

        plan.push(ChunkSource::Fetch);
    }
    Ok((plan, sources))
}

/// Upper bound on distinct reuse-source files opened for a single output file.
/// A chunk that would need a source beyond this is fetched instead. Bounds file
/// descriptors when one output's chunks are scattered across many old files,
/// well below typical process limits while covering any realistic layout.
const MAX_REUSE_SOURCES: usize = 512;

/// Open `path` once, returning its index in `sources` (reusing an already-open
/// handle for the same path). `None` if the file cannot be opened or the
/// per-output source cap is reached; both cases just make the chunk a fetch.
fn open_source(
    path: &Path,
    sources: &mut Vec<Arc<File>>,
    by_path: &mut std::collections::HashMap<PathBuf, usize>,
) -> Option<usize> {
    if let Some(&i) = by_path.get(path) {
        return Some(i);
    }
    if sources.len() >= MAX_REUSE_SOURCES {
        return None;
    }
    let f = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) => {
            // A source we cannot open just falls back to a fetch; surface it so
            // a silent drop in reuse rate (e.g. hitting the fd limit) is visible.
            tracing::debug!("reuse source {} unavailable: {e}", path.display());
            return None;
        }
    };
    let idx = sources.len();
    sources.push(Arc::new(f));
    by_path.insert(path.to_path_buf(), idx);
    Some(idx)
}

/// SHA-1 of `data`, the identity a depot chunk is keyed by.
fn sha1_of(data: &[u8]) -> [u8; 20] {
    use sha1::Digest;
    sha1::Sha1::digest(data).into()
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

/// `create_dir_all` for `dir`, skipping the syscall once a directory has
/// already been created in this run.
fn ensure_dir(cache: &mut std::collections::HashSet<PathBuf>, dir: &Path) -> io::Result<()> {
    if !cache.contains(dir) {
        std::fs::create_dir_all(dir)?;
        cache.insert(dir.to_path_buf());
    }
    Ok(())
}

/// Create a symlink at `path` pointing at `target`, replacing any existing
/// entry. Returns `Ok(false)` when there is no target to link to or the
/// platform has no symlink support, so the caller can record it as skipped.
#[cfg(unix)]
fn create_symlink(path: &Path, target: Option<&str>) -> io::Result<bool> {
    let Some(target) = target else {
        return Ok(false);
    };
    // Remove a stale link/file first; `symlink` fails if the path exists.
    // `symlink_metadata` does not follow links, so an existing symlink is
    // removed rather than its target being inspected.
    if std::fs::symlink_metadata(path).is_ok() {
        std::fs::remove_file(path)?;
    }
    std::os::unix::fs::symlink(target, path)?;
    Ok(true)
}

#[cfg(not(unix))]
fn create_symlink(_path: &Path, _target: Option<&str>) -> io::Result<bool> {
    // Windows symlink creation needs elevated privileges; treat as unsupported.
    Ok(false)
}

/// Remove `path` only if it is currently a symlink, so a manifest entry that is
/// now a regular file replaces the link rather than writing through it. Regular
/// files are left in place (in-place chunk reuse depends on them) and
/// directories are left for the write/open to fail against loudly.
fn remove_stale_symlink(path: &Path) -> io::Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => std::fs::remove_file(path),
        _ => Ok(()),
    }
}

/// Reconcile a regular file's unix executable bit with the manifest's
/// `EXECUTABLE` flag: add the exec bits when the file should be executable and
/// they are missing, strip them when it should not be and they are present.
/// Returns whether a change was made. Directories and symlinks are left alone.
/// A no-op on non-unix, matching DepotDownloader (Windows has no exec bit).
#[cfg(unix)]
fn sync_executable(path: &Path, want_exec: bool) -> io::Result<bool> {
    use std::os::unix::fs::PermissionsExt;
    let meta = std::fs::symlink_metadata(path)?;
    if !meta.file_type().is_file() {
        return Ok(false);
    }
    let mode = meta.permissions().mode();
    let has_exec = mode & 0o111 != 0;
    if want_exec == has_exec {
        return Ok(false);
    }
    let new_mode = if want_exec {
        mode | 0o111
    } else {
        mode & !0o111
    };
    let mut perms = meta.permissions();
    perms.set_mode(new_mode);
    std::fs::set_permissions(path, perms)?;
    Ok(true)
}

#[cfg(not(unix))]
fn sync_executable(_path: &Path, _want_exec: bool) -> io::Result<bool> {
    Ok(false)
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

/// Whole-file skip check for verify mode: true only when `path` provably holds
/// the manifest's content. Without a content SHA there is no way to prove that,
/// so this returns false and the file goes through the chunk pipeline, which
/// verifies each chunk by its SHA-1 identity. A size-only match is never
/// treated as sufficient.
fn file_matches(path: &Path, expected_size: u64, sha_content: Option<&[u8; 20]>) -> bool {
    let Some(expected_sha) = sha_content else {
        return false;
    };
    let meta = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(_) => return false,
    };
    if meta.len() != expected_size {
        return false;
    }
    // Stream the hash so verifying a multi-GB file does not pull it all into
    // memory at once.
    use sha1::Digest;
    use std::io::Read;
    let Ok(mut f) = File::open(path) else {
        return false;
    };
    let mut hasher = sha1::Sha1::new();
    let mut buf = vec![0u8; HASH_READ_BUFFER];
    loop {
        match f.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => hasher.update(&buf[..n]),
            Err(_) => return false,
        }
    }
    let actual: [u8; 20] = hasher.finalize().into();
    actual == *expected_sha
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
    old_file_layouts: Option<std::collections::HashMap<String, Vec<OldChunkLoc>>>,
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

    /// Chunk layout of each file in the previously-installed manifest, keyed by
    /// normalized path. Enables content-addressed reuse: an unchanged chunk is
    /// copied from the installed file even if an update shifted its offset.
    pub fn old_file_layouts(
        mut self,
        layouts: std::collections::HashMap<String, Vec<OldChunkLoc>>,
    ) -> Self {
        self.old_file_layouts = Some(layouts);
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
            old_file_layouts: self.old_file_layouts,
            #[cfg(test)]
            checkpoints: Arc::new(ReuseCheckpoints::default()),
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
