use crate::download::BoxError;
use crate::download::ChunkFetcher;
use crate::download::DepotJob;
use crate::download::FileFilter;
use crate::event::DownloadEvent;
use bytes::Bytes;
use std::collections::HashMap;
use std::sync::Arc;
use steamroom::depot::ChunkId;
use steamroom::depot::DepotId;
use steamroom::depot::DepotKey;
use steamroom::depot::manifest::DepotManifest;
use steamroom::depot::manifest::ManifestChunk;
use steamroom::depot::manifest::ManifestFile;
use steamroom::util::checksum::SteamAdler32;

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
    let checksum = SteamAdler32::compute(plaintext);

    let chunk_id = ChunkId([1; 20]);
    let encrypted = encrypt_chunk(plaintext, &key);

    let mut chunks = HashMap::new();
    chunks.insert(chunk_id.clone(), Bytes::from(encrypted));

    let mut chunk = ManifestChunk::new(chunk_id, checksum.0, plaintext.len() as u32);
    chunk.offset = Some(0);
    let manifest = DepotManifest::new(vec![file_with_chunks("test.txt", vec![chunk])]);

    let job = DepotJob::builder()
        .depot_id(DepotId(481))
        .depot_key(key)
        .install_dir(install.to_path_buf())
        .build()
        .unwrap();

    let stats = job
        .download(&manifest, Arc::new(MockFetcher { chunks }))
        .await
        .unwrap();

    assert_eq!(stats.files_completed, 1);
    assert_eq!(std::fs::read(install.join("test.txt")).unwrap(), plaintext);
}

#[tokio::test]
async fn download_multi_chunk_file_reassembles_in_order() {
    let dir = tempfile::tempdir().unwrap();
    let install = dir.path();
    let key = DepotKey([0xBB; 32]);

    let part_a = b"AAAA";
    let part_b = b"BBBB";
    let combined: Vec<u8> = [&part_a[..], &part_b[..]].concat();

    let id_a = ChunkId([0xA0; 20]);
    let id_b = ChunkId([0xB0; 20]);

    let mut chunks = HashMap::new();
    chunks.insert(id_a.clone(), Bytes::from(encrypt_chunk(part_a, &key)));
    chunks.insert(id_b.clone(), Bytes::from(encrypt_chunk(part_b, &key)));

    let mut chunk_a =
        ManifestChunk::new(id_a, SteamAdler32::compute(part_a).0, part_a.len() as u32);
    chunk_a.offset = Some(0);
    let mut chunk_b =
        ManifestChunk::new(id_b, SteamAdler32::compute(part_b).0, part_b.len() as u32);
    chunk_b.offset = Some(part_a.len() as u64);
    let manifest = DepotManifest::new(vec![file_with_chunks("multi.bin", vec![chunk_a, chunk_b])]);

    let job = DepotJob::builder()
        .depot_id(DepotId(481))
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
        .depot_id(DepotId(481))
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
    let checksum = SteamAdler32::compute(plaintext);
    let chunk_id = ChunkId([0xEE; 20]);

    let mut chunks = HashMap::new();
    chunks.insert(
        chunk_id.clone(),
        Bytes::from(encrypt_chunk(plaintext, &key)),
    );

    let mut chunk = ManifestChunk::new(chunk_id, checksum.0, plaintext.len() as u32);
    chunk.offset = Some(0);
    let manifest = DepotManifest::new(vec![file_with_chunks("evented.bin", vec![chunk])]);

    let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel();

    let job = DepotJob::builder()
        .depot_id(DepotId(481))
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
        .depot_id(DepotId(481))
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
        .depot_id(DepotId(481))
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
        .depot_id(DepotId(481))
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
        .depot_id(DepotId(481))
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
        .depot_id(DepotId(481))
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
        "game\\bin\\old.dll".to_string(),
        "game\\bin\\keep.dll".to_string(),
    ];
    let new_manifest = manifest_with(&["game\\bin\\keep.dll"]);

    let job = DepotJob::builder()
        .depot_id(DepotId(481))
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

    let id_a = ChunkId([0xA0; 20]);
    let id_b = ChunkId([0xB0; 20]);

    let mut chunks = HashMap::new();
    chunks.insert(
        id_a.clone(),
        Bytes::from(encrypt_chunk(chunk_a_plain, &key)),
    );
    chunks.insert(
        id_b.clone(),
        Bytes::from(encrypt_chunk(chunk_b_plain, &key)),
    );

    let chunk_a = ManifestChunk::new(
        id_a,
        SteamAdler32::compute(chunk_a_plain).0,
        chunk_a_plain.len() as u32,
    );
    let chunk_b = ManifestChunk::new(
        id_b,
        SteamAdler32::compute(chunk_b_plain).0,
        chunk_b_plain.len() as u32,
    );
    let manifest = DepotManifest::new(vec![file_with_chunks("resume.bin", vec![chunk_a, chunk_b])]);

    // Simulate an interrupted download: chunk A fully written + 5 garbage bytes
    // from a partially-written chunk B
    let staging_dir = install.join(".depotdownloader").join("staging");
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
        .depot_id(DepotId(481))
        .depot_key(key)
        .install_dir(install.to_path_buf())
        .build()
        .unwrap();

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
}

