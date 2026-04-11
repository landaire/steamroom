use std::sync::Arc;
use bytes::Bytes;
use steamroom::depot::{DepotId, DepotKey, ChunkId};
use steamroom::depot::manifest::{DepotManifest, ManifestFile};
use crate::download::{ChunkFetcher, DepotJob, BoxError};
use crate::event::DownloadEvent;

struct NullFetcher;

impl ChunkFetcher for NullFetcher {
    async fn fetch_chunk(
        &self,
        _depot_id: DepotId,
        _chunk_id: &ChunkId,
    ) -> Result<Bytes, BoxError> {
        panic!("NullFetcher should not be called in these tests");
    }
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

#[tokio::test]
async fn delta_removes_files_not_in_new_manifest() {
    let dir = tempfile::tempdir().unwrap();
    let install = dir.path();

    // Simulate files from a previous download
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

    let stats = job.download(&new_manifest, Arc::new(NullFetcher)).await.unwrap();

    assert_eq!(stats.files_removed, 2);
    assert!(!install.join("remove_me.txt").exists());
    assert!(!install.join("also_gone.dat").exists());
    assert!(install.join("keep.txt").exists());
    assert!(install.join("new_file.txt").exists());

    // Check we got FileRemoved events
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

    let stats = job.download(&new_manifest, Arc::new(NullFetcher)).await.unwrap();

    assert_eq!(stats.files_removed, 0);
    assert!(install.join("stale.txt").exists());
}

#[tokio::test]
async fn delta_skips_already_missing_files() {
    let dir = tempfile::tempdir().unwrap();
    let install = dir.path();

    // Old manifest says "gone.txt" existed, but it's already missing on disk
    let old_files = vec!["gone.txt".to_string()];
    let new_manifest = manifest_with(&["new.txt"]);

    let job = DepotJob::builder()
        .depot_id(DepotId(481))
        .depot_key(DepotKey([0; 32]))
        .install_dir(install.to_path_buf())
        .old_manifest_files(old_files)
        .build()
        .unwrap();

    let stats = job.download(&new_manifest, Arc::new(NullFetcher)).await.unwrap();

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

    let stats = job.download(&new_manifest, Arc::new(NullFetcher)).await.unwrap();

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

    let stats = job.download(&new_manifest, Arc::new(NullFetcher)).await.unwrap();

    // remove_dir fails on non-empty dirs, so it should not be counted
    assert_eq!(stats.files_removed, 0);
    assert!(sub.exists());
}

#[tokio::test]
async fn delta_handles_nested_paths() {
    let dir = tempfile::tempdir().unwrap();
    let install = dir.path();

    // Create nested file that should be removed
    let nested = install.join("game").join("bin");
    std::fs::create_dir_all(&nested).unwrap();
    std::fs::write(nested.join("old.dll"), b"old").unwrap();

    // Create nested file that should survive
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

    let stats = job.download(&new_manifest, Arc::new(NullFetcher)).await.unwrap();

    assert_eq!(stats.files_removed, 1);
    assert!(!nested.join("old.dll").exists());
    assert!(nested.join("keep.dll").exists());
}
