use crate::download::BoxError;
use crate::download::ChunkFetcher;
use crate::download::DepotJob;
use crate::download::DownloadError;
use crate::download::FileFilter;
use crate::event::DownloadEvent;
use bytes::Bytes;
use std::collections::HashMap;
use std::sync::Arc;
use steamroom::depot::AppId;
use steamroom::depot::ChunkId;
use steamroom::depot::DepotId;
use steamroom::depot::DepotKey;
use steamroom::depot::manifest::DepotManifest;
use steamroom::depot::manifest::ManifestChunk;
use steamroom::depot::manifest::ManifestFile;
use steamroom::enums::DepotFileFlags;
use steamroom::util::checksum::Sha1Hash;
use steamroom::util::checksum::SteamAdler32;

/// Steam apps that genuinely allow anonymous (no-account) content access. They
/// are used as realistic identifiers when exercising pipeline behavior against
/// mock fetchers. The unit test suite never performs a live download of them:
/// these constants stand in for real depots without touching the network.
mod anon {
    use super::AppId;
    use super::DepotId;

    /// Valve's "Spacewar" SDK example, app 480 / depot 481. The canonical
    /// anonymous-access depot and the one Valve documents for testing. The app
    /// id is consumed by higher-level orchestration tests; the depot id drives
    /// the download-pipeline behavior tests here.
    #[allow(dead_code)]
    pub const SPACEWAR_APP: AppId = AppId(480);
    pub const SPACEWAR_DEPOT: DepotId = DepotId(481);

    /// "Steamworks Common Redistributables", app 228980. Also anonymous-access;
    /// kept here so behavior tests have a second real identifier to reach for.
    #[allow(dead_code)]
    pub const STEAMWORKS_COMMON_REDIST_APP: AppId = AppId(228980);
}

struct NullFetcher;

impl ChunkFetcher for NullFetcher {
    async fn fetch_chunk(
        &self,
        _depot_id: DepotId,
        _chunk_id: &ChunkId,
    ) -> Result<Bytes, BoxError> {
        panic!("NullFetcher should not be called");
    }
}

/// A mock fetcher that returns pre-encrypted chunk data keyed by ChunkId.
struct MockFetcher {
    chunks: HashMap<ChunkId, Bytes>,
}

impl ChunkFetcher for MockFetcher {
    async fn fetch_chunk(&self, _depot_id: DepotId, chunk_id: &ChunkId) -> Result<Bytes, BoxError> {
        self.chunks
            .get(chunk_id)
            .cloned()
            .ok_or_else(|| format!("chunk {:?} not found in mock", chunk_id).into())
    }
}

/// A fetcher that only serves specific chunk IDs and panics on anything else.
/// Used to prove that reusable chunks are never fetched from the network.
struct SelectiveFetcher {
    allowed: HashMap<ChunkId, Bytes>,
}

impl ChunkFetcher for SelectiveFetcher {
    async fn fetch_chunk(&self, _depot_id: DepotId, chunk_id: &ChunkId) -> Result<Bytes, BoxError> {
        match self.allowed.get(chunk_id) {
            Some(data) => Ok(data.clone()),
            None => panic!(
                "SelectiveFetcher: chunk {} should have been reused, not fetched",
                chunk_id
            ),
        }
    }
}

/// Build an encrypted chunk from plaintext using the given depot key.
/// Format: ECB(IV) ++ CBC(plaintext, key, IV)
fn encrypt_chunk(plaintext: &[u8], key: &DepotKey) -> Vec<u8> {
    let iv = [0x42u8; 16];
    let encrypted_iv = steamroom::crypto::symmetric_encrypt_ecb_nopad(&iv, &key.0).unwrap();
    let encrypted_body = steamroom::crypto::symmetric_encrypt_cbc(plaintext, &key.0, &iv).unwrap();
    let mut chunk = Vec::with_capacity(encrypted_iv.len() + encrypted_body.len());
    chunk.extend_from_slice(&encrypted_iv);
    chunk.extend_from_slice(&encrypted_body);
    chunk
}

/// The real depot identity of a chunk: the SHA-1 of its uncompressed bytes.
fn sha_id(content: &[u8]) -> ChunkId {
    ChunkId(Sha1Hash::compute(content).0)
}

fn enc(content: &[u8], key: &DepotKey) -> Bytes {
    Bytes::from(encrypt_chunk(content, key))
}

/// A manifest chunk carrying its true SHA-1 identity and Adler-32, positioned
/// at `offset`.
fn chunk_at(content: &[u8], offset: u64) -> ManifestChunk {
    let mut c = ManifestChunk::new(
        sha_id(content),
        SteamAdler32::compute(content).0,
        content.len() as u32,
    );
    c.offset = Some(offset);
    c
}

fn empty_file(name: &str) -> ManifestFile {
    ManifestFile::new(name.to_string(), 0)
}

fn manifest_with(files: &[&str]) -> DepotManifest {
    DepotManifest::new(files.iter().map(|n| empty_file(n)).collect())
}

fn file_with_chunks(name: &str, chunks: Vec<ManifestChunk>) -> ManifestFile {
    let size: u64 = chunks.iter().map(|c| c.uncompressed_size as u64).sum();
    let mut f = ManifestFile::new(name.to_string(), size);
    f.chunks = chunks;
    f
}

#[test]
fn filter_none_matches_everything() {
    let f = FileFilter::None;
    assert!(f.matches("anything.txt"));
    assert!(f.matches(""));
}

#[test]
fn filter_regex_matches_pattern() {
    let f = FileFilter::Regex(regex::Regex::new(r"\.dll$").unwrap());
    assert!(f.matches("bin/game.dll"));
    assert!(!f.matches("bin/game.exe"));
}

#[test]
fn filelist_literal_case_insensitive() {
    let f = FileFilter::from_filelist(&["Game\\Bin\\Server.dll".into()]).unwrap();
    assert!(f.matches("game\\bin\\server.dll"));
    assert!(f.matches("Game\\Bin\\Server.dll"));
}

#[test]
fn filelist_normalizes_separators() {
    let f = FileFilter::from_filelist(&["game/bin/server.dll".into()]).unwrap();
    assert!(f.matches("game\\bin\\server.dll"));
}

#[test]
fn filelist_regex_prefix() {
    let f = FileFilter::from_filelist(&["regex:.*\\.idx$".into()]).unwrap();
    assert!(f.matches("bin/123/idx/foo.idx"));
    assert!(!f.matches("bin/123/idx/foo.txt"));
}

#[test]
fn filelist_mixed_literal_and_regex() {
    let f = FileFilter::from_filelist(&["exact_file.txt".into(), "regex:^maps/.*\\.vpk$".into()])
        .unwrap();
    assert!(f.matches("exact_file.txt"));
    assert!(f.matches("maps/de_dust2.vpk"));
    assert!(!f.matches("other.txt"));
}

#[test]
fn filelist_invalid_regex_returns_error() {
    let result = FileFilter::from_filelist(&["regex:[invalid".into()]);
    assert!(result.is_err());
}

#[test]
fn filelist_empty_gives_no_matches() {
    let f = FileFilter::from_filelist(&[]).unwrap();
    assert!(!f.matches("anything"));
}

#[tokio::test]
async fn download_single_file_with_one_chunk() {
    let dir = tempfile::tempdir().unwrap();
    let install = dir.path();
    let key = DepotKey([0xAA; 32]);
    let plaintext = b"hello steam depot";

    let mut chunks = HashMap::new();
    chunks.insert(sha_id(plaintext), enc(plaintext, &key));

    let manifest = DepotManifest::new(vec![file_with_chunks(
        "test.txt",
        vec![chunk_at(plaintext, 0)],
    )]);

    let job = DepotJob::builder()
        .depot_id(anon::SPACEWAR_DEPOT)
        .depot_key(key)
        .install_dir(install.to_path_buf())
        .build()
        .unwrap();

    let cp = job.checkpoints();
    let stats = job
        .download(&manifest, Arc::new(MockFetcher { chunks }))
        .await
        .unwrap();

    assert_eq!(stats.files_completed, 1);
    assert_eq!(std::fs::read(install.join("test.txt")).unwrap(), plaintext);
    // Nothing on disk to reuse: the one chunk is fetched.
    assert_eq!(cp.snapshot(), (0, 0, 1));
}