#[tokio::test]
async fn resume_skips_fully_staged_chunks() {
    let dir = tempfile::tempdir().unwrap();
    let install = dir.path();
    let key = DepotKey([0xAA; 32]);

    let chunk_a_plain = b"AAAAAAAAAAAAAAAA";
    let chunk_b_plain = b"BBBBBBBBBBBBBBBB";
    let combined = [&chunk_a_plain[..], &chunk_b_plain[..]].concat();

    let id_a = ChunkId([0xA0; 20]);
    let id_b = ChunkId([0xB0; 20]);

    // Only chunk B in the mock — chunk A should be skipped via resume
    let mut chunks = HashMap::new();
    chunks.insert(
        id_b.clone(),
        Bytes::from(encrypt_chunk(chunk_b_plain, &key)),
    );

    let chunk_a = ManifestChunk::new(
        id_a,
        SteamAdler32::compute(chunk_a_plain).0,
        chunk_a_plain.len() as u32,
    );
    let chunk_b = ManifestChunk::new(
        id_b,
        SteamAdler32::compute(chunk_b_plain).0,
        chunk_b_plain.len() as u32,
    );
    let manifest = DepotManifest::new(vec![file_with_chunks(
        "resume2.bin",
        vec![chunk_a, chunk_b],
    )]);

    // Pre-stage chunk A exactly (no partial data)
    let staging_dir = install.join(".depotdownloader").join("staging");
    std::fs::create_dir_all(&staging_dir).unwrap();
    let staging_path = staging_dir.join("resume2.bin");
    std::fs::write(&staging_path, chunk_a_plain).unwrap();

    let job = DepotJob::builder()
        .depot_id(DepotId(481))
        .depot_key(key)
        .install_dir(install.to_path_buf())
        .build()
        .unwrap();

    let stats = job
        .download(&manifest, Arc::new(MockFetcher { chunks }))
        .await
        .unwrap();

    assert_eq!(stats.files_completed, 1);
    let result = std::fs::read(install.join("resume2.bin")).unwrap();
    assert_eq!(result, combined);
}

/// A fetcher that only serves specific chunk IDs and panics on anything else.
/// Used to verify that reusable chunks are NOT fetched from CDN.
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

#[tokio::test]
async fn delta_reuses_unchanged_chunks() {
    let dir = tempfile::tempdir().unwrap();
    let install = dir.path();
    let key = DepotKey([0xAA; 32]);

    let chunk_a = b"AAAAAAAAAAAAAAAA"; // unchanged
    let chunk_b_old = b"BBBBBBBBBBBBBBBB"; // will change
    let chunk_c = b"CCCCCCCCCCCCCCCC"; // unchanged
    let chunk_b_new = b"bbbbbbbbbbbbbbbb"; // new version

    let id_a = ChunkId([0xA0; 20]);
    let id_b = ChunkId([0xB0; 20]);
    let id_c = ChunkId([0xC0; 20]);

    // Write the "old" version of the file
    let old_content = [&chunk_a[..], &chunk_b_old[..], &chunk_c[..]].concat();
    let file_path = install.join("delta.bin");
    std::fs::write(&file_path, &old_content).unwrap();

    // New manifest: chunk A and C are the same, chunk B changed
    let mc_a = ManifestChunk::new(
        id_a.clone(),
        SteamAdler32::compute(chunk_a).0,
        chunk_a.len() as u32,
    );
    let mc_b = ManifestChunk::new(
        id_b.clone(),
        SteamAdler32::compute(chunk_b_new).0,
        chunk_b_new.len() as u32,
    );
    let mc_c = ManifestChunk::new(
        id_c.clone(),
        SteamAdler32::compute(chunk_c).0,
        chunk_c.len() as u32,
    );

    let manifest = DepotManifest::new(vec![file_with_chunks("delta.bin", vec![mc_a, mc_b, mc_c])]);

    // Only provide chunk B in the fetcher — A and C must be reused from disk.
    // If the pipeline tries to fetch A or C, SelectiveFetcher will panic.
    let mut allowed = HashMap::new();
    allowed.insert(id_b, Bytes::from(encrypt_chunk(chunk_b_new, &key)));

    let job = DepotJob::builder()
        .depot_id(DepotId(481))
        .depot_key(key)
        .install_dir(install.to_path_buf())
        .non_atomic(true)
        .build()
        .unwrap();

    let stats = job
        .download(&manifest, Arc::new(SelectiveFetcher { allowed }))
        .await
        .unwrap();

    assert_eq!(stats.files_completed, 1);
    let expected = [&chunk_a[..], &chunk_b_new[..], &chunk_c[..]].concat();
    assert_eq!(std::fs::read(&file_path).unwrap(), expected);
}

