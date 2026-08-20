//! High-level Steam login orchestration.
//!
//! Provides a typestate-driven builder over `steamroom::client::SteamClient`
//! for the full login lifecycle: CM server discovery, transport selection,
//! encryption handshake, the OAuth-like auth flow (including 2FA), and the
//! final CM logon. See the design spec at
//! `docs/superpowers/specs/2026-05-24-login-builder-design.md`.

mod credentials;
pub use credentials::ConfirmationChallenge;
pub use credentials::CredentialsLogin;
pub use credentials::CredentialsLoginFlow;
mod error;
mod qr;
pub use qr::QrLogin;
pub use qr::QrLoginFlow;
mod terminal;
pub use terminal::AnonymousLogin;
pub use terminal::ApprovedAuth;
pub use terminal::TokenLogin;

pub use error::LoginError;

// Re-exports from steamroom::auth so callers don't need both imports.
pub use steamroom::auth::AuthTokens;
pub use steamroom::auth::GuardType;

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

use steamroom::client::Encrypted;
use steamroom::client::Ready;
use steamroom::client::SteamClient;
use steamroom::connection::Protocol;

/// `EAuthTokenPlatformType::SteamClient`. Sent in `BeginAuthSession*` requests
/// so Steam issues tokens valid for client logon (`aud` includes `"client"`).
/// Defaulting to `Unknown` (0) yields a web-platform token whose audiences
/// (`web`/`renew`/`derive`) the CM rejects at `CMsgClientLogon` with
/// `InvalidPassword`.
pub(crate) const STEAM_CLIENT_PLATFORM_TYPE: i32 = 1;

/// Configuration shared by `LoginBuilder` and `PreparedLoginBuilder`.
/// Private — the public API exposes setters individually.
#[derive(Debug, Default)]
pub(crate) struct BuilderConfig {
    pub(crate) device_name: Option<String>,
    pub(crate) cell_id: u32,
    pub(crate) login_id: Option<u32>,
    pub(crate) client_os: ClientOs,
}

/// How the builder obtains the underlying ready client.
pub(crate) enum TransportConfig {
    /// Discover CM servers, connect via preferred protocol, run encryption,
    /// then send `CMsgClientHello`.
    Auto {
        prefer: Protocol,
        allow_fallback: bool,
        /// When set, the chosen transport is wrapped in a
        /// `RecordingTransport` recording into this handle.
        recorder: Option<Recorder>,
    },
    /// Use a pre-built `Ready` client (capture/replay, custom transport). The
    /// caller is responsible for having driven `connect → encrypt → prepare`.
    Provided(SteamClient<Ready>),
}

impl Default for TransportConfig {
    fn default() -> Self {
        Self::Auto {
            prefer: Protocol::Tcp,
            allow_fallback: true,
            recorder: None,
        }
    }
}

use steamroom::connection::CmServer;
use steamroom::transport::recording::Recorder;
use steamroom::transport::recording::RecordingTransport;
use steamroom::transport::tcp::TcpTransport;
use steamroom::transport::websocket::WebSocketTransport;

/// Establish a `Ready` Steam client according to `transport`. For `Provided`,
/// returns the caller-supplied client as-is (already prepared by the caller).
pub(crate) async fn establish_ready_client(
    transport: TransportConfig,
) -> Result<SteamClient<Ready>, LoginError> {
    match transport {
        TransportConfig::Provided(client) => Ok(client),
        TransportConfig::Auto {
            prefer,
            allow_fallback,
            recorder,
        } => Ok(connect_auto(prefer, allow_fallback, recorder.as_ref())
            .await?
            .prepare()
            .await?),
    }
}