#[tokio::test]
async fn download_multi_chunk_file_reassembles_in_order() {
    let dir = tempfile::tempdir().unwrap();
    let install = dir.path();
    let key = DepotKey([0xBB; 32]);

    let part_a = b"AAAA";
    let part_b = b"BBBB";
    let combined: Vec<u8> = [&part_a[..], &part_b[..]].concat();

    let mut chunks = HashMap::new();
    chunks.insert(sha_id(part_a), enc(part_a, &key));
    chunks.insert(sha_id(part_b), enc(part_b, &key));

    let manifest = DepotManifest::new(vec![file_with_chunks(
        "multi.bin",
        vec![chunk_at(part_a, 0), chunk_at(part_b, part_a.len() as u64)],
    )]);

    let job = DepotJob::builder()
        .depot_id(anon::SPACEWAR_DEPOT)
        .depot_key(key)
        .install_dir(install.to_path_buf())
        .build()
        .unwrap();

    let stats = job
        .download(&manifest, Arc::new(MockFetcher { chunks }))
        .await
        .unwrap();

    assert_eq!(stats.files_completed, 1);
    assert_eq!(std::fs::read(install.join("multi.bin")).unwrap(), combined);
}

#[tokio::test]
async fn download_skips_filtered_files() {
    let dir = tempfile::tempdir().unwrap();
    let install = dir.path();
    let key = DepotKey([0xCC; 32]);

    let manifest = manifest_with(&["include.txt", "exclude.dat"]);

    let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel();

    let job = DepotJob::builder()
        .depot_id(anon::SPACEWAR_DEPOT)
        .depot_key(key)
        .install_dir(install.to_path_buf())
        .file_filter(FileFilter::Regex(regex::Regex::new(r"\.txt$").unwrap()))
        .event_sender(event_tx)
        .build()
        .unwrap();

    let stats = job
        .download(&manifest, Arc::new(NullFetcher))
        .await
        .unwrap();

    assert_eq!(stats.files_completed, 1);
    assert_eq!(stats.files_skipped, 1);
    assert!(install.join("include.txt").exists());
    assert!(!install.join("exclude.dat").exists());

    drop(job);
    let mut skipped = vec![];
    while let Ok(event) = event_rx.try_recv() {
        if let DownloadEvent::FileSkipped { filename } = event {
            skipped.push(filename);
        }
    }
    assert_eq!(skipped, vec!["exclude.dat"]);
}

#[tokio::test]
async fn download_emits_progress_events() {
    let dir = tempfile::tempdir().unwrap();
    let install = dir.path();
    let key = DepotKey([0xDD; 32]);
    let plaintext = b"event test data!";

    let mut chunks = HashMap::new();
    chunks.insert(sha_id(plaintext), enc(plaintext, &key));

    let manifest = DepotManifest::new(vec![file_with_chunks(
        "evented.bin",
        vec![chunk_at(plaintext, 0)],
    )]);

    let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel();

    let job = DepotJob::builder()
        .depot_id(anon::SPACEWAR_DEPOT)
        .depot_key(key)
        .install_dir(install.to_path_buf())
        .event_sender(event_tx)
        .build()
        .unwrap();

    job.download(&manifest, Arc::new(MockFetcher { chunks }))
        .await
        .unwrap();
    drop(job);

    let mut saw_started = false;
    let mut saw_chunk = false;
    let mut saw_completed = false;
    while let Ok(event) = event_rx.try_recv() {
        match event {
            DownloadEvent::FileStarted { filename } if filename == "evented.bin" => {
                saw_started = true
            }
            DownloadEvent::ChunkCompleted { bytes } if bytes == plaintext.len() as u64 => {
                saw_chunk = true
            }
            DownloadEvent::FileCompleted { filename } if filename == "evented.bin" => {
                saw_completed = true
            }
            _ => {}
        }
    }
    assert!(saw_started, "missing FileStarted event");
    assert!(saw_chunk, "missing ChunkCompleted event");
    assert!(saw_completed, "missing FileCompleted event");
}

#[tokio::test]
async fn delta_removes_files_not_in_new_manifest() {
    let dir = tempfile::tempdir().unwrap();
    let install = dir.path();

    std::fs::write(install.join("keep.txt"), b"keep").unwrap();
    std::fs::write(install.join("remove_me.txt"), b"old").unwrap();
    std::fs::write(install.join("also_gone.dat"), b"old").unwrap();

    let old_files = vec![
        "keep.txt".to_string(),
        "remove_me.txt".to_string(),
        "also_gone.dat".to_string(),
    ];

    let new_manifest = manifest_with(&["keep.txt", "new_file.txt"]);

    let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel();

    let job = DepotJob::builder()
        .depot_id(anon::SPACEWAR_DEPOT)
        .depot_key(DepotKey([0; 32]))
        .install_dir(install.to_path_buf())
        .old_manifest_files(old_files)
        .event_sender(event_tx)
        .build()
        .unwrap();

    let stats = job
        .download(&new_manifest, Arc::new(NullFetcher))
        .await
        .unwrap();

    assert_eq!(stats.files_removed, 2);
    assert!(!install.join("remove_me.txt").exists());
    assert!(!install.join("also_gone.dat").exists());
    assert!(install.join("keep.txt").exists());
    assert!(install.join("new_file.txt").exists());

    drop(job);
    let mut removed = vec![];
    while let Ok(event) = event_rx.try_recv() {
        if let DownloadEvent::FileRemoved { filename } = event {
            removed.push(filename);
        }
    }
    removed.sort();
    assert_eq!(removed, vec!["also_gone.dat", "remove_me.txt"]);
}

#[tokio::test]
async fn delta_no_removal_without_old_manifest() {
    let dir = tempfile::tempdir().unwrap();
    let install = dir.path();
    std::fs::write(install.join("stale.txt"), b"should survive").unwrap();

    let new_manifest = manifest_with(&["new.txt"]);

    let job = DepotJob::builder()
        .depot_id(anon::SPACEWAR_DEPOT)
        .depot_key(DepotKey([0; 32]))
        .install_dir(install.to_path_buf())
        .build()
        .unwrap();

    let stats = job
        .download(&new_manifest, Arc::new(NullFetcher))
        .await
        .unwrap();

    assert_eq!(stats.files_removed, 0);
    assert!(install.join("stale.txt").exists());
}

#[tokio::test]
async fn delta_skips_already_missing_files() {
    let dir = tempfile::tempdir().unwrap();
    let install = dir.path();

    let old_files = vec!["gone.txt".to_string()];
    let new_manifest = manifest_with(&["new.txt"]);

    let job = DepotJob::builder()
        .depot_id(anon::SPACEWAR_DEPOT)
        .depot_key(DepotKey([0; 32]))
        .install_dir(install.to_path_buf())
        .old_manifest_files(old_files)
        .build()
        .unwrap();

    let stats = job
        .download(&new_manifest, Arc::new(NullFetcher))
        .await
        .unwrap();

    assert_eq!(stats.files_removed, 0);
}

#[tokio::test]
async fn delta_removes_empty_directories() {
    let dir = tempfile::tempdir().unwrap();
    let install = dir.path();

    let sub = install.join("old_subdir");
    std::fs::create_dir_all(&sub).unwrap();

    let old_files = vec!["old_subdir".to_string()];
    let new_manifest = manifest_with(&["file.txt"]);

    let job = DepotJob::builder()
        .depot_id(anon::SPACEWAR_DEPOT)
        .depot_key(DepotKey([0; 32]))
        .install_dir(install.to_path_buf())
        .old_manifest_files(old_files)
        .build()
        .unwrap();

    let stats = job
        .download(&new_manifest, Arc::new(NullFetcher))
        .await
        .unwrap();

    assert_eq!(stats.files_removed, 1);
    assert!(!sub.exists());
}