#[tokio::test]
async fn delta_all_chunks_match_skips_all_fetches() {
    let dir = tempfile::tempdir().unwrap();
    let install = dir.path();
    let key = DepotKey([0xAA; 32]);

    let chunk_a = b"AAAAAAAAAAAAAAAA";
    let chunk_b = b"BBBBBBBBBBBBBBBB";

    let id_a = ChunkId([0xA0; 20]);
    let id_b = ChunkId([0xB0; 20]);

    // Write file that already matches the manifest
    let content = [&chunk_a[..], &chunk_b[..]].concat();
    std::fs::write(install.join("same.bin"), &content).unwrap();

    let mc_a = ManifestChunk::new(id_a, SteamAdler32::compute(chunk_a).0, chunk_a.len() as u32);
    let mc_b = ManifestChunk::new(id_b, SteamAdler32::compute(chunk_b).0, chunk_b.len() as u32);

    let manifest = DepotManifest::new(vec![file_with_chunks("same.bin", vec![mc_a, mc_b])]);

    // Empty fetcher — any fetch attempt = panic
    let job = DepotJob::builder()
        .depot_id(DepotId(481))
        .depot_key(key)
        .install_dir(install.to_path_buf())
        .non_atomic(true)
        .build()
        .unwrap();

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
}

#[tokio::test]
async fn non_atomic_writes_directly_to_target() {
    let dir = tempfile::tempdir().unwrap();
    let install = dir.path();
    let key = DepotKey([0xAA; 32]);
    let plaintext = b"direct write test!";
    let checksum = SteamAdler32::compute(plaintext);
    let chunk_id = ChunkId([0xDD; 20]);

    let mut chunks = HashMap::new();
    chunks.insert(
        chunk_id.clone(),
        Bytes::from(encrypt_chunk(plaintext, &key)),
    );

    let mc = ManifestChunk::new(chunk_id, checksum.0, plaintext.len() as u32);
    let manifest = DepotManifest::new(vec![file_with_chunks("direct.bin", vec![mc])]);

    let job = DepotJob::builder()
        .depot_id(DepotId(481))
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
    // No staging directory should exist in non-atomic mode
    assert!(!install.join(".depotdownloader").join("staging").exists());
}

#[tokio::test]
async fn atomic_mode_uses_staging_then_renames() {
    let dir = tempfile::tempdir().unwrap();
    let install = dir.path();
    let key = DepotKey([0xAA; 32]);
    let plaintext = b"atomic write test!";
    let checksum = SteamAdler32::compute(plaintext);
    let chunk_id = ChunkId([0xEE; 20]);

    let mut chunks = HashMap::new();
    chunks.insert(
        chunk_id.clone(),
        Bytes::from(encrypt_chunk(plaintext, &key)),
    );

    let mc = ManifestChunk::new(chunk_id, checksum.0, plaintext.len() as u32);
    let manifest = DepotManifest::new(vec![file_with_chunks("atomic.bin", vec![mc])]);

    let job = DepotJob::builder()
        .depot_id(DepotId(481))
        .depot_key(key)
        .install_dir(install.to_path_buf())
        .build()
        .unwrap();

    job.download(&manifest, Arc::new(MockFetcher { chunks }))
        .await
        .unwrap();

    // File should be at final path
    assert_eq!(
        std::fs::read(install.join("atomic.bin")).unwrap(),
        plaintext
    );
    // Staging file should be cleaned up (renamed away)
    let staging = install.join(".depotdownloader").join("staging");
    if staging.exists() {
        assert!(std::fs::read_dir(&staging).unwrap().next().is_none());
    }
}

#[tokio::test]
async fn non_atomic_overwrites_existing_larger_file() {
    let dir = tempfile::tempdir().unwrap();
    let install = dir.path();
    let key = DepotKey([0xAA; 32]);

    // Old file is larger than the new one
    let old_content = vec![0xFFu8; 1024];
    std::fs::write(install.join("shrink.bin"), &old_content).unwrap();

    let new_data = b"small new file";
    let chunk_id = ChunkId([0x11; 20]);
    let checksum = SteamAdler32::compute(new_data);

    let mut chunks = HashMap::new();
    chunks.insert(chunk_id.clone(), Bytes::from(encrypt_chunk(new_data, &key)));

    let mc = ManifestChunk::new(chunk_id, checksum.0, new_data.len() as u32);
    let manifest = DepotManifest::new(vec![file_with_chunks("shrink.bin", vec![mc])]);

    let job = DepotJob::builder()
        .depot_id(DepotId(481))
        .depot_key(key)
        .install_dir(install.to_path_buf())
        .non_atomic(true)
        .build()
        .unwrap();

    job.download(&manifest, Arc::new(MockFetcher { chunks }))
        .await
        .unwrap();

    // File should be the new smaller size, not 1024 bytes
    let result = std::fs::read(install.join("shrink.bin")).unwrap();
    assert_eq!(result, new_data);
    assert_eq!(result.len(), new_data.len());
}
