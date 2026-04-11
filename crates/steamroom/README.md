# steamroom

Rust library implementing the Steam client protocol for depot downloading, manifest parsing, and CDN interaction.

## What's in this crate

- **Connection & auth** -- Connect to Steam CM servers over TCP or WebSocket, encrypt sessions, authenticate via credentials or QR code
- **Depot manifests** -- Parse and decrypt Steam depot manifests, list files, extract chunk metadata
- **Chunk processing** -- Decrypt and decompress depot chunks (AES-256-CBC, Valve LZMA, Valve zstd, zip)
- **CDN** -- Download manifests and chunks from Steam's CDN with server pool rotation and rate-limit handling
- **Protobuf types** -- Generated Steam protocol message types

## Usage

```rust,no_run
use steamroom::client::SteamClient;
use steamroom::transport::websocket::WebSocketTransport;
use steamroom::connection::CmServer;
use steamroom::depot::manifest::DepotManifest;
use steamroom::depot::{AppId, DepotId, ManifestId};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Connect to Steam
    let servers = CmServer::fetch().await?;
    let ws = servers.iter().find(|s| s.protocol == steamroom::connection::Protocol::WebSocket).unwrap();
    let transport = WebSocketTransport::connect(ws).await?;
    let (client, _rx) = SteamClient::connect_ws(transport).await?;

    // Anonymous login (for free apps)
    // ... build logon message, call client.login() ...

    Ok(())
}
```

See the [steamroom-cli](https://crates.io/crates/steamroom-cli) crate for a complete working example, or [steamroom-client](https://crates.io/crates/steamroom-client) for download orchestration.

## License

MIT OR Apache-2.0