#[tokio::test]
async fn delta_does_not_remove_nonempty_directories() {
    let dir = tempfile::tempdir().unwrap();
    let install = dir.path();

    let sub = install.join("subdir");
    std::fs::create_dir_all(&sub).unwrap();
    std::fs::write(sub.join("child.txt"), b"content").unwrap();

    let old_files = vec!["subdir".to_string()];
    let new_manifest = manifest_with(&["other.txt"]);

    let job = DepotJob::builder()
        .depot_id(anon::SPACEWAR_DEPOT)
        .depot_key(DepotKey([0; 32]))
        .install_dir(install.to_path_buf())
        .old_manifest_files(old_files)
        .build()
        .unwrap();

    let stats = job
        .download(&new_manifest, Arc::new(NullFetcher))
        .await
        .unwrap();

    assert_eq!(stats.files_removed, 0);
    assert!(sub.exists());
}

#[tokio::test]
async fn delta_handles_nested_paths() {
    let dir = tempfile::tempdir().unwrap();
    let install = dir.path();

    let nested = install.join("game").join("bin");
    std::fs::create_dir_all(&nested).unwrap();
    std::fs::write(nested.join("old.dll"), b"old").unwrap();
    std::fs::write(nested.join("keep.dll"), b"keep").unwrap();

    let old_files = vec![
        "game/bin/old.dll".to_string(),
        "game/bin/keep.dll".to_string(),
    ];
    let new_manifest = manifest_with(&["game\\bin\\keep.dll"]);

    let job = DepotJob::builder()
        .depot_id(anon::SPACEWAR_DEPOT)
        .depot_key(DepotKey([0; 32]))
        .install_dir(install.to_path_buf())
        .old_manifest_files(old_files)
        .build()
        .unwrap();

    let stats = job
        .download(&new_manifest, Arc::new(NullFetcher))
        .await
        .unwrap();

    assert_eq!(stats.files_removed, 1);
    assert!(!nested.join("old.dll").exists());
    assert!(nested.join("keep.dll").exists());
}

#[tokio::test]
async fn resume_truncates_partial_chunk_data() {
    let dir = tempfile::tempdir().unwrap();
    let install = dir.path();
    let key = DepotKey([0xAA; 32]);

    let chunk_a_plain = b"AAAAAAAAAAAAAAAA";
    let chunk_b_plain = b"BBBBBBBBBBBBBBBB";
    let combined = [&chunk_a_plain[..], &chunk_b_plain[..]].concat();

    let mut chunks = HashMap::new();
    chunks.insert(sha_id(chunk_a_plain), enc(chunk_a_plain, &key));
    chunks.insert(sha_id(chunk_b_plain), enc(chunk_b_plain, &key));

    let manifest = DepotManifest::new(vec![file_with_chunks(
        "resume.bin",
        vec![chunk_at(chunk_a_plain, 0), chunk_at(chunk_b_plain, 16)],
    )]);

    // Simulate an interrupted download: chunk A fully written + 5 garbage bytes
    // from a partially-written chunk B.
    let staging_dir = install.join(".DepotDownloader").join("staging");
    std::fs::create_dir_all(&staging_dir).unwrap();
    let staging_path = staging_dir.join("resume.bin");
    {
        let mut f = std::fs::File::create(&staging_path).unwrap();
        use std::io::Write;
        f.write_all(chunk_a_plain).unwrap();
        f.write_all(b"XXXXX").unwrap(); // partial garbage
    }
    assert_eq!(
        std::fs::metadata(&staging_path).unwrap().len(),
        chunk_a_plain.len() as u64 + 5
    );

    let job = DepotJob::builder()
        .depot_id(anon::SPACEWAR_DEPOT)
        .depot_key(key)
        .install_dir(install.to_path_buf())
        .build()
        .unwrap();

    let cp = job.checkpoints();
    let stats = job
        .download(&manifest, Arc::new(MockFetcher { chunks }))
        .await
        .unwrap();

    assert_eq!(stats.files_completed, 1);
    let result = std::fs::read(install.join("resume.bin")).unwrap();
    assert_eq!(
        result, combined,
        "file should be chunk_a + chunk_b with no garbage"
    );
    // Chunk A is reused in place from staging; chunk B (partial garbage) is
    // refetched. The garbage tail is truncated by set_len.
    assert_eq!(cp.snapshot(), (1, 0, 1));
}

#[tokio::test]
async fn resume_skips_fully_staged_chunks() {
    let dir = tempfile::tempdir().unwrap();
    let install = dir.path();
    let key = DepotKey([0xAA; 32]);

    let chunk_a_plain = b"AAAAAAAAAAAAAAAA";
    let chunk_b_plain = b"BBBBBBBBBBBBBBBB";
    let combined = [&chunk_a_plain[..], &chunk_b_plain[..]].concat();

    // Only chunk B in the mock: chunk A must be reused from staging.
    let mut chunks = HashMap::new();
    chunks.insert(sha_id(chunk_b_plain), enc(chunk_b_plain, &key));

    let manifest = DepotManifest::new(vec![file_with_chunks(
        "resume2.bin",
        vec![chunk_at(chunk_a_plain, 0), chunk_at(chunk_b_plain, 16)],
    )]);

    // Pre-stage chunk A exactly (no partial data).
    let staging_dir = install.join(".DepotDownloader").join("staging");
    std::fs::create_dir_all(&staging_dir).unwrap();
    let staging_path = staging_dir.join("resume2.bin");
    std::fs::write(&staging_path, chunk_a_plain).unwrap();

    let job = DepotJob::builder()
        .depot_id(anon::SPACEWAR_DEPOT)
        .depot_key(key)
        .install_dir(install.to_path_buf())
        .build()
        .unwrap();

    let cp = job.checkpoints();
    let stats = job
        .download(&manifest, Arc::new(MockFetcher { chunks }))
        .await
        .unwrap();

    assert_eq!(stats.files_completed, 1);
    let result = std::fs::read(install.join("resume2.bin")).unwrap();
    assert_eq!(result, combined);
    assert_eq!(cp.snapshot(), (1, 0, 1));
}

#[tokio::test]
async fn delta_reuses_unchanged_chunks() {
    let dir = tempfile::tempdir().unwrap();
    let install = dir.path();
    let key = DepotKey([0xAA; 32]);

    let chunk_a = b"AAAAAAAAAAAAAAAA"; // unchanged
    let chunk_b_old = b"BBBBBBBBBBBBBBBB"; // will change
    let chunk_c = b"CCCCCCCCCCCCCCCC"; // unchanged
    let chunk_b_new = b"bbbbbbbbbbbbbbbb"; // new version

    // Write the "old" version of the file in place.
    let old_content = [&chunk_a[..], &chunk_b_old[..], &chunk_c[..]].concat();
    let file_path = install.join("delta.bin");
    std::fs::write(&file_path, &old_content).unwrap();

    let manifest = DepotManifest::new(vec![file_with_chunks(
        "delta.bin",
        vec![
            chunk_at(chunk_a, 0),
            chunk_at(chunk_b_new, 16),
            chunk_at(chunk_c, 32),
        ],
    )]);

    // Only chunk B is available; A and C must be reused in place from disk.
    let mut allowed = HashMap::new();
    allowed.insert(sha_id(chunk_b_new), enc(chunk_b_new, &key));

    let job = DepotJob::builder()
        .depot_id(anon::SPACEWAR_DEPOT)
        .depot_key(key)
        .install_dir(install.to_path_buf())
        .non_atomic(true)
        .build()
        .unwrap();

    let cp = job.checkpoints();
    let stats = job
        .download(&manifest, Arc::new(SelectiveFetcher { allowed }))
        .await
        .unwrap();

    assert_eq!(stats.files_completed, 1);
    let expected = [&chunk_a[..], &chunk_b_new[..], &chunk_c[..]].concat();
    assert_eq!(std::fs::read(&file_path).unwrap(), expected);
    // A and C reused in place, B refetched.
    assert_eq!(cp.snapshot(), (2, 0, 1));
}

