//! Steam client protocol library for depot downloading, manifest parsing, and CDN access.
//!
//! This crate provides the low-level building blocks for interacting with Steam:
//!
//! - [`client`] -- Connect, encrypt, authenticate, and send/receive Steam protocol messages
//! - [`depot`] -- Manifest parsing, chunk decryption/decompression, and checksum verification
//! - [`cdn`] -- Download content from Steam's CDN with server pooling and rate-limit handling
//! - [`auth`] -- Credential and QR code authentication flows
//! - [`types`] -- KeyValue format parsing (binary and text) with serde deserialization
//!
//! # Quick start
//!
//! ```rust,no_run
//! use steamroom::client::SteamClient;
//! use steamroom::connection::CmServer;
//! use steamroom::transport::websocket::WebSocketTransport;
//! use steamroom::depot::{AppId, DepotId};
//!
//! # async fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Discover CM servers and connect
//! let servers = CmServer::fetch().await?;
//! let ws_server = servers.iter()
//!     .find(|s| s.protocol == steamroom::connection::Protocol::WebSocket)
//!     .expect("no WebSocket server");
//! let transport = WebSocketTransport::connect(ws_server).await?;
//! let (client, _rx) = SteamClient::connect_ws(transport).await?;
//!
//! // For authenticated downloads, build a logon message and call client.login().
//! // For anonymous access (free apps like Spacewar), use an anonymous logon.
//!
//! // After login, request product info, depot keys, and manifests:
//! // let tokens = client.get_access_tokens(&[AppId(480)]).await?;
//! // let key = client.get_depot_decryption_key(DepotId(481), AppId(480)).await?;
//! # Ok(())
//! # }
//! ```
//!
//! For a higher-level download API with retry, delta patching, and progress events,
//! see the [`steamroom-client`](https://crates.io/crates/steamroom-client) crate.

/// PICS app info, access tokens, and product metadata.
pub mod apps;
/// Credential and QR code authentication flows.
pub mod auth;
/// CDN client, server pool, and lancache support.
pub mod cdn;
/// Steam CM client with typestate connection lifecycle.
pub mod client;
/// CM server discovery, encryption, and packet framing.
pub mod connection;
/// CDN auth token types.
pub mod content;
/// AES-256, RSA, and other cryptographic primitives.
pub mod crypto;
/// Depot manifests, chunk decryption/decompression, and ID types.
pub mod depot;
/// Steam protocol enums (EResult, file flags, etc.).
pub mod enums;
/// Error types for all operations.
pub mod error;
/// Raw Steam protocol message IDs and header parsing.
pub mod messages;
/// Transport implementations (TCP, WebSocket) and capture/replay for testing.
pub mod transport;
/// KeyValue format parsing, SteamID, GameID, and serde integration.
pub mod types;
/// Checksum and compression utilities.
pub mod util;

/// Auto-generated protobuf message types from Steam's `.proto` definitions.
pub mod generated;

pub use error::Error;
pub use error::Result;
