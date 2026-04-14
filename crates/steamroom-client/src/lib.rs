//! High-level Steam depot download orchestration and delta patching.
//!
//! Built on [`steamroom`] for protocol-level operations, this crate handles the
//! full download lifecycle:
//!
//! - **[`download`]** -- Pipelined chunk fetching with concurrent I/O, backpressure,
//!   retry with exponential backoff, and ordered file assembly
//! - **[`download::FileFilter`]** -- Filter files by literal paths, regex, or
//!   `regex:`-prefixed filelist entries (compatible with DepotDownloader format)
//! - **[`download::CdnChunkFetcher`]** -- CDN fetcher with automatic server rotation
//!   and rate-limit awareness via [`steamroom::cdn::CdnServerPool`]
//! - **[`depot_config`]** -- Track installed manifests and depot keys for delta updates
//! - **[`event`]** -- [`DownloadEvent`](event::DownloadEvent) stream for progress reporting
//! - **[`manifest`]** -- Manifest cache for avoiding redundant CDN downloads
//!
//! # Example
//!
//! ```rust,no_run
//! use steamroom::depot::{DepotId, DepotKey};
//! use steamroom::cdn::{CdnClient, CdnServerPool};
//! use steamroom::cdn::server::CdnServer;
//! use steamroom_client::download::{CdnChunkFetcher, DepotJob};
//! use steamroom_client::event::DownloadEvent;
//!
//! # async fn example(
//! #     depot_key: DepotKey,
//! #     cdn_servers: Vec<CdnServer>,
//! #     manifest: steamroom::depot::manifest::DepotManifest,
//! # ) -> Result<(), Box<dyn std::error::Error>> {
//! let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel();
//!
//! let job = DepotJob::builder()
//!     .depot_id(DepotId(481))
//!     .depot_key(depot_key)
//!     .install_dir("/tmp/spacewar".into())
//!     .verify(true)
//!     .event_sender(event_tx)
//!     .build()
//!     .expect("missing required fields");
//!
//! let fetcher = CdnChunkFetcher::new(
//!     CdnClient::new().expect("http client"),
//!     CdnServerPool::new(cdn_servers),
//!     None,
//! );
//!
//! let stats = job.download(&manifest, std::sync::Arc::new(fetcher)).await
//!     .expect("download failed");
//! println!("downloaded {} files ({} bytes)", stats.files_completed, stats.bytes_downloaded);
//! # Ok(())
//! # }
//! ```

/// Saved login token storage.
pub mod credentials;
/// Installed depot/manifest tracking for delta updates.
pub mod depot_config;
/// Pipelined download orchestration, file filtering, and retry logic.
pub mod download;
/// Download progress events for UI integration.
pub mod event;
pub mod manifest;
/// Extract cached credentials from a local Steam installation.
pub mod steam_creds;

#[cfg(test)]
mod depot_config_tests;
#[cfg(test)]
mod download_tests;