#[tokio::test]
async fn delta_all_chunks_match_skips_all_fetches() {
    let dir = tempfile::tempdir().unwrap();
    let install = dir.path();
    let key = DepotKey([0xAA; 32]);

    let chunk_a = b"AAAAAAAAAAAAAAAA";
    let chunk_b = b"BBBBBBBBBBBBBBBB";

    let content = [&chunk_a[..], &chunk_b[..]].concat();
    std::fs::write(install.join("same.bin"), &content).unwrap();

    let manifest = DepotManifest::new(vec![file_with_chunks(
        "same.bin",
        vec![chunk_at(chunk_a, 0), chunk_at(chunk_b, 16)],
    )]);

    let job = DepotJob::builder()
        .depot_id(anon::SPACEWAR_DEPOT)
        .depot_key(key)
        .install_dir(install.to_path_buf())
        .non_atomic(true)
        .build()
        .unwrap();

    let cp = job.checkpoints();
    let stats = job
        .download(
            &manifest,
            Arc::new(SelectiveFetcher {
                allowed: HashMap::new(),
            }),
        )
        .await
        .unwrap();

    assert_eq!(stats.files_completed, 1);
    assert_eq!(std::fs::read(install.join("same.bin")).unwrap(), content);
    assert_eq!(cp.snapshot(), (2, 0, 0));
}

#[tokio::test]
async fn atomic_delta_reuses_from_installed_file() {
    let dir = tempfile::tempdir().unwrap();
    let install = dir.path();
    let key = DepotKey([0xAA; 32]);

    let chunk_a = b"AAAAAAAAAAAAAAAA";
    let chunk_b_old = b"BBBBBBBBBBBBBBBB";
    let chunk_c = b"CCCCCCCCCCCCCCCC";
    let chunk_b_new = b"bbbbbbbbbbbbbbbb";

    // The currently-installed file (target, not staging).
    let file_path = install.join("game.bin");
    let old_content = [&chunk_a[..], &chunk_b_old[..], &chunk_c[..]].concat();
    std::fs::write(&file_path, &old_content).unwrap();

    let manifest = DepotManifest::new(vec![file_with_chunks(
        "game.bin",
        vec![
            chunk_at(chunk_a, 0),
            chunk_at(chunk_b_new, 16),
            chunk_at(chunk_c, 32),
        ],
    )]);

    // Only chunk B is fetchable; A and C must be copied from the installed file
    // into staging. Default (atomic) mode.
    let mut allowed = HashMap::new();
    allowed.insert(sha_id(chunk_b_new), enc(chunk_b_new, &key));

    let job = DepotJob::builder()
        .depot_id(anon::SPACEWAR_DEPOT)
        .depot_key(key)
        .install_dir(install.to_path_buf())
        .build()
        .unwrap();

    let cp = job.checkpoints();
    let stats = job
        .download(&manifest, Arc::new(SelectiveFetcher { allowed }))
        .await
        .unwrap();

    assert_eq!(stats.files_completed, 1);
    let expected = [&chunk_a[..], &chunk_b_new[..], &chunk_c[..]].concat();
    assert_eq!(std::fs::read(&file_path).unwrap(), expected);
    // A and C copied off disk, B fetched. This is the atomic delta path that
    // previously refetched everything.
    assert_eq!(cp.snapshot(), (0, 2, 1));
}

#[tokio::test]
async fn atomic_delta_all_match_fetches_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let install = dir.path();
    let key = DepotKey([0xAA; 32]);

    let chunk_a = b"AAAAAAAAAAAAAAAA";
    let chunk_b = b"BBBBBBBBBBBBBBBB";
    let content = [&chunk_a[..], &chunk_b[..]].concat();

    let file_path = install.join("game.bin");
    std::fs::write(&file_path, &content).unwrap();

    let manifest = DepotManifest::new(vec![file_with_chunks(
        "game.bin",
        vec![chunk_at(chunk_a, 0), chunk_at(chunk_b, 16)],
    )]);

    let job = DepotJob::builder()
        .depot_id(anon::SPACEWAR_DEPOT)
        .depot_key(key)
        .install_dir(install.to_path_buf())
        .build()
        .unwrap();

    let cp = job.checkpoints();
    let stats = job
        .download(
            &manifest,
            Arc::new(SelectiveFetcher {
                allowed: HashMap::new(),
            }),
        )
        .await
        .unwrap();

    assert_eq!(stats.files_completed, 1);
    assert_eq!(std::fs::read(&file_path).unwrap(), content);
    // Every chunk copied from the installed file; the network is never touched.
    assert_eq!(cp.snapshot(), (0, 2, 0));
}

/// Two distinct byte strings with an identical (Steam) Adler-32 but different
/// SHA-1. Adjusting bytes by +1, -2, +1 at three positions leaves both the
/// running sum and the weighted sum unchanged, so the checksum collides.
fn adler_collision_pair() -> (Vec<u8>, Vec<u8>) {
    let base = vec![10u8; 16];
    let mut other = base.clone();
    other[0] = other[0].wrapping_add(1);
    other[1] = other[1].wrapping_sub(2);
    other[2] = other[2].wrapping_add(1);
    assert_eq!(
        SteamAdler32::compute(&base).0,
        SteamAdler32::compute(&other).0,
        "constructed pair must share an Adler-32"
    );
    assert_ne!(base, other);
    (base, other)
}

#[tokio::test]
async fn reuse_is_gated_on_sha1_not_adler() {
    // Non-heuristic guarantee: a chunk whose weak Adler-32 matches the manifest
    // but whose content differs must NOT be reused. Only the SHA-1 identity
    // decides reuse.
    let dir = tempfile::tempdir().unwrap();
    let install = dir.path();
    let key = DepotKey([0xAA; 32]);

    let (real, on_disk) = adler_collision_pair();
    assert_eq!(
        SteamAdler32::compute(&real).0,
        SteamAdler32::compute(&on_disk).0
    );
    assert_ne!(sha_id(&real), sha_id(&on_disk));

    let file_path = install.join("f.bin");
    std::fs::write(&file_path, &on_disk).unwrap();

    // A correct manifest for `real`: identity and Adler both describe `real`.
    // The on-disk bytes only happen to share the Adler.
    let mut chunk = ManifestChunk::new(
        sha_id(&real),
        SteamAdler32::compute(&real).0,
        real.len() as u32,
    );
    chunk.offset = Some(0);
    let manifest = DepotManifest::new(vec![file_with_chunks("f.bin", vec![chunk])]);

    let mut chunks = HashMap::new();
    chunks.insert(sha_id(&real), enc(&real, &key));

    let job = DepotJob::builder()
        .depot_id(anon::SPACEWAR_DEPOT)
        .depot_key(key)
        .install_dir(install.to_path_buf())
        .non_atomic(true)
        .build()
        .unwrap();

    let cp = job.checkpoints();
    let stats = job
        .download(&manifest, Arc::new(MockFetcher { chunks }))
        .await
        .unwrap();

    assert_eq!(stats.files_completed, 1);
    // The Adler-matching on-disk bytes were rejected and the real content fetched.
    assert_eq!(std::fs::read(&file_path).unwrap(), real);
    assert_eq!(cp.snapshot(), (0, 0, 1));
}

