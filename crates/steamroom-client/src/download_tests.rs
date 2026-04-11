use crate::download::BoxError;
use crate::download::ChunkFetcher;
use crate::download::DepotJob;
use crate::download::FileFilter;
use crate::event::DownloadEvent;
use bytes::Bytes;
use std::collections::HashMap;
use std::sync::Arc;
use steamroom::depot::manifest::DepotManifest;
use steamroom::depot::manifest::ManifestChunk;
use steamroom::depot::manifest::ManifestFile;
use steamroom::depot::ChunkId;
use steamroom::depot::DepotId;
use steamroom::depot::DepotKey;
use steamroom::util::checksum::SteamAdler32;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

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
    ManifestFile {
        filename: Some(name.to_string()),
        size: Some(0),
        flags: Some(0),
        sha_content: None,
        chunks: vec![],
        link_target: None,
    }
}

fn manifest_with(files: &[&str]) -> DepotManifest {
    DepotManifest {
        depot_id: Some(DepotId(481)),
        manifest_id: None,
        creation_time: None,
        filenames_encrypted: false,
        total_uncompressed_size: None,
        total_compressed_size: None,
        files: files.iter().map(|n| empty_file(n)).collect(),
    }
}

// ---------------------------------------------------------------------------
// FileFilter tests
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Download pipeline: actual chunk fetch + decrypt
// ---------------------------------------------------------------------------

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

    let manifest = DepotManifest {
        depot_id: Some(DepotId(481)),
        manifest_id: None,
        creation_time: None,
        filenames_encrypted: false,
        total_uncompressed_size: None,
        total_compressed_size: None,
        files: vec![ManifestFile {
            filename: Some("test.txt".into()),
            size: Some(plaintext.len() as u64),
            flags: Some(0),
            sha_content: None,
            chunks: vec![ManifestChunk {
                id: Some(chunk_id),
                checksum: Some(checksum.0),
                offset: Some(0),
                compressed_size: None,
                uncompressed_size: Some(plaintext.len() as u32),
            }],
            link_target: None,
        }],
    };

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

    let manifest = DepotManifest {
        depot_id: Some(DepotId(481)),
        manifest_id: None,
        creation_time: None,
        filenames_encrypted: false,
        total_uncompressed_size: None,
        total_compressed_size: None,
        files: vec![ManifestFile {
            filename: Some("multi.bin".into()),
            size: Some(combined.len() as u64),
            flags: Some(0),
            sha_content: None,
            chunks: vec![
                ManifestChunk {
                    id: Some(id_a),
                    checksum: Some(SteamAdler32::compute(part_a).0),
                    offset: Some(0),
                    compressed_size: None,
                    uncompressed_size: Some(part_a.len() as u32),
                },
                ManifestChunk {
                    id: Some(id_b),
                    checksum: Some(SteamAdler32::compute(part_b).0),
                    offset: Some(part_a.len() as u64),
                    compressed_size: None,
                    uncompressed_size: Some(part_b.len() as u32),
                },
            ],
            link_target: None,
        }],
    };

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

    let manifest = DepotManifest {
        depot_id: Some(DepotId(481)),
        manifest_id: None,
        creation_time: None,
        filenames_encrypted: false,
        total_uncompressed_size: None,
        total_compressed_size: None,
        files: vec![ManifestFile {
            filename: Some("evented.bin".into()),
            size: Some(plaintext.len() as u64),
            flags: Some(0),
            sha_content: None,
            chunks: vec![ManifestChunk {
                id: Some(chunk_id),
                checksum: Some(checksum.0),
                offset: Some(0),
                compressed_size: None,
                uncompressed_size: Some(plaintext.len() as u32),
            }],
            link_target: None,
        }],
    };

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

// ---------------------------------------------------------------------------
// Delta removal tests (from original file)
// ---------------------------------------------------------------------------

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
