//! Verifies the streaming download path keeps peak heap usage bounded by
//! `max_downloads * chunk_size`, not by the file size. With the previous
//! slot-based design (`Vec<OnceLock<Vec<u8>>>` of every decompressed chunk
//! plus a full-file slurp for SHA-1 verification) peak heap scaled with the
//! file. A 13 GiB file (Subnautica2-Windows.ucas, depot 1962701, manifest
//! 4222263125962173451) OOMed the host.

#[global_allocator]
static ALLOC: dhat::Alloc = dhat::Alloc;

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
use steamroom_client::download::BoxError;
use steamroom_client::download::ChunkFetcher;
use steamroom_client::download::DepotJob;

struct MockFetcher {
    chunks: HashMap<ChunkId, Bytes>,
}

impl ChunkFetcher for MockFetcher {
    async fn fetch_chunk(&self, _depot_id: DepotId, chunk_id: &ChunkId) -> Result<Bytes, BoxError> {
        match self.chunks.get(chunk_id) {
            Some(b) => Ok(b.clone()),
            None => Err(format!("chunk {chunk_id} not in fixture").into()),
        }
    }
}

fn encrypt_chunk(plaintext: &[u8], key: &DepotKey) -> Vec<u8> {
    let iv = [0x42u8; 16];
    let encrypted_iv = steamroom::crypto::symmetric_encrypt_ecb_nopad(&iv, &key.0).unwrap();
    let encrypted_body = steamroom::crypto::symmetric_encrypt_cbc(plaintext, &key.0, &iv).unwrap();
    let mut chunk = Vec::with_capacity(encrypted_iv.len() + encrypted_body.len());
    chunk.extend_from_slice(&encrypted_iv);
    chunk.extend_from_slice(&encrypted_body);
    chunk
}

const NUM_CHUNKS: usize = 256;
const CHUNK_SIZE: usize = 256 * 1024;
const FILE_SIZE: u64 = (NUM_CHUNKS * CHUNK_SIZE) as u64;
const MAX_DOWNLOADS: usize = 4;
// Bug-era peak scaled with FILE_SIZE (~64 MiB plus the second full-file
// SHA-1 slurp). The streaming path holds at most
// `max_downloads * (encrypted + decompressed chunk)` plus a few hundred KiB
// of bookkeeping. 16 MiB caps that with 4x headroom for tokio task structs
// and allocator noise; far below the file size, so a regression is visible.
const HEAP_BUDGET_BYTES: u64 = 16 * 1024 * 1024;

fn build_depot(key: &DepotKey) -> (DepotManifest, MockFetcher) {
    let mut chunks_map = HashMap::with_capacity(NUM_CHUNKS);
    let mut chunks_meta = Vec::with_capacity(NUM_CHUNKS);
    let mut plaintext = vec![0u8; CHUNK_SIZE];
    for i in 0..NUM_CHUNKS {
        // Distinct content per chunk so chunk ids and Adler-32 checksums differ.
        // A fixed non-magic two-byte prefix keeps ChunkCompression::detect() on
        // the `None` arm regardless of `fill`. Otherwise byte values like 0x5D
        // (LZMA) or 0x50/0x56 (zip/VZ/VS) would trigger decompression on raw
        // plaintext and the test would fail before measuring anything.
        let fill = (i as u8).wrapping_add(1);
        plaintext.fill(fill);
        plaintext[0] = 0x01;
        plaintext[1] = 0x02;
        let mut id_bytes = [0u8; 20];
        id_bytes[..8].copy_from_slice(&(i as u64).to_le_bytes());
        let chunk_id = ChunkId(id_bytes);
        let checksum = SteamAdler32::compute(&plaintext);
        let encrypted = encrypt_chunk(&plaintext, key);
        chunks_map.insert(chunk_id.clone(), Bytes::from(encrypted));
        let mut chunk = ManifestChunk::new(chunk_id, checksum.0, CHUNK_SIZE as u32);
        chunk.offset = Some((i * CHUNK_SIZE) as u64);
        chunks_meta.push(chunk);
    }
    let mut file = ManifestFile::new("big.bin".to_string(), FILE_SIZE);
    file.chunks = chunks_meta;
    let manifest = DepotManifest::new(vec![file]);
    (manifest, MockFetcher { chunks: chunks_map })
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn streaming_download_stays_under_heap_budget() {
    let dir = tempfile::tempdir().unwrap();
    let install = dir.path();
    let key = DepotKey([0xAA; 32]);

    // Build the fixture BEFORE arming the profiler so its ~64 MiB of pre-built
    // encrypted bytes do not count toward the budget. dhat's allocator only
    // tracks allocations made while a profiler is alive; pre-existing live
    // memory is invisible.
    let (manifest, fetcher) = build_depot(&key);

    let job = DepotJob::builder()
        .depot_id(DepotId(481))
        .depot_key(key)
        .install_dir(install.to_path_buf())
        .max_downloads(MAX_DOWNLOADS)
        .non_atomic(true)
        .build()
        .unwrap();

    let profiler = dhat::Profiler::builder().testing().build();
    job.download(&manifest, Arc::new(fetcher)).await.unwrap();
    let stats = dhat::HeapStats::get();
    drop(profiler);

    let peak = stats.max_bytes as u64;
    eprintln!(
        "streaming download peak heap = {peak} bytes ({:.2} MiB), budget = {HEAP_BUDGET_BYTES}",
        peak as f64 / (1024.0 * 1024.0)
    );
    assert!(
        peak <= HEAP_BUDGET_BYTES,
        "streaming download peak heap = {peak} bytes, budget = {HEAP_BUDGET_BYTES} \
         (file {FILE_SIZE} bytes, max_downloads {MAX_DOWNLOADS}, chunks {NUM_CHUNKS}); \
         regression to a per-file (rather than per-window) memory model",
    );
}