#[tokio::test]
async fn fetched_chunk_failing_sha1_is_rejected() {
    // A CDN that serves bytes matching the manifest size and Adler but not the
    // chunk's SHA-1 identity must be rejected, not written.
    let dir = tempfile::tempdir().unwrap();
    let install = dir.path();
    let key = DepotKey([0xAA; 32]);

    let served = b"corrupt cdn data"; // 16 bytes actually served
    let expected = b"the real content!"; // what the id claims

    // Chunk claims the identity of `expected` but its Adler is that of `served`
    // so process_chunk passes and the SHA-1 gate is what rejects it.
    let mut chunk = ManifestChunk::new(
        sha_id(expected),
        SteamAdler32::compute(served).0,
        served.len() as u32,
    );
    chunk.offset = Some(0);
    let manifest = DepotManifest::new(vec![file_with_chunks("f.bin", vec![chunk])]);

    let mut chunks = HashMap::new();
    // Fetcher answers to the requested id but returns the wrong bytes.
    chunks.insert(sha_id(expected), enc(served, &key));

    let job = DepotJob::builder()
        .depot_id(anon::SPACEWAR_DEPOT)
        .depot_key(key)
        .install_dir(install.to_path_buf())
        .build()
        .unwrap();

    let err = job
        .download(&manifest, Arc::new(MockFetcher { chunks }))
        .await
        .unwrap_err();

    assert!(
        matches!(
            err.current_context(),
            DownloadError::ChunkSha1Mismatch { .. }
        ),
        "expected ChunkSha1Mismatch, got {:?}",
        err.current_context()
    );
    // Atomic mode: the target file is never created from a bad staging file.
    assert!(!install.join("f.bin").exists());
}

#[tokio::test]
async fn non_atomic_writes_directly_to_target() {
    let dir = tempfile::tempdir().unwrap();
    let install = dir.path();
    let key = DepotKey([0xAA; 32]);
    let plaintext = b"direct write test!";

    let mut chunks = HashMap::new();
    chunks.insert(sha_id(plaintext), enc(plaintext, &key));

    let manifest = DepotManifest::new(vec![file_with_chunks(
        "direct.bin",
        vec![chunk_at(plaintext, 0)],
    )]);

    let job = DepotJob::builder()
        .depot_id(anon::SPACEWAR_DEPOT)
        .depot_key(key)
        .install_dir(install.to_path_buf())
        .non_atomic(true)
        .build()
        .unwrap();

    job.download(&manifest, Arc::new(MockFetcher { chunks }))
        .await
        .unwrap();

    assert_eq!(
        std::fs::read(install.join("direct.bin")).unwrap(),
        plaintext
    );
    // No staging directory should exist in non-atomic mode.
    assert!(!install.join(".DepotDownloader").join("staging").exists());
}

#[tokio::test]
async fn atomic_mode_uses_staging_then_renames() {
    let dir = tempfile::tempdir().unwrap();
    let install = dir.path();
    let key = DepotKey([0xAA; 32]);
    let plaintext = b"atomic write test!";

    let mut chunks = HashMap::new();
    chunks.insert(sha_id(plaintext), enc(plaintext, &key));

    let manifest = DepotManifest::new(vec![file_with_chunks(
        "atomic.bin",
        vec![chunk_at(plaintext, 0)],
    )]);

    let job = DepotJob::builder()
        .depot_id(anon::SPACEWAR_DEPOT)
        .depot_key(key)
        .install_dir(install.to_path_buf())
        .build()
        .unwrap();

    job.download(&manifest, Arc::new(MockFetcher { chunks }))
        .await
        .unwrap();

    assert_eq!(
        std::fs::read(install.join("atomic.bin")).unwrap(),
        plaintext
    );
    let staging = install.join(".DepotDownloader").join("staging");
    if staging.exists() {
        assert!(std::fs::read_dir(&staging).unwrap().next().is_none());
    }
}

#[tokio::test]
async fn non_atomic_overwrites_existing_larger_file() {
    let dir = tempfile::tempdir().unwrap();
    let install = dir.path();
    let key = DepotKey([0xAA; 32]);

    // Old file is larger than the new one.
    let old_content = vec![0xFFu8; 1024];
    std::fs::write(install.join("shrink.bin"), &old_content).unwrap();

    let new_data = b"small new file";
    let mut chunks = HashMap::new();
    chunks.insert(sha_id(new_data), enc(new_data, &key));

    let manifest = DepotManifest::new(vec![file_with_chunks(
        "shrink.bin",
        vec![chunk_at(new_data, 0)],
    )]);

    let job = DepotJob::builder()
        .depot_id(anon::SPACEWAR_DEPOT)
        .depot_key(key)
        .install_dir(install.to_path_buf())
        .non_atomic(true)
        .build()
        .unwrap();

    job.download(&manifest, Arc::new(MockFetcher { chunks }))
        .await
        .unwrap();

    // File should be truncated to the new smaller size, not 1024 bytes.
    let result = std::fs::read(install.join("shrink.bin")).unwrap();
    assert_eq!(result, new_data);
    assert_eq!(result.len(), new_data.len());
}

#[tokio::test]
async fn version_update_removes_old_files_and_downloads_new() {
    let dir = tempfile::tempdir().unwrap();
    let install = dir.path();
    let key = DepotKey([0xAA; 32]);

    // Version A files on disk (simulating a previous download).
    std::fs::write(install.join("file_a.bin"), b"file A content!!").unwrap();
    std::fs::write(install.join("shared.bin"), b"shared version 1").unwrap();

    let data_b = b"file B content!!";
    let data_shared_v2 = b"shared version 2";

    let mut chunks = HashMap::new();
    chunks.insert(sha_id(data_b), enc(data_b, &key));
    chunks.insert(sha_id(data_shared_v2), enc(data_shared_v2, &key));

    let manifest_b = DepotManifest::new(vec![
        file_with_chunks("file_b.bin", vec![chunk_at(data_b, 0)]),
        file_with_chunks("shared.bin", vec![chunk_at(data_shared_v2, 0)]),
    ]);

    let old_files = vec!["file_a.bin".to_string(), "shared.bin".to_string()];

    let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel();

    let job = DepotJob::builder()
        .depot_id(anon::SPACEWAR_DEPOT)
        .depot_key(key)
        .install_dir(install.to_path_buf())
        .non_atomic(true)
        .old_manifest_files(old_files)
        .event_sender(event_tx)
        .build()
        .unwrap();

    let stats = job
        .download(&manifest_b, Arc::new(MockFetcher { chunks }))
        .await
        .unwrap();

    assert_eq!(stats.files_completed, 2);
    assert_eq!(stats.files_removed, 1);

    assert!(!install.join("file_a.bin").exists());
    assert_eq!(std::fs::read(install.join("file_b.bin")).unwrap(), data_b);
    assert_eq!(
        std::fs::read(install.join("shared.bin")).unwrap(),
        data_shared_v2
    );

    drop(job);
    let mut removed = vec![];
    while let Ok(event) = event_rx.try_recv() {
        if let DownloadEvent::FileRemoved { filename } = event {
            removed.push(filename);
        }
    }
    assert_eq!(removed, vec!["file_a.bin"]);
}

#[tokio::test]
async fn version_update_no_overlap_removes_all_old_files() {
    let dir = tempfile::tempdir().unwrap();
    let install = dir.path();
    let key = DepotKey([0xAA; 32]);

    std::fs::write(install.join("old_1.bin"), b"old file 1").unwrap();
    std::fs::write(install.join("old_2.bin"), b"old file 2").unwrap();

    let data_new = b"new file content";
    let mut chunks = HashMap::new();
    chunks.insert(sha_id(data_new), enc(data_new, &key));

    let manifest_b = DepotManifest::new(vec![file_with_chunks(
        "new.bin",
        vec![chunk_at(data_new, 0)],
    )]);

    let old_files = vec!["old_1.bin".to_string(), "old_2.bin".to_string()];

    let job = DepotJob::builder()
        .depot_id(anon::SPACEWAR_DEPOT)
        .depot_key(key)
        .install_dir(install.to_path_buf())
        .non_atomic(true)
        .old_manifest_files(old_files)
        .build()
        .unwrap();

    let stats = job
        .download(&manifest_b, Arc::new(MockFetcher { chunks }))
        .await
        .unwrap();

    assert_eq!(stats.files_completed, 1);
    assert_eq!(stats.files_removed, 2);
    assert!(!install.join("old_1.bin").exists());
    assert!(!install.join("old_2.bin").exists());
    assert_eq!(std::fs::read(install.join("new.bin")).unwrap(), data_new);
}

