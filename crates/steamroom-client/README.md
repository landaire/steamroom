# steamroom-client

High-level download orchestration for Steam depots built on top of [steamroom](https://crates.io/crates/steamroom).

## What's in this crate

- **Pipelined downloads** -- Concurrent chunk fetching with backpressure, retry, and ordered reassembly
- **Delta patching** -- Compares old and new manifests to skip unchanged files and remove deleted ones
- **File filtering** -- Filter downloads by literal paths, regex patterns, or `regex:`-prefixed filelist files
- **CDN server pool** -- Automatic rotation across CDN servers with health tracking and rate-limit backoff
- **Manifest caching** -- Local cache for depot manifests to avoid redundant downloads
- **Event stream** -- `DownloadEvent` channel for progress bars, logging, or custom UI

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
let fetcher = CdnChunkFetcher {
    cdn: CdnClient::new().unwrap(),
    pool: CdnServerPool::new(cdn_servers),
    cdn_auth_token: None,
};

// Download (returns stats)
// let stats = job.download(&manifest, std::sync::Arc::new(fetcher)).await?;
```

## License

MIT OR Apache-2.0
