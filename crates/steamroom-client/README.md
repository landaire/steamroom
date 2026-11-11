# steamroom-client

High-level download orchestration for Steam depots built on top of [steamroom](https://crates.io/crates/steamroom).

## What's in this crate

- **Pipelined downloads** -- Concurrent chunk fetching with backpressure, retry, and ordered reassembly
- **Delta patching** -- Compares old and new manifests to skip unchanged files and remove deleted ones
- **File filtering** -- Filter downloads by literal paths, regex patterns, or `regex:`-prefixed filelist files
- **CDN server pool** -- Automatic rotation across CDN servers with health tracking and rate-limit backoff
- **Manifest caching** -- Local cache for depot manifests to avoid redundant downloads
- **Event stream** -- `DownloadEvent` channel for progress bars, logging, or custom UI
- **Config watermarks** -- Decode and encode account-local and machine-local values from Steam's config store

## Usage

```rust,no_run
use steamroom::depot::{DepotId, DepotKey};
use steamroom::cdn::{CdnClient, CdnServerPool};
use steamroom_client::download::{CdnChunkFetcher, DepotJob, FileFilter};

// Build a download job
let job = DepotJob::builder()
    .depot_id(DepotId(481))
    .depot_key(depot_key)
    .install_dir("/tmp/output".into())
    .file_filter(FileFilter::Regex(regex::Regex::new(r"\.dll$").unwrap()))
    .verify(true)
    .build()
    .unwrap();

// Create a CDN fetcher with server pool
let fetcher = CdnChunkFetcher::new(
    CdnClient::new().unwrap(),
    CdnServerPool::new(cdn_servers),
    None,
);

// Download (returns stats)
// let stats = job.download(&manifest, std::sync::Arc::new(fetcher)).await?;
```

## Config watermarks

Steam stores some `config.vdf` values as hexadecimal watermarked frames. The watermark detects corruption and binds a value to an account or Steam installation. It is not encryption.

Use `encoded_scope_hex` to inspect a literal value before selecting its seed. Account-local values use the 32-bit Steam account ID. Machine-local values use `Software\Valve\Steam\MTBF` from the same config store.

```rust
use steamroom_client::watermark::MachineWatermarkSeed;
use steamroom_client::watermark::WatermarkError;
use steamroom_client::watermark::WatermarkScope;
use steamroom_client::watermark::WatermarkScopeKind;
use steamroom_client::watermark::decode_hex;
use steamroom_client::watermark::encoded_scope_hex;

fn main() -> Result<(), WatermarkError> {
    let value = "02000000a2c4d301c9bbd5730000011f2549";
    assert_eq!(encoded_scope_hex(value)?, WatermarkScopeKind::Machine);

    let decoded = decode_hex(
        value,
        WatermarkScope::Machine(MachineWatermarkSeed::from_mtbf(1082155420)),
    )?;
    assert_eq!(decoded.payload, b"US\0\0\0\0\0\0\0\0");
    Ok(())
}
```

## License

MIT OR Apache-2.0
