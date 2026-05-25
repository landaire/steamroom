//! High-level Steam login orchestration.
//!
//! Provides a typestate-driven builder over `steamroom::client::SteamClient`
//! for the full login lifecycle: CM server discovery, transport selection,
//! encryption handshake, the OAuth-like auth flow (including 2FA), and the
//! final CM logon. See the design spec at
//! `docs/superpowers/specs/2026-05-24-login-builder-design.md`.

mod credentials;
pub use credentials::{CredentialsLogin, CredentialsLoginFlow, GuardChallenge, MobileChallenge};
mod error;
mod qr;
pub use qr::{QrLogin, QrLoginFlow};
mod terminal;
pub use terminal::{AnonymousLogin, ApprovedAuth, TokenLogin};

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

/// Top-level builder for the auto-discovery login path: builder discovers
/// CM servers, connects, runs the encryption handshake, then drives the
/// chosen auth method. For a pre-built `SteamClient<Encrypted>`, use
/// `PreparedLoginBuilder` instead.
pub struct LoginBuilder {
    config: BuilderConfig,
    transport_prefer: Protocol,
    transport_allow_fallback: bool,
}

impl LoginBuilder {
    pub fn new() -> Self {
        Self {
            config: BuilderConfig::default(),
            transport_prefer: Protocol::Tcp,
            transport_allow_fallback: true,
        }
    }

    pub fn device_name(mut self, name: impl Into<String>) -> Self {
        self.config.device_name = Some(name.into());
        self
    }

    pub fn cell_id(mut self, id: u32) -> Self {
        self.config.cell_id = id;
        self
    }

    pub fn login_id(mut self, id: u32) -> Self {
        self.config.login_id = Some(id);
        self
    }

    pub fn client_os(mut self, os: ClientOs) -> Self {
        self.config.client_os = os;
        self
    }

    pub fn prefer_protocol(mut self, p: Protocol) -> Self {
        self.transport_prefer = p;
        self
    }

    pub fn allow_protocol_fallback(mut self, allow: bool) -> Self {
        self.transport_allow_fallback = allow;
        self
    }

    fn transport(&self) -> TransportConfig {
        TransportConfig::Auto {
            prefer: self.transport_prefer,
            allow_fallback: self.transport_allow_fallback,
        }
    }

    pub fn anonymous(self) -> AnonymousLogin {
        AnonymousLogin {
            transport: self.transport(),
            config: self.config,
        }
    }

    pub fn with_refresh_token(
        self,
        account: impl Into<String>,
        refresh_token: impl Into<String>,
    ) -> TokenLogin {
        TokenLogin {
            transport: self.transport(),
            config: self.config,
            account_name: account.into(),
            refresh_token: refresh_token.into(),
        }
    }

    pub fn with_credentials(
        self,
        account: impl Into<String>,
        password: impl Into<String>,
    ) -> CredentialsLogin {
        CredentialsLogin {
            transport: self.transport(),
            config: self.config,
            account_name: account.into(),
            password: password.into(),
        }
    }

    pub fn with_qr(self) -> QrLogin {
        QrLogin {
            transport: self.transport(),
            config: self.config,
        }
    }
}

impl Default for LoginBuilder {
    fn default() -> Self {
        Self::new()
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

    #[test]
    fn builder_accumulates_config() {
        let b = LoginBuilder::new()
            .device_name("test-device")
            .cell_id(42)
            .login_id(7)
            .client_os(ClientOs::new(99))
            .prefer_protocol(Protocol::WebSocket)
            .allow_protocol_fallback(false);
        assert_eq!(b.config.device_name.as_deref(), Some("test-device"));
        assert_eq!(b.config.cell_id, 42);
        assert_eq!(b.config.login_id, Some(7));
        assert_eq!(b.config.client_os.value(), 99);
        assert_eq!(b.transport_prefer, Protocol::WebSocket);
        assert!(!b.transport_allow_fallback);
    }

    #[test]
    fn all_terminal_methods_compile() {
        // Just exercising the API surface — no I/O.
        let _ = LoginBuilder::new().anonymous();
        let _ = LoginBuilder::new().with_refresh_token("u", "t");
        let _ = LoginBuilder::new().with_credentials("u", "p");
        let _ = LoginBuilder::new().with_qr();
    }
}