#[tokio::test]
async fn empty_file_is_created() {
    let dir = tempfile::tempdir().unwrap();
    let install = dir.path();

    let manifest = manifest_with(&["empty.txt"]);

    let job = DepotJob::builder()
        .depot_id(anon::SPACEWAR_DEPOT)
        .depot_key(DepotKey([0; 32]))
        .install_dir(install.to_path_buf())
        .build()
        .unwrap();

    let stats = job
        .download(&manifest, Arc::new(NullFetcher))
        .await
        .unwrap();

    assert_eq!(stats.files_completed, 1);
    let path = install.join("empty.txt");
    assert!(path.exists());
    assert_eq!(std::fs::metadata(&path).unwrap().len(), 0);
}

#[tokio::test]
async fn empty_file_truncates_stale_content() {
    // A file that used to have content but is now empty must be truncated,
    // even though verify is off.
    let dir = tempfile::tempdir().unwrap();
    let install = dir.path();
    std::fs::write(install.join("was_big.txt"), b"old non-empty content").unwrap();

    let manifest = manifest_with(&["was_big.txt"]);

    let job = DepotJob::builder()
        .depot_id(anon::SPACEWAR_DEPOT)
        .depot_key(DepotKey([0; 32]))
        .install_dir(install.to_path_buf())
        .build()
        .unwrap();

    job.download(&manifest, Arc::new(NullFetcher))
        .await
        .unwrap();

    assert_eq!(
        std::fs::metadata(install.join("was_big.txt"))
            .unwrap()
            .len(),
        0
    );
}

#[tokio::test]
async fn empty_file_verify_truncates_stale_nonempty() {
    // In verify mode a stale, non-empty file that should now be empty must be
    // rewritten, not skipped just because it exists.
    let dir = tempfile::tempdir().unwrap();
    let install = dir.path();
    std::fs::write(install.join("f.txt"), b"stale bytes").unwrap();

    let manifest = manifest_with(&["f.txt"]);

    let job = DepotJob::builder()
        .depot_id(anon::SPACEWAR_DEPOT)
        .depot_key(DepotKey([0; 32]))
        .install_dir(install.to_path_buf())
        .verify(true)
        .build()
        .unwrap();

    let stats = job
        .download(&manifest, Arc::new(NullFetcher))
        .await
        .unwrap();

    assert_eq!(stats.files_completed, 1);
    assert_eq!(stats.files_skipped, 0);
    assert_eq!(std::fs::metadata(install.join("f.txt")).unwrap().len(), 0);
}

#[tokio::test]
async fn empty_file_verify_skips_already_empty() {
    let dir = tempfile::tempdir().unwrap();
    let install = dir.path();
    std::fs::write(install.join("f.txt"), b"").unwrap();

    let manifest = manifest_with(&["f.txt"]);

    let job = DepotJob::builder()
        .depot_id(anon::SPACEWAR_DEPOT)
        .depot_key(DepotKey([0; 32]))
        .install_dir(install.to_path_buf())
        .verify(true)
        .build()
        .unwrap();

    let stats = job
        .download(&manifest, Arc::new(NullFetcher))
        .await
        .unwrap();

    assert_eq!(stats.files_skipped, 1);
    assert_eq!(stats.files_completed, 0);
}

#[tokio::test]
async fn file_that_was_empty_now_has_content() {
    let dir = tempfile::tempdir().unwrap();
    let install = dir.path();
    let key = DepotKey([0xAA; 32]);

    // Previously empty.
    std::fs::write(install.join("grow.bin"), b"").unwrap();

    let data = b"now it has content";
    let mut chunks = HashMap::new();
    chunks.insert(sha_id(data), enc(data, &key));

    let manifest = DepotManifest::new(vec![file_with_chunks("grow.bin", vec![chunk_at(data, 0)])]);

    let job = DepotJob::builder()
        .depot_id(anon::SPACEWAR_DEPOT)
        .depot_key(key)
        .install_dir(install.to_path_buf())
        .non_atomic(true)
        .build()
        .unwrap();

    let cp = job.checkpoints();
    job.download(&manifest, Arc::new(MockFetcher { chunks }))
        .await
        .unwrap();

    assert_eq!(std::fs::read(install.join("grow.bin")).unwrap(), data);
    assert_eq!(cp.snapshot(), (0, 0, 1));
}

#[tokio::test]
async fn empty_directory_entry_is_created() {
    let dir = tempfile::tempdir().unwrap();
    let install = dir.path();

    let mut d = ManifestFile::new("emptydir".to_string(), 0);
    d.flags = DepotFileFlags::DIRECTORY.bits();
    let manifest = DepotManifest::new(vec![d]);

    let job = DepotJob::builder()
        .depot_id(anon::SPACEWAR_DEPOT)
        .depot_key(DepotKey([0; 32]))
        .install_dir(install.to_path_buf())
        .build()
        .unwrap();

    job.download(&manifest, Arc::new(NullFetcher))
        .await
        .unwrap();

    let path = install.join("emptydir");
    assert!(path.is_dir(), "empty directory entry should be created");
}

#[cfg(unix)]
#[tokio::test]
async fn symlink_is_created_not_written_as_empty_file() {
    let dir = tempfile::tempdir().unwrap();
    let install = dir.path();

    let mut link = ManifestFile::new("link".to_string(), 0);
    link.flags = DepotFileFlags::SYMLINK.bits();
    link.link_target = Some("target/real.bin".to_string());
    let manifest = DepotManifest::new(vec![link]);

    let job = DepotJob::builder()
        .depot_id(anon::SPACEWAR_DEPOT)
        .depot_key(DepotKey([0; 32]))
        .install_dir(install.to_path_buf())
        .build()
        .unwrap();

    let stats = job
        .download(&manifest, Arc::new(NullFetcher))
        .await
        .unwrap();

    assert_eq!(stats.files_completed, 1);
    let path = install.join("link");
    let meta = std::fs::symlink_metadata(&path).unwrap();
    assert!(
        meta.file_type().is_symlink(),
        "must be a symlink, not a file"
    );
    assert_eq!(
        std::fs::read_link(&path).unwrap(),
        std::path::PathBuf::from("target/real.bin")
    );
}

#[cfg(unix)]
#[tokio::test]
async fn symlink_replaces_existing_regular_file() {
    let dir = tempfile::tempdir().unwrap();
    let install = dir.path();

    // A regular file previously sat where the symlink now belongs.
    std::fs::write(install.join("link"), b"was a real file").unwrap();

    let mut link = ManifestFile::new("link".to_string(), 0);
    link.flags = DepotFileFlags::SYMLINK.bits();
    link.link_target = Some("elsewhere".to_string());
    let manifest = DepotManifest::new(vec![link]);

    let job = DepotJob::builder()
        .depot_id(anon::SPACEWAR_DEPOT)
        .depot_key(DepotKey([0; 32]))
        .install_dir(install.to_path_buf())
        .build()
        .unwrap();

    job.download(&manifest, Arc::new(NullFetcher))
        .await
        .unwrap();

    let meta = std::fs::symlink_metadata(install.join("link")).unwrap();
    assert!(meta.file_type().is_symlink());
}