async fn connect_auto(
    prefer: Protocol,
    allow_fallback: bool,
    recorder: Option<&Recorder>,
) -> Result<SteamClient<Encrypted>, LoginError> {
    let servers = CmServer::fetch()
        .await
        .unwrap_or_else(|_| CmServer::defaults());

    if let Some(server) = servers.iter().find(|s| s.protocol == prefer) {
        match try_connect(server, recorder).await {
            Ok(client) => return Ok(client),
            Err(e) if !allow_fallback => return Err(e),
            Err(_) => {}
        }
    }

    let other = match prefer {
        Protocol::Tcp => Protocol::WebSocket,
        Protocol::WebSocket => Protocol::Tcp,
    };
    if let Some(server) = servers.iter().find(|s| s.protocol == other) {
        return try_connect(server, recorder).await;
    }

    Err(LoginError::NoCmServers)
}

async fn try_connect(
    server: &CmServer,
    recorder: Option<&Recorder>,
) -> Result<SteamClient<Encrypted>, LoginError> {
    match server.protocol {
        Protocol::Tcp => {
            let transport = TcpTransport::connect(server).await?;
            let (client, _rx) = match recorder {
                Some(r) => {
                    SteamClient::connect(RecordingTransport::with_recorder(transport, r.clone()))
                        .await?
                }
                None => SteamClient::connect(transport).await?,
            };
            Ok(client.encrypt().await?)
        }
        Protocol::WebSocket => {
            let transport = WebSocketTransport::connect(server).await?;
            let (client, _rx) = match recorder {
                Some(r) => {
                    SteamClient::connect_ws(RecordingTransport::with_recorder(transport, r.clone()))
                        .await?
                }
                None => SteamClient::connect_ws(transport).await?,
            };
            Ok(client)
        }
    }
}

/// Top-level builder for the auto-discovery login path: builder discovers
/// CM servers, connects, runs the encryption handshake, then drives the
/// chosen auth method. For a pre-built `SteamClient<Ready>`, use
/// `PreparedLoginBuilder` instead.
pub struct LoginBuilder {
    config: BuilderConfig,
    transport_prefer: Protocol,
    transport_allow_fallback: bool,
    recorder: Option<Recorder>,
}

impl LoginBuilder {
    pub fn new() -> Self {
        Self {
            config: BuilderConfig::default(),
            transport_prefer: Protocol::Tcp,
            transport_allow_fallback: true,
            recorder: None,
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

    /// Record the session's received packets into `recorder`. The caller
    /// keeps a clone to flush the capture once login and subsequent work
    /// are done. Has no effect on the BYO-client (`PreparedLoginBuilder`)
    /// path.
    pub fn record(mut self, recorder: Recorder) -> Self {
        self.recorder = Some(recorder);
        self
    }

    fn transport(&self) -> TransportConfig {
        TransportConfig::Auto {
            prefer: self.transport_prefer,
            allow_fallback: self.transport_allow_fallback,
            recorder: self.recorder.clone(),
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

/// Top-level builder for the BYO-client path. Use this when you have already
/// driven a client through `connect → encrypt → prepare` yourself (e.g. via
/// the capture/replay transport in tests, or a custom transport). Transport
/// configuration methods are intentionally absent — the transport is already
/// chosen by the caller.
pub struct PreparedLoginBuilder {
    config: BuilderConfig,
    client: Option<SteamClient<Ready>>,
}

impl PreparedLoginBuilder {
    pub fn new(client: SteamClient<Ready>) -> Self {
        Self {
            config: BuilderConfig::default(),
            client: Some(client),
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

    fn transport(&mut self) -> TransportConfig {
        TransportConfig::Provided(
            self.client
                .take()
                .expect("client consumed twice; PreparedLoginBuilder methods consume self"),
        )
    }

    pub fn anonymous(mut self) -> AnonymousLogin {
        AnonymousLogin {
            transport: self.transport(),
            config: self.config,
        }
    }

    pub fn with_refresh_token(
        mut self,
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
        mut self,
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

    pub fn with_qr(mut self) -> QrLogin {
        QrLogin {
            transport: self.transport(),
            config: self.config,
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
