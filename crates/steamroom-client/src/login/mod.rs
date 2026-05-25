//! High-level Steam login orchestration.
//!
//! Provides a typestate-driven builder over `steamroom::client::SteamClient`
//! for the full login lifecycle: CM server discovery, transport selection,
//! encryption handshake, the OAuth-like auth flow (including 2FA), and the
//! final CM logon. See the design spec at
//! `docs/superpowers/specs/2026-05-24-login-builder-design.md`.

mod credentials;
mod error;
mod qr;
mod terminal;

pub use error::LoginError;

// Re-exports from steamroom::auth so callers don't need both imports.
pub use steamroom::auth::{AuthTokens, GuardType};

/// Steam's `client_os_type` value (its internal `EOSType` — hundreds of entries
/// covering OS + version combinations). Steam encodes both OS and version in a
/// single integer; the full list is too large to enumerate here.
///
/// Use [`ClientOs::new`] for any value not predefined here. Steam does not
/// reject unknown values; it just records what you send.
///
/// ```
/// # use steamroom_client::login::ClientOs;
/// let os = ClientOs::WINDOWS_11;
/// let custom = ClientOs::new(42);
/// assert_eq!(custom.value(), 42);
/// ```
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ClientOs(i32);

impl ClientOs {
    /// Wrap a raw `EOSType` wire value.
    pub const fn new(value: i32) -> Self {
        Self(value)
    }

    /// Get the raw `EOSType` wire value.
    pub const fn value(self) -> i32 {
        self.0
    }

    /// Convert to the `uint32` representation used by proto fields
    /// (`client_os_type` in `CMsgClientLogon` is `optional uint32`).
    /// Steam's wire encoding treats the bit pattern of the signed
    /// `EOSType` as unsigned; this cast preserves it.
    pub(crate) const fn proto_value(self) -> u32 {
        self.0 as u32
    }

    /// Windows 11 (wire value 20). Matches the steamroom CLI default.
    pub const WINDOWS_11: Self = Self(20);
}

impl Default for ClientOs {
    fn default() -> Self {
        Self::WINDOWS_11
    }
}

use steamroom::client::{Encrypted, SteamClient};
use steamroom::connection::Protocol;

/// Configuration shared by `LoginBuilder` and `PreparedLoginBuilder`.
/// Private — the public API exposes setters individually.
#[derive(Debug)]
pub(crate) struct BuilderConfig {
    pub(crate) device_name: Option<String>,
    pub(crate) cell_id: u32,
    pub(crate) login_id: Option<u32>,
    pub(crate) client_os: ClientOs,
}

impl Default for BuilderConfig {
    fn default() -> Self {
        Self {
            device_name: None,
            cell_id: 0,
            login_id: None,
            client_os: ClientOs::default(),
        }
    }
}

/// How the builder obtains the underlying encrypted client.
pub(crate) enum TransportConfig {
    /// Discover CM servers, connect via preferred protocol, run encryption.
    Auto {
        prefer: Protocol,
        allow_fallback: bool,
    },
    /// Use a pre-built encrypted client (capture/replay, custom transport).
    Provided(SteamClient<Encrypted>),
}

impl Default for TransportConfig {
    fn default() -> Self {
        Self::Auto {
            prefer: Protocol::Tcp,
            allow_fallback: true,
        }
    }
}

use steamroom::connection::CmServer;
use steamroom::transport::tcp::TcpTransport;
use steamroom::transport::websocket::WebSocketTransport;

/// Establish an encrypted Steam client according to `transport`.
/// Consumes the `TransportConfig`. For `Provided`, returns the client as-is.
pub(crate) async fn establish_encrypted_client(
    transport: TransportConfig,
) -> Result<SteamClient<Encrypted>, LoginError> {
    match transport {
        TransportConfig::Provided(client) => Ok(client),
        TransportConfig::Auto { prefer, allow_fallback } => {
            connect_auto(prefer, allow_fallback).await
        }
    }
}

async fn connect_auto(
    prefer: Protocol,
    allow_fallback: bool,
) -> Result<SteamClient<Encrypted>, LoginError> {
    let servers = CmServer::fetch()
        .await
        .unwrap_or_else(|_| CmServer::defaults());

    if let Some(server) = servers.iter().find(|s| s.protocol == prefer) {
        match try_connect(server).await {
            Ok(client) => return Ok(client),
            Err(e) if !allow_fallback => return Err(e),
            Err(_) => { /* fall through to other protocol */ }
        }
    }

    let other = match prefer {
        Protocol::Tcp => Protocol::WebSocket,
        Protocol::WebSocket => Protocol::Tcp,
    };
    if let Some(server) = servers.iter().find(|s| s.protocol == other) {
        return try_connect(server).await;
    }

    Err(LoginError::NoCmServers)
}

async fn try_connect(server: &CmServer) -> Result<SteamClient<Encrypted>, LoginError> {
    match server.protocol {
        Protocol::Tcp => {
            let transport = TcpTransport::connect(server).await?;
            let (client, _rx) = SteamClient::connect(transport).await?;
            Ok(client.encrypt().await?)
        }
        Protocol::WebSocket => {
            let transport = WebSocketTransport::connect(server).await?;
            let (client, _rx) = SteamClient::connect_ws(transport).await?;
            Ok(client)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_windows_11() {
        assert_eq!(ClientOs::default(), ClientOs::WINDOWS_11);
        assert_eq!(ClientOs::default().value(), 20);
    }

    #[test]
    fn new_round_trips() {
        assert_eq!(ClientOs::new(42).value(), 42);
        assert_eq!(ClientOs::new(-1).value(), -1);
    }
}