#[tokio::test]
async fn content_addressed_reuse_follows_moved_chunk() {
    // An update prepends a new block, shifting an unchanged chunk to a later
    // offset. Positional reuse would miss it; content-addressed reuse copies it
    // from its old offset in the installed file.
    let dir = tempfile::tempdir().unwrap();
    let install = dir.path();
    let key = DepotKey([0xAA; 32]);

    let head_new = b"NEW HEADER BLOCK"; // inserted at the front (fetched)
    let moved = b"unchanged payload"; // 17 bytes, was at offset 0, now after head

    // Installed file: just `moved` at offset 0.
    let file_path = install.join("game.bin");
    std::fs::write(&file_path, moved).unwrap();

    // Old layout: `moved` lived at offset 0 in the installed file.
    let mut old_layouts = HashMap::new();
    old_layouts.insert(
        "game.bin".to_string(),
        vec![crate::download::OldChunkLoc {
            id: sha_id(moved),
            offset: 0,
            size: moved.len() as u32,
        }],
    );

    // New manifest: head at 0, moved at head.len().
    let manifest = DepotManifest::new(vec![file_with_chunks(
        "game.bin",
        vec![
            chunk_at(head_new, 0),
            chunk_at(moved, head_new.len() as u64),
        ],
    )]);

    // Only the header is fetchable; `moved` must be copied from its old offset.
    let mut allowed = HashMap::new();
    allowed.insert(sha_id(head_new), enc(head_new, &key));

    let job = DepotJob::builder()
        .depot_id(anon::SPACEWAR_DEPOT)
        .depot_key(key)
        .install_dir(install.to_path_buf())
        .old_file_layouts(old_layouts)
        .build()
        .unwrap();

    let cp = job.checkpoints();
    let stats = job
        .download(&manifest, Arc::new(SelectiveFetcher { allowed }))
        .await
        .unwrap();

    assert_eq!(stats.files_completed, 1);
    let expected = [&head_new[..], &moved[..]].concat();
    assert_eq!(std::fs::read(&file_path).unwrap(), expected);
    // Header fetched, moved chunk copied from its shifted old location.
    assert_eq!(cp.snapshot(), (0, 1, 1));
}

#[tokio::test]
async fn content_addressed_reuse_copies_across_files() {
    // A chunk that the new manifest places in file B already exists in a
    // different old file A on disk. The global CAS must copy it from A rather
    // than fetch it.
    let dir = tempfile::tempdir().unwrap();
    let install = dir.path();
    let key = DepotKey([0xAA; 32]);

    let shared = b"shared depot blk"; // 16 bytes, present in old file A
    let b_only = b"b-specific block"; // fetched

    // Old file A holds `shared`; it is not in the new manifest at all.
    std::fs::write(install.join("a.bin"), shared).unwrap();

    let mut old_layouts = HashMap::new();
    old_layouts.insert(
        "a.bin".to_string(),
        vec![crate::download::OldChunkLoc {
            id: sha_id(shared),
            offset: 0,
            size: shared.len() as u32,
        }],
    );

    // New manifest: only file b.bin, which reuses `shared` then adds `b_only`.
    let manifest = DepotManifest::new(vec![file_with_chunks(
        "b.bin",
        vec![chunk_at(shared, 0), chunk_at(b_only, shared.len() as u64)],
    )]);

    let mut allowed = HashMap::new();
    allowed.insert(sha_id(b_only), enc(b_only, &key));

    let job = DepotJob::builder()
        .depot_id(anon::SPACEWAR_DEPOT)
        .depot_key(key)
        .install_dir(install.to_path_buf())
        .old_file_layouts(old_layouts)
        .build()
        .unwrap();

    let cp = job.checkpoints();
    let stats = job
        .download(&manifest, Arc::new(SelectiveFetcher { allowed }))
        .await
        .unwrap();

    assert_eq!(stats.files_completed, 1);
    let expected = [&shared[..], &b_only[..]].concat();
    assert_eq!(std::fs::read(install.join("b.bin")).unwrap(), expected);
    // `shared` copied from a.bin, `b_only` fetched.
    assert_eq!(cp.snapshot(), (0, 1, 1));
}

#[tokio::test]
async fn evicted_cas_source_falls_back_to_fetch() {
    // The CAS claims a chunk lives in a file that, on disk, no longer holds
    // those bytes (evicted/overwritten). Verification must fail and the chunk
    // must be fetched, never copied blindly.
    let dir = tempfile::tempdir().unwrap();
    let install = dir.path();
    let key = DepotKey([0xAA; 32]);

    let wanted = b"the wanted block"; // what the manifest wants
    let stale = b"STALE OTHER BYTES"; // what a.bin actually holds now (17 bytes)

    // a.bin exists but its bytes do not match the CAS entry's claimed identity.
    std::fs::write(install.join("a.bin"), stale).unwrap();

    let mut old_layouts = HashMap::new();
    old_layouts.insert(
        "a.bin".to_string(),
        vec![crate::download::OldChunkLoc {
            id: sha_id(wanted), // CAS claims a.bin@0 is `wanted`...
            offset: 0,
            size: wanted.len() as u32,
        }],
    );

    let manifest = DepotManifest::new(vec![file_with_chunks("b.bin", vec![chunk_at(wanted, 0)])]);

    let mut chunks = HashMap::new();
    chunks.insert(sha_id(wanted), enc(wanted, &key));

    let job = DepotJob::builder()
        .depot_id(anon::SPACEWAR_DEPOT)
        .depot_key(key)
        .install_dir(install.to_path_buf())
        .old_file_layouts(old_layouts)
        .build()
        .unwrap();

    let cp = job.checkpoints();
    job.download(&manifest, Arc::new(MockFetcher { chunks }))
        .await
        .unwrap();

    assert_eq!(std::fs::read(install.join("b.bin")).unwrap(), wanted);
    // The stale source was rejected by SHA-1; the chunk was fetched.
    assert_eq!(cp.snapshot(), (0, 0, 1));
}

#[cfg(unix)]
#[tokio::test]
async fn executable_flag_sets_exec_bit_on_download() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let install = dir.path();
    let key = DepotKey([0xAA; 32]);
    let data = b"#!/bin/sh\necho hello\n";

    let mut chunks = HashMap::new();
    chunks.insert(sha_id(data), enc(data, &key));

    let mut f = file_with_chunks("run.sh", vec![chunk_at(data, 0)]);
    f.flags = DepotFileFlags::EXECUTABLE.bits();
    let manifest = DepotManifest::new(vec![f]);

    let job = DepotJob::builder()
        .depot_id(anon::SPACEWAR_DEPOT)
        .depot_key(key)
        .install_dir(install.to_path_buf())
        .build()
        .unwrap();

    job.download(&manifest, Arc::new(MockFetcher { chunks }))
        .await
        .unwrap();

    let mode = std::fs::metadata(install.join("run.sh"))
        .unwrap()
        .permissions()
        .mode();
    assert!(mode & 0o111 != 0, "expected exec bits, got mode {mode:o}");
}

#[cfg(unix)]
#[tokio::test]
async fn verify_sets_missing_exec_bit_on_matching_file() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let install = dir.path();
    let data = b"#!/bin/sh\necho hello\n";

    // Content already correct, but not marked executable on disk.
    std::fs::write(install.join("run.sh"), data).unwrap();
    std::fs::set_permissions(
        install.join("run.sh"),
        std::fs::Permissions::from_mode(0o644),
    )
    .unwrap();

    let mut f = file_with_chunks("run.sh", vec![chunk_at(data, 0)]);
    f.flags = DepotFileFlags::EXECUTABLE.bits();
    f.sha_content = Some(Sha1Hash::compute(data).0);
    let manifest = DepotManifest::new(vec![f]);

    let job = DepotJob::builder()
        .depot_id(anon::SPACEWAR_DEPOT)
        .depot_key(DepotKey([0; 32]))
        .install_dir(install.to_path_buf())
        .verify(true)
        .build()
        .unwrap();

    let stats = job
        .download(&manifest, Arc::new(NullFetcher))
        .await
        .unwrap();

    // Content matched, so the download is skipped, but the flag is repaired.
    assert_eq!(stats.files_skipped, 1);
    let mode = std::fs::metadata(install.join("run.sh"))
        .unwrap()
        .permissions()
        .mode();
    assert!(mode & 0o111 != 0, "verify should set exec bits, got {mode:o}");
}

#[cfg(unix)]
#[tokio::test]
async fn verify_strips_exec_bit_when_not_flagged() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let install = dir.path();
    let data = b"plain data file.";

    std::fs::write(install.join("data.bin"), data).unwrap();
    std::fs::set_permissions(
        install.join("data.bin"),
        std::fs::Permissions::from_mode(0o755),
    )
    .unwrap();

    // Not flagged executable: verify should make the file match the manifest.
    let mut f = file_with_chunks("data.bin", vec![chunk_at(data, 0)]);
    f.sha_content = Some(Sha1Hash::compute(data).0);
    let manifest = DepotManifest::new(vec![f]);

    let job = DepotJob::builder()
        .depot_id(anon::SPACEWAR_DEPOT)
        .depot_key(DepotKey([0; 32]))
        .install_dir(install.to_path_buf())
        .verify(true)
        .build()
        .unwrap();

    job.download(&manifest, Arc::new(NullFetcher))
        .await
        .unwrap();

    let mode = std::fs::metadata(install.join("data.bin"))
        .unwrap()
        .permissions()
        .mode();
    assert_eq!(mode & 0o111, 0, "verify should strip exec bits, got {mode:o}");
}

#[cfg(unix)]
#[tokio::test]
async fn stale_symlink_replaced_by_regular_empty_file() {
    // A path that was a symlink in the old version is now a regular empty file.
    // The link must be removed, not written through.
    let dir = tempfile::tempdir().unwrap();
    let install = dir.path();

    std::fs::write(install.join("target"), b"link target contents").unwrap();
    std::os::unix::fs::symlink("target", install.join("was_link")).unwrap();

    let manifest = manifest_with(&["was_link"]);

    let job = DepotJob::builder()
        .depot_id(anon::SPACEWAR_DEPOT)
        .depot_key(DepotKey([0; 32]))
        .install_dir(install.to_path_buf())
        .verify(true)
        .build()
        .unwrap();

    let stats = job
        .download(&manifest, Arc::new(NullFetcher))
        .await
        .unwrap();

    // Not skipped despite the link resolving to a non-empty file.
    assert_eq!(stats.files_skipped, 0);
    let meta = std::fs::symlink_metadata(install.join("was_link")).unwrap();
    assert!(meta.file_type().is_file(), "must be a regular file now");
    assert_eq!(meta.len(), 0);
    // The link target must be untouched (not truncated through the link).
    assert_eq!(
        std::fs::read(install.join("target")).unwrap(),
        b"link target contents"
    );
}

#[tokio::test]
async fn atomic_resume_reuses_staging_and_installed_file() {
    // Crash-resume in atomic mode: a partial staging file exists AND an older
    // installed file exists. Chunks already staged are reused in place, chunks
    // unchanged from the installed file are copied, only the rest are fetched.
    let dir = tempfile::tempdir().unwrap();
    let install = dir.path();
    let key = DepotKey([0xAA; 32]);

    let chunk_a = b"AAAAAAAAAAAAAAAA"; // already correct in staging
    let chunk_b = b"BBBBBBBBBBBBBBBB"; // unchanged from installed file
    let chunk_c = b"CCCCCCCCCCCCCCCC"; // changed: must be fetched
    let chunk_c_old = b"cccccccccccccccc"; // old version in installed file

    // Installed file: a, b, c_old.
    let file_path = install.join("game.bin");
    std::fs::write(
        &file_path,
        [&chunk_a[..], &chunk_b[..], &chunk_c_old[..]].concat(),
    )
    .unwrap();

    // Partial staging file: chunk a already written at offset 0.
    let staging_dir = install.join(".DepotDownloader").join("staging");
    std::fs::create_dir_all(&staging_dir).unwrap();
    std::fs::write(staging_dir.join("game.bin"), chunk_a).unwrap();

    let manifest = DepotManifest::new(vec![file_with_chunks(
        "game.bin",
        vec![
            chunk_at(chunk_a, 0),
            chunk_at(chunk_b, 16),
            chunk_at(chunk_c, 32),
        ],
    )]);

    // Only C is fetchable; A must come from staging, B from the installed file.
    let mut allowed = HashMap::new();
    allowed.insert(sha_id(chunk_c), enc(chunk_c, &key));

    let job = DepotJob::builder()
        .depot_id(anon::SPACEWAR_DEPOT)
        .depot_key(key)
        .install_dir(install.to_path_buf())
        .build()
        .unwrap();

    let cp = job.checkpoints();
    let stats = job
        .download(&manifest, Arc::new(SelectiveFetcher { allowed }))
        .await
        .unwrap();

    assert_eq!(stats.files_completed, 1);
    let expected = [&chunk_a[..], &chunk_b[..], &chunk_c[..]].concat();
    assert_eq!(std::fs::read(&file_path).unwrap(), expected);
    // A in place (staging), B copied (installed), C fetched.
    assert_eq!(cp.snapshot(), (1, 1, 1));
}

#[tokio::test]
async fn delta_prunes_empty_parent_dirs_of_removed_files() {
    let dir = tempfile::tempdir().unwrap();
    let install = dir.path();

    let nested = install.join("bin").join("12345").join("idx");
    std::fs::create_dir_all(&nested).unwrap();
    std::fs::write(nested.join("old.idx"), b"old").unwrap();

    let old_files = vec!["bin/12345/idx/old.idx".to_string()];
    let new_manifest = manifest_with(&["other.txt"]);

    let job = DepotJob::builder()
        .depot_id(anon::SPACEWAR_DEPOT)
        .depot_key(DepotKey([0; 32]))
        .install_dir(install.to_path_buf())
        .old_manifest_files(old_files)
        .build()
        .unwrap();

    let stats = job
        .download(&new_manifest, Arc::new(NullFetcher))
        .await
        .unwrap();

    assert_eq!(stats.files_removed, 1);
    assert!(!nested.join("old.idx").exists());
    assert!(!nested.exists(), "idx/ dir should be removed");
    assert!(
        !install.join("bin").join("12345").exists(),
        "12345/ dir should be removed"
    );
    assert!(!install.join("bin").exists(), "bin/ dir should be removed");
}

#[tokio::test]
async fn delta_prune_does_not_remove_dirs_with_remaining_files() {
    let dir = tempfile::tempdir().unwrap();
    let install = dir.path();

    let build_dir = install.join("bin").join("12345").join("idx");
    std::fs::create_dir_all(&build_dir).unwrap();
    std::fs::write(build_dir.join("removed.idx"), b"gone").unwrap();
    std::fs::write(build_dir.join("kept.idx"), b"stay").unwrap();

    let old_files = vec![
        "bin/12345/idx/removed.idx".to_string(),
        "bin/12345/idx/kept.idx".to_string(),
    ];
    let new_manifest = manifest_with(&["bin\\12345\\idx\\kept.idx"]);

    let job = DepotJob::builder()
        .depot_id(anon::SPACEWAR_DEPOT)
        .depot_key(DepotKey([0; 32]))
        .install_dir(install.to_path_buf())
        .old_manifest_files(old_files)
        .build()
        .unwrap();

    let stats = job
        .download(&new_manifest, Arc::new(NullFetcher))
        .await
        .unwrap();

    assert_eq!(stats.files_removed, 1);
    assert!(!build_dir.join("removed.idx").exists());
    assert!(build_dir.join("kept.idx").exists());
    assert!(build_dir.exists());
    assert!(install.join("bin").join("12345").exists());
}

#[tokio::test]
async fn delta_prune_does_not_touch_user_dirs() {
    let dir = tempfile::tempdir().unwrap();
    let install = dir.path();

    let old_dir = install.join("bin").join("old");
    std::fs::create_dir_all(&old_dir).unwrap();
    std::fs::write(old_dir.join("data.bin"), b"old").unwrap();

    let user_dir = install.join("my_stuff");
    std::fs::create_dir_all(&user_dir).unwrap();
    std::fs::write(user_dir.join("notes.txt"), b"user file").unwrap();

    let old_files = vec!["bin/old/data.bin".to_string()];
    let new_manifest = manifest_with(&["new.txt"]);

    let job = DepotJob::builder()
        .depot_id(anon::SPACEWAR_DEPOT)
        .depot_key(DepotKey([0; 32]))
        .install_dir(install.to_path_buf())
        .old_manifest_files(old_files)
        .build()
        .unwrap();

    job.download(&new_manifest, Arc::new(NullFetcher))
        .await
        .unwrap();

    assert!(!old_dir.exists());
    assert!(!install.join("bin").exists());
    assert!(user_dir.exists());
    assert!(user_dir.join("notes.txt").exists());
}
