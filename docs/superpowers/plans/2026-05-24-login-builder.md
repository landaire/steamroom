# Login Builder Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Move login orchestration out of `steamroom-cli/src/main.rs` into a typestate-driven builder in `steamroom-client::login`, with 2FA support exposed as a state machine.

**Architecture:** New `steamroom-client/src/login/` module hosting `LoginBuilder` (auto-discovery path) and `PreparedLoginBuilder` (BYO encrypted client). Each provides four terminal auth methods: `anonymous()`, `with_refresh_token()`, `with_credentials()`, `with_qr()`. Credentials and QR return state-machine handles (`CredentialsLoginFlow` / `QrLoginFlow`); anonymous and token are direct `.login() -> Result<SteamClient<LoggedIn>, LoginError>`. Builder is pure (no filesystem). `ApprovedAuth::tokens()` exposes refresh tokens for caller-driven persistence.

**Tech Stack:** Rust 2024 edition, tokio async, prost protobuf, thiserror, base64 (RSA password encoding). All workspace deps already exist except `base64` for steamroom-client.

**Reference spec:** `docs/superpowers/specs/2026-05-24-login-builder-design.md`

**Commit conventions:** Use `jj` (not git). Use conventional-commit subjects (`feat(login):`, `refactor(cli):`, `docs:`, etc.). **Never** add `Co-Authored-By:` trailers.

---

## File Structure

```
crates/steamroom-client/
├── Cargo.toml                            # add base64 dep
├── src/
│   ├── lib.rs                            # add `pub mod login;`
│   └── login/
│       ├── mod.rs                        # LoginBuilder + PreparedLoginBuilder + ClientOs + shared config
│       ├── error.rs                      # LoginError
│       ├── terminal.rs                   # AnonymousLogin + TokenLogin + ApprovedAuth + logon helper
│       ├── credentials.rs                # CredentialsLogin + flow enum + GuardChallenge + MobileChallenge
│       └── qr.rs                         # QrLogin + QrLoginFlow

crates/steamroom-cli/
└── src/
    ├── main.rs                           # delete freeform login fns; add small CLI-local drivers
    └── errors.rs                         # add LoginError -> CliError conversion
```

Per project convention: tests are inline `#[cfg(test)] mod tests` at the bottom of each file (matches the `steamroom` crate). Async tests use `#[tokio::test]`.

---

## Task 1: Scaffold the login module

**Files:**
- Create: `crates/steamroom-client/src/login/mod.rs`
- Create: `crates/steamroom-client/src/login/error.rs` (stub)
- Create: `crates/steamroom-client/src/login/terminal.rs` (stub)
- Create: `crates/steamroom-client/src/login/credentials.rs` (stub)
- Create: `crates/steamroom-client/src/login/qr.rs` (stub)
- Modify: `crates/steamroom-client/src/lib.rs`
- Modify: `crates/steamroom-client/Cargo.toml`

- [ ] **Step 1: Add the base64 dependency**

Edit `crates/steamroom-client/Cargo.toml`. Under `[dependencies]`, after the `dirs-next = { workspace = true }` line, add:

```toml
base64 = { workspace = true }
```

- [ ] **Step 2: Create stub module files**

Create `crates/steamroom-client/src/login/error.rs`:

```rust
// LoginError defined in task 2.
```

Create `crates/steamroom-client/src/login/terminal.rs`:

```rust
// AnonymousLogin / TokenLogin / ApprovedAuth defined in later tasks.
```

Create `crates/steamroom-client/src/login/credentials.rs`:

```rust
// CredentialsLogin and friends defined in later tasks.
```

Create `crates/steamroom-client/src/login/qr.rs`:

```rust
// QrLogin and friends defined in later tasks.
```

Create `crates/steamroom-client/src/login/mod.rs`:

```rust
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
```

- [ ] **Step 3: Wire the module into `lib.rs`**

Edit `crates/steamroom-client/src/lib.rs`. After the existing `pub mod manifest;` (or wherever the module list ends), add:

```rust
/// High-level login orchestration with typestate-driven 2FA handling.
pub mod login;
```

- [ ] **Step 4: Verify it compiles**

Run: `cargo build -p steamroom-client`
Expected: clean build, no warnings about the new module.

- [ ] **Step 5: Commit**

```bash
jj describe -m "feat(login): scaffold login module in steamroom-client"
jj new
```

---

## Task 2: Define `LoginError`

**Files:**
- Modify: `crates/steamroom-client/src/login/error.rs`

- [ ] **Step 1: Write failing tests**

Replace the entire contents of `crates/steamroom-client/src/login/error.rs` with:

```rust
use steamroom::enums::EResultError;
use thiserror::Error;

/// Errors produced by the login builder.
///
/// `InvalidPassword` and `InvalidGuardCode` are promoted to top-level variants
/// because they are the expected user-recoverable cases. The CLI's retry loops
/// match them directly without inspecting nested error chains.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum LoginError {
    #[error("transport / connection error: {0}")]
    Transport(#[from] steamroom::Error),

    #[error("logon rejected by Steam: {0:?}")]
    LogonFailed(EResultError),

    #[error("invalid password")]
    InvalidPassword,

    #[error("two-factor code rejected")]
    InvalidGuardCode,

    #[error("Steam response missing field: {0}")]
    MissingField(&'static str),

    #[error("no CM servers available")]
    NoCmServers,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn invalid_password_display() {
        let err = LoginError::InvalidPassword;
        assert_eq!(err.to_string(), "invalid password");
    }

    #[test]
    fn missing_field_display() {
        let err = LoginError::MissingField("eresult");
        assert_eq!(err.to_string(), "Steam response missing field: eresult");
    }

    #[test]
    fn transport_from_steamroom_error() {
        let inner = steamroom::Error::Connection(
            steamroom::error::ConnectionError::DnsResolutionFailed,
        );
        let err: LoginError = inner.into();
        assert!(matches!(err, LoginError::Transport(_)));
    }
}
```

- [ ] **Step 2: Run tests and verify they pass**

Run: `cargo test -p steamroom-client login::error`
Expected: 3 passed, 0 failed.

- [ ] **Step 3: Commit**

```bash
jj describe -m "feat(login): add LoginError type"
jj new
```

---

## Task 3: Define `ClientOs` newtype

**Files:**
- Modify: `crates/steamroom-client/src/login/mod.rs`

- [ ] **Step 1: Add ClientOs to mod.rs**

In `crates/steamroom-client/src/login/mod.rs`, append after the existing re-exports:

```rust
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

    /// Windows 11 (wire value 20). Matches the steamroom CLI default.
    pub const WINDOWS_11: Self = Self(20);
}

impl Default for ClientOs {
    fn default() -> Self {
        Self::WINDOWS_11
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
```

- [ ] **Step 2: Run tests**

Run: `cargo test -p steamroom-client login::tests`
Expected: 2 passed.

- [ ] **Step 3: Commit**

```bash
jj describe -m "feat(login): add ClientOs newtype"
jj new
```

---

## Task 4: Internal config types

Introduce private `BuilderConfig` (the dials common to both builder types) and `TransportConfig` (auto-discovery vs. provided client). These are private implementation details — neither type appears in the public API.

**Files:**
- Modify: `crates/steamroom-client/src/login/mod.rs`

- [ ] **Step 1: Add config types**

In `crates/steamroom-client/src/login/mod.rs`, after the `ClientOs` block (before the `#[cfg(test)]` module), add:

```rust
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
```

- [ ] **Step 2: Verify compile**

Run: `cargo build -p steamroom-client`
Expected: clean build.

- [ ] **Step 3: Commit**

```bash
jj describe -m "feat(login): add internal BuilderConfig and TransportConfig"
jj new
```

---

## Task 5: Internal helper — establish encrypted client

Extract the existing CLI's CM-discovery + transport + encryption-handshake logic into a private helper that takes a `TransportConfig` and yields a `SteamClient<Encrypted>`. This is purely a refactor of code currently in `connect_and_login` (`steamroom-cli/src/main.rs` lines 109–135).

**Files:**
- Modify: `crates/steamroom-client/src/login/mod.rs`

- [ ] **Step 1: Add the helper**

Append to `crates/steamroom-client/src/login/mod.rs` (before the `#[cfg(test)]` block):

```rust
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
```

- [ ] **Step 2: Verify compile**

Run: `cargo build -p steamroom-client`
Expected: clean. Unused warnings on `establish_encrypted_client` are expected for now — it's used in later tasks.

- [ ] **Step 3: Commit**

```bash
jj describe -m "feat(login): add internal helper for establishing encrypted client"
jj new
```

---

## Task 6: `AnonymousLogin` and `TokenLogin` terminal flows

These two flows skip the entire auth dance: they go straight from encrypted client → `CMsgClientLogon` → `LoggedIn` client. Implements the logic currently in `build_anon_logon` / `build_token_logon` (`steamroom-cli/src/main.rs` lines 399–421).

**Files:**
- Modify: `crates/steamroom-client/src/login/terminal.rs`

- [ ] **Step 1: Implement the two terminal flows**

Replace the contents of `crates/steamroom-client/src/login/terminal.rs` with:

```rust
use crate::login::error::LoginError;
use crate::login::{BuilderConfig, TransportConfig, establish_encrypted_client};

use prost::Message;
use steamroom::client::msg::ClientMsg;
use steamroom::client::{Encrypted, LoggedIn, PROTOCOL_VERSION, SteamClient};
use steamroom::generated::CMsgClientLogon;
use steamroom::messages::EMsg;
use steamroom::types::SteamId;

/// Anonymous login. No credentials, no tokens. Produced by
/// `LoginBuilder::anonymous()` or `PreparedLoginBuilder::anonymous()`.
pub struct AnonymousLogin {
    pub(crate) config: BuilderConfig,
    pub(crate) transport: TransportConfig,
}

impl AnonymousLogin {
    /// Connect (if needed), send `CMsgClientLogon` with no credentials, wait
    /// for `CLIENT_LOG_ON_RESPONSE`.
    pub async fn login(self) -> Result<SteamClient<LoggedIn>, LoginError> {
        let client = establish_encrypted_client(self.transport).await?;
        let logon = CMsgClientLogon {
            protocol_version: Some(PROTOCOL_VERSION),
            cell_id: Some(self.config.cell_id),
            client_os_type: Some(self.config.client_os.value()),
            ..Default::default()
        };
        let steam_id = SteamId::from_parts(1, 10, 0, 0).raw();
        finish_logon(client, logon, steam_id).await
    }
}

/// Login with a previously-obtained refresh token. Produced by
/// `LoginBuilder::with_refresh_token()` or
/// `PreparedLoginBuilder::with_refresh_token()`.
pub struct TokenLogin {
    pub(crate) config: BuilderConfig,
    pub(crate) transport: TransportConfig,
    pub(crate) account_name: String,
    pub(crate) refresh_token: String,
}

impl TokenLogin {
    /// Connect (if needed), send `CMsgClientLogon` with the refresh token
    /// in the `access_token` field (Steam's wire name for this slot — yes,
    /// it's confusing), wait for `CLIENT_LOG_ON_RESPONSE`.
    pub async fn login(self) -> Result<SteamClient<LoggedIn>, LoginError> {
        let client = establish_encrypted_client(self.transport).await?;
        let logon = CMsgClientLogon {
            protocol_version: Some(PROTOCOL_VERSION),
            cell_id: Some(self.config.cell_id),
            client_os_type: Some(self.config.client_os.value()),
            account_name: Some(self.account_name),
            access_token: Some(self.refresh_token),
            ..Default::default()
        };
        let steam_id = SteamId::from_parts(1, 1, 1, 0).raw();
        finish_logon(client, logon, steam_id).await
    }
}

/// Shared logon: send `CMsgClientLogon`, await `CLIENT_LOG_ON_RESPONSE`,
/// translate any `EResult` failure into `LoginError::LogonFailed`.
pub(crate) async fn finish_logon(
    client: SteamClient<Encrypted>,
    logon: CMsgClientLogon,
    steam_id: u64,
) -> Result<SteamClient<LoggedIn>, LoginError> {
    let body = logon.encode_to_vec();
    let mut msg = ClientMsg::with_body(EMsg::CLIENT_LOGON, &body);
    msg.header.steamid = Some(steam_id);
    msg.header.client_sessionid = Some(0);

    match client.login(msg).await {
        Ok((client, _resp)) => Ok(client),
        Err(steamroom::Error::Connection(
            steamroom::error::ConnectionError::LogonFailed(eresult),
        )) => Err(LoginError::LogonFailed(eresult)),
        Err(e) => Err(LoginError::Transport(e)),
    }
}
```

- [ ] **Step 2: Verify compile**

Run: `cargo build -p steamroom-client`
Expected: clean. (No tests yet — these methods do real I/O against Steam servers; tested via CLI smoke tests at the end of the plan.)

- [ ] **Step 3: Commit**

```bash
jj describe -m "feat(login): add AnonymousLogin and TokenLogin terminal flows"
jj new
```

---

## Task 7: `LoginBuilder` with anonymous and refresh-token methods

Add the public `LoginBuilder` (auto-discovery path) with the simple terminal methods first. The credentials / QR methods come in later tasks.

**Files:**
- Modify: `crates/steamroom-client/src/login/mod.rs`
- Modify: `crates/steamroom-client/src/login/terminal.rs`

- [ ] **Step 1: Make terminal types accessible**

Edit `crates/steamroom-client/src/login/mod.rs`. Update the imports and add public re-exports. Replace the line `mod terminal;` with:

```rust
mod terminal;
pub use terminal::{AnonymousLogin, TokenLogin};
```

- [ ] **Step 2: Add the `LoginBuilder` struct**

In `crates/steamroom-client/src/login/mod.rs`, after the `TransportConfig` block (still before the `#[cfg(test)]` block), add:

```rust
/// Top-level builder for the auto-discovery login path: builder discovers
/// CM servers, connects, runs the encryption handshake, then drives the
/// chosen auth method. For a pre-built `SteamClient<Encrypted>`, use
/// [`PreparedLoginBuilder`] instead.
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
}

impl Default for LoginBuilder {
    fn default() -> Self {
        Self::new()
    }
}
```

- [ ] **Step 3: Add test for the builder accumulating config**

In the `#[cfg(test)] mod tests` block in `mod.rs`, append:

```rust
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
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p steamroom-client login::tests`
Expected: 3 passed (default_is_windows_11, new_round_trips, builder_accumulates_config).

- [ ] **Step 5: Commit**

```bash
jj describe -m "feat(login): add LoginBuilder with anonymous and refresh-token methods"
jj new
```

---

## Task 8: `ApprovedAuth` (shared post-2FA state)

`ApprovedAuth` represents "auth flow done, tokens in hand, ready to send `CMsgClientLogon`." Used by both credentials and QR flows.

**Files:**
- Modify: `crates/steamroom-client/src/login/terminal.rs`
- Modify: `crates/steamroom-client/src/login/mod.rs`

- [ ] **Step 1: Add `ApprovedAuth` to terminal.rs**

Append to `crates/steamroom-client/src/login/terminal.rs`:

```rust
use steamroom::auth::AuthTokens;

/// Auth flow has completed; tokens are available. Inspect via [`tokens()`]
/// (e.g. to persist the refresh token) and then call [`finish()`] to send the
/// `CMsgClientLogon` and reach the `LoggedIn` state.
///
/// [`tokens()`]: ApprovedAuth::tokens
/// [`finish()`]: ApprovedAuth::finish
pub struct ApprovedAuth {
    pub(crate) client: SteamClient<Encrypted>,
    pub(crate) config: BuilderConfig,
    pub(crate) tokens: AuthTokens,
}

impl ApprovedAuth {
    pub fn tokens(&self) -> &AuthTokens {
        &self.tokens
    }

    pub async fn finish(self) -> Result<SteamClient<LoggedIn>, LoginError> {
        let account_name = self
            .tokens
            .account_name
            .clone()
            .ok_or(LoginError::MissingField("account_name"))?;
        let logon = CMsgClientLogon {
            protocol_version: Some(PROTOCOL_VERSION),
            cell_id: Some(self.config.cell_id),
            client_os_type: Some(self.config.client_os.value()),
            account_name: Some(account_name),
            access_token: Some(self.tokens.access_token),
            ..Default::default()
        };
        let steam_id = SteamId::from_parts(1, 1, 1, 0).raw();
        finish_logon(self.client, logon, steam_id).await
    }
}
```

- [ ] **Step 2: Re-export from mod.rs**

In `crates/steamroom-client/src/login/mod.rs`, change `pub use terminal::{AnonymousLogin, TokenLogin};` to:

```rust
pub use terminal::{AnonymousLogin, ApprovedAuth, TokenLogin};
```

- [ ] **Step 3: Verify compile**

Run: `cargo build -p steamroom-client`
Expected: clean.

- [ ] **Step 4: Commit**

```bash
jj describe -m "feat(login): add ApprovedAuth shared post-2FA state"
jj new
```

---

## Task 9: `CredentialsLogin::begin()` — RSA exchange and BeginAuthSession

Implements the RSA exchange, password encryption, and `BeginAuthSessionViaCredentials` call. Returns one of three states. Mirrors lines 290–325 of the current `authenticate_credentials` in `steamroom-cli/src/main.rs`.

**Files:**
- Modify: `crates/steamroom-client/src/login/credentials.rs`
- Modify: `crates/steamroom-client/src/login/mod.rs`

- [ ] **Step 1: Implement credentials.rs**

Replace the contents of `crates/steamroom-client/src/login/credentials.rs` with:

```rust
use crate::login::error::LoginError;
use crate::login::terminal::ApprovedAuth;
use crate::login::{BuilderConfig, TransportConfig, establish_encrypted_client};

use base64::Engine;
use steamroom::auth::GuardType;
use steamroom::client::{Encrypted, SteamClient};
use steamroom::generated::CAuthenticationBeginAuthSessionViaCredentialsRequest;

/// Configured credentials login. Call [`begin()`] to start the auth flow.
///
/// [`begin()`]: CredentialsLogin::begin
pub struct CredentialsLogin {
    pub(crate) config: BuilderConfig,
    pub(crate) transport: TransportConfig,
    pub(crate) account_name: String,
    pub(crate) password: String,
}

/// State of a credentials login after `begin()`.
#[non_exhaustive]
pub enum CredentialsLoginFlow {
    /// No 2FA required. Call `ApprovedAuth::finish()` to complete the logon.
    Approved(ApprovedAuth),
    /// Steam Guard code required. Call `GuardChallenge::submit_code()`.
    NeedsGuardCode(GuardChallenge),
    /// Mobile-app confirmation pending. Call
    /// `MobileChallenge::wait_for_confirmation()`.
    NeedsMobileConfirm(MobileChallenge),
}

pub struct GuardChallenge {
    pub(crate) client: SteamClient<Encrypted>,
    pub(crate) config: BuilderConfig,
    pub(crate) client_id: u64,
    pub(crate) steam_id: u64,
    pub(crate) request_id: Vec<u8>,
    pub(crate) poll_interval: f32,
    pub(crate) allowed_kinds: Vec<GuardType>,
}

pub struct MobileChallenge {
    pub(crate) client: SteamClient<Encrypted>,
    pub(crate) config: BuilderConfig,
    pub(crate) client_id: u64,
    pub(crate) request_id: Vec<u8>,
    pub(crate) poll_interval: f32,
}

impl CredentialsLogin {
    /// Connect (or accept BYO client), fetch the RSA pubkey, RSA-encrypt the
    /// password, and call `BeginAuthSessionViaCredentials`. The returned flow
    /// indicates whether 2FA is needed and which kind.
    pub async fn begin(self) -> Result<CredentialsLoginFlow, LoginError> {
        let client = establish_encrypted_client(self.transport).await?;

        // RSA exchange
        let rsa = client
            .get_password_rsa_public_key(&self.account_name)
            .await?;
        let modulus = rsa.publickey_mod.ok_or(LoginError::MissingField("publickey_mod"))?;
        let exponent = rsa.publickey_exp.ok_or(LoginError::MissingField("publickey_exp"))?;
        let timestamp = rsa.timestamp.unwrap_or(0);

        let encrypted_password = steamroom::crypto::rsa::encrypt_with_rsa_public_key(
            self.password.as_bytes(),
            &modulus,
            &exponent,
        )
        .map_err(|e| LoginError::Transport(e.into()))?;
        let encoded_password =
            base64::engine::general_purpose::STANDARD.encode(&encrypted_password);

        // Begin auth session
        let req = CAuthenticationBeginAuthSessionViaCredentialsRequest {
            account_name: Some(self.account_name.clone()),
            encrypted_password: Some(encoded_password),
            encryption_timestamp: Some(timestamp),
            remember_login: Some(true),
            persistence: Some(1),
            device_friendly_name: self.config.device_name.clone(),
            ..Default::default()
        };
        let session = match client.begin_auth_session_via_credentials(req).await {
            Ok(s) => s,
            Err(steamroom::Error::Connection(
                steamroom::error::ConnectionError::ServiceMethodFailed(
                    steamroom::enums::EResultError::InvalidPassword,
                ),
            )) => return Err(LoginError::InvalidPassword),
            Err(e) => return Err(LoginError::Transport(e)),
        };

        let client_id = session.client_id.ok_or(LoginError::MissingField("client_id"))?;
        let request_id = session.request_id.ok_or(LoginError::MissingField("request_id"))?;
        let steam_id = session.steam_id.ok_or(LoginError::MissingField("steamid"))?;
        let poll_interval = session.poll_interval.unwrap_or(5.0);

        // Classify the next step.
        if session.allowed_confirmations.is_empty()
            || session
                .allowed_confirmations
                .iter()
                .any(|g| *g == GuardType::None)
        {
            // No 2FA needed — poll once for tokens (Steam may have them ready)
            // and produce ApprovedAuth directly.
            let tokens = poll_until_tokens(
                &client,
                client_id,
                &request_id,
                poll_interval,
            )
            .await?;
            return Ok(CredentialsLoginFlow::Approved(ApprovedAuth {
                client,
                config: self.config,
                tokens,
            }));
        }

        let needs_code = session
            .allowed_confirmations
            .iter()
            .any(|g| matches!(g, GuardType::DeviceCode | GuardType::EmailCode));

        if needs_code {
            return Ok(CredentialsLoginFlow::NeedsGuardCode(GuardChallenge {
                client,
                config: self.config,
                client_id,
                steam_id,
                request_id,
                poll_interval,
                allowed_kinds: session.allowed_confirmations,
            }));
        }

        // Mobile confirmation (DeviceConfirmation only)
        Ok(CredentialsLoginFlow::NeedsMobileConfirm(MobileChallenge {
            client,
            config: self.config,
            client_id,
            request_id,
            poll_interval,
        }))
    }
}

/// Poll `PollAuthSessionStatus` until tokens are returned. Used by the
/// guard-code and mobile-confirmation completion paths, and by the no-2FA
/// path in `begin()`.
pub(crate) async fn poll_until_tokens(
    client: &SteamClient<Encrypted>,
    client_id: u64,
    request_id: &[u8],
    interval_secs: f32,
) -> Result<steamroom::auth::AuthTokens, LoginError> {
    loop {
        tokio::time::sleep(std::time::Duration::from_secs_f32(interval_secs)).await;
        if let Some(tokens) = client.poll_auth_session(client_id, request_id).await? {
            return Ok(tokens);
        }
    }
}
```

- [ ] **Step 2: Re-export from mod.rs**

In `crates/steamroom-client/src/login/mod.rs`, change `mod credentials;` to:

```rust
mod credentials;
pub use credentials::{CredentialsLogin, CredentialsLoginFlow, GuardChallenge, MobileChallenge};
```

- [ ] **Step 3: Verify compile**

Run: `cargo build -p steamroom-client`
Expected: clean. Unused warnings on `GuardChallenge` / `MobileChallenge` methods are expected — they're added in the next task.

- [ ] **Step 4: Commit**

```bash
jj describe -m "feat(login): add CredentialsLogin and begin() with state-machine result"
jj new
```

---

## Task 10: `GuardChallenge::submit_code` and `MobileChallenge::wait_for_confirmation`

The two state-advance methods on the credentials flow.

**Files:**
- Modify: `crates/steamroom-client/src/login/credentials.rs`

- [ ] **Step 1: Add submit_code and wait_for_confirmation impls**

Append to `crates/steamroom-client/src/login/credentials.rs`:

```rust
impl GuardChallenge {
    /// Which kinds of Steam Guard code Steam is willing to accept.
    pub fn allowed_kinds(&self) -> &[GuardType] {
        &self.allowed_kinds
    }

    /// Submit a Steam Guard code, then poll for tokens.
    ///
    /// On `LoginError::InvalidGuardCode`, the challenge is returned unchanged
    /// so the caller can prompt for a new code without restarting the RSA
    /// exchange. On any other error, the session is dead.
    pub async fn submit_code(
        self,
        code: &str,
        kind: GuardType,
    ) -> Result<ApprovedAuth, (GuardChallenge, LoginError)> {
        match self
            .client
            .submit_steam_guard_code(self.client_id, self.steam_id, code, kind)
            .await
        {
            Ok(()) => {}
            Err(steamroom::Error::Connection(
                steamroom::error::ConnectionError::ServiceMethodFailed(
                    steamroom::enums::EResultError::TwoFactorCodeMismatch,
                ),
            )) => return Err((self, LoginError::InvalidGuardCode)),
            Err(e) => return Err((self, LoginError::Transport(e))),
        }

        match poll_until_tokens(
            &self.client,
            self.client_id,
            &self.request_id,
            self.poll_interval,
        )
        .await
        {
            Ok(tokens) => Ok(ApprovedAuth {
                client: self.client,
                config: self.config,
                tokens,
            }),
            Err(e) => Err((
                GuardChallenge {
                    client: self.client,
                    config: self.config,
                    client_id: self.client_id,
                    steam_id: self.steam_id,
                    request_id: self.request_id,
                    poll_interval: self.poll_interval,
                    allowed_kinds: self.allowed_kinds,
                },
                e,
            )),
        }
    }
}

impl MobileChallenge {
    /// Poll `PollAuthSessionStatus` until the user approves on their mobile
    /// app and tokens are returned.
    pub async fn wait_for_confirmation(self) -> Result<ApprovedAuth, LoginError> {
        let tokens = poll_until_tokens(
            &self.client,
            self.client_id,
            &self.request_id,
            self.poll_interval,
        )
        .await?;
        Ok(ApprovedAuth {
            client: self.client,
            config: self.config,
            tokens,
        })
    }
}
```

- [ ] **Step 2: Verify compile**

Run: `cargo build -p steamroom-client`
Expected: clean.

- [ ] **Step 3: Commit**

```bash
jj describe -m "feat(login): add GuardChallenge::submit_code and MobileChallenge::wait_for_confirmation"
jj new
```

---

## Task 11: QR flow

Implements `QrLogin` and `QrLoginFlow`. Mirrors lines 366–397 of `authenticate_qr` in `steamroom-cli/src/main.rs`.

**Files:**
- Modify: `crates/steamroom-client/src/login/qr.rs`
- Modify: `crates/steamroom-client/src/login/mod.rs`

- [ ] **Step 1: Implement qr.rs**

Replace the contents of `crates/steamroom-client/src/login/qr.rs` with:

```rust
use crate::login::credentials::poll_until_tokens;
use crate::login::error::LoginError;
use crate::login::terminal::ApprovedAuth;
use crate::login::{BuilderConfig, TransportConfig, establish_encrypted_client};

use steamroom::auth::GuardType;
use steamroom::client::{Encrypted, SteamClient};
use steamroom::generated::CAuthenticationBeginAuthSessionViaQrRequest;

/// Configured QR login. Call [`begin()`] to start the flow.
///
/// [`begin()`]: QrLogin::begin
pub struct QrLogin {
    pub(crate) config: BuilderConfig,
    pub(crate) transport: TransportConfig,
}

/// QR auth session in progress: the caller renders `challenge_url()` as a QR
/// code (or prints it), then calls `wait_for_scan()` to block until the user
/// approves on their Steam mobile app.
pub struct QrLoginFlow {
    client: SteamClient<Encrypted>,
    config: BuilderConfig,
    challenge_url: String,
    client_id: u64,
    request_id: Vec<u8>,
    poll_interval: f32,
    allowed_kinds: Vec<GuardType>,
}

impl QrLogin {
    /// Connect (or accept BYO client) and call `BeginAuthSessionViaQR`.
    pub async fn begin(self) -> Result<QrLoginFlow, LoginError> {
        let client = establish_encrypted_client(self.transport).await?;

        let device_name = self
            .config
            .device_name
            .clone()
            .unwrap_or_else(|| "steamroom".to_string());
        let req = CAuthenticationBeginAuthSessionViaQrRequest {
            device_friendly_name: Some(device_name),
            ..Default::default()
        };
        let session = client.begin_auth_session_via_qr(req).await?;

        Ok(QrLoginFlow {
            client,
            config: self.config,
            challenge_url: session
                .challenge_url
                .ok_or(LoginError::MissingField("challenge_url"))?,
            client_id: session
                .client_id
                .ok_or(LoginError::MissingField("client_id"))?,
            request_id: session
                .request_id
                .ok_or(LoginError::MissingField("request_id"))?,
            poll_interval: session.poll_interval.unwrap_or(5.0),
            allowed_kinds: session.allowed_confirmations,
        })
    }
}

impl QrLoginFlow {
    /// URL to encode as a QR code or print for the user. The caller picks
    /// the renderer (the steamroom CLI uses the `qrcode` crate).
    pub fn challenge_url(&self) -> &str {
        &self.challenge_url
    }

    /// Confirmation kinds Steam reported as acceptable (informational —
    /// always mobile confirmation for QR sessions).
    pub fn allowed_kinds(&self) -> &[GuardType] {
        &self.allowed_kinds
    }

    /// Poll `PollAuthSessionStatus` until the user scans + approves and
    /// tokens are returned.
    pub async fn wait_for_scan(self) -> Result<ApprovedAuth, LoginError> {
        let tokens = poll_until_tokens(
            &self.client,
            self.client_id,
            &self.request_id,
            self.poll_interval,
        )
        .await?;
        Ok(ApprovedAuth {
            client: self.client,
            config: self.config,
            tokens,
        })
    }
}
```

- [ ] **Step 2: Re-export from mod.rs**

In `crates/steamroom-client/src/login/mod.rs`, change `mod qr;` to:

```rust
mod qr;
pub use qr::{QrLogin, QrLoginFlow};
```

- [ ] **Step 3: Verify compile**

Run: `cargo build -p steamroom-client`
Expected: clean.

- [ ] **Step 4: Commit**

```bash
jj describe -m "feat(login): add QR login flow"
jj new
```

---

## Task 12: Wire `with_credentials` and `with_qr` into `LoginBuilder`

**Files:**
- Modify: `crates/steamroom-client/src/login/mod.rs`

- [ ] **Step 1: Add the two terminal methods**

In `crates/steamroom-client/src/login/mod.rs`, inside the `impl LoginBuilder` block, after `with_refresh_token`, add:

```rust
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
```

- [ ] **Step 2: Verify compile**

Run: `cargo build -p steamroom-client`
Expected: clean.

- [ ] **Step 3: Add a test that all four terminal methods are callable**

In the `#[cfg(test)] mod tests` block in `mod.rs`, append:

```rust
    #[test]
    fn all_terminal_methods_compile() {
        // Just exercising the API surface — no I/O.
        let _ = LoginBuilder::new().anonymous();
        let _ = LoginBuilder::new().with_refresh_token("u", "t");
        let _ = LoginBuilder::new().with_credentials("u", "p");
        let _ = LoginBuilder::new().with_qr();
    }
```

- [ ] **Step 4: Run tests**

Run: `cargo test -p steamroom-client login`
Expected: 4 passed.

- [ ] **Step 5: Commit**

```bash
jj describe -m "feat(login): wire credentials and QR methods into LoginBuilder"
jj new
```

---

## Task 13: `PreparedLoginBuilder`

The BYO-encrypted-client builder. Duplicates the common setters and terminal methods from `LoginBuilder` but omits the transport-policy methods (those are meaningless when the caller has already chosen and built the transport).

**Files:**
- Modify: `crates/steamroom-client/src/login/mod.rs`

- [ ] **Step 1: Add `PreparedLoginBuilder`**

In `crates/steamroom-client/src/login/mod.rs`, after the `impl Default for LoginBuilder` block, add:

```rust
/// Top-level builder for the BYO-encrypted-client path. Use this when you
/// have already constructed a `SteamClient<Encrypted>` yourself (e.g. via the
/// capture/replay transport in tests, or a custom transport). Transport
/// configuration methods are intentionally absent — the transport is already
/// chosen by the caller.
pub struct PreparedLoginBuilder {
    config: BuilderConfig,
    client: Option<SteamClient<Encrypted>>,
}

impl PreparedLoginBuilder {
    pub fn new(client: SteamClient<Encrypted>) -> Self {
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
```

- [ ] **Step 2: Verify compile**

Run: `cargo build -p steamroom-client`
Expected: clean.

- [ ] **Step 3: Commit**

```bash
jj describe -m "feat(login): add PreparedLoginBuilder for BYO encrypted clients"
jj new
```

---

## Task 14: Convert `LoginError` to `CliError`

The CLI needs to surface `LoginError` through its existing error machinery without losing the variant info (the password-retry loop matches on `InvalidPassword`).

**Files:**
- Modify: `crates/steamroom-cli/src/errors.rs`

- [ ] **Step 1: Add a `Login` variant and From impl**

In `crates/steamroom-cli/src/errors.rs`, in the `CliError` enum (currently lines 1–65), add a new variant before `NoCmServers`:

```rust
    #[error("{}", display_login_error(.0))]
    Login(#[from] steamroom_client::login::LoginError),
```

After the `From<steamroom::error::ConnectionError> for CliError` impl, add:

```rust
fn display_login_error(e: &steamroom_client::login::LoginError) -> String {
    use steamroom_client::login::LoginError;
    match e {
        LoginError::InvalidPassword => "invalid password".into(),
        LoginError::InvalidGuardCode => "two-factor code rejected".into(),
        LoginError::LogonFailed(r) => format!("login failed: {}", eresult_message(r)),
        LoginError::Transport(inner) => display_steam_error(inner),
        LoginError::MissingField(f) => format!("Steam response missing field: {f}"),
        LoginError::NoCmServers => "could not find any Steam CM servers to connect to".into(),
    }
}
```

- [ ] **Step 2: Verify compile**

Run: `cargo build -p steamroom-cli`
Expected: clean.

- [ ] **Step 3: Commit**

```bash
jj describe -m "refactor(cli): add LoginError to CliError"
jj new
```

---

## Task 15: Refactor CLI to use the new builder

This is the largest task — replace `connect_and_login`, `authenticate_credentials`, `authenticate_qr`, `build_token_logon`, `build_anon_logon`. Add small CLI-local drivers for the TTY parts. Preserve exact existing behavior (three-attempt password retry, saved-token short-circuit, etc.).

**Files:**
- Modify: `crates/steamroom-cli/src/main.rs`

- [ ] **Step 1: Update imports**

In `crates/steamroom-cli/src/main.rs`, find the existing `use` block near the top and ensure these imports are present (add any that are missing):

```rust
use steamroom::client::{LoggedIn, SteamClient};
use steamroom_client::login::{
    CredentialsLoginFlow, GuardType, LoginBuilder, LoginError,
};
```

Remove unused imports that the old freeform login code required: `steamroom::client::msg::ClientMsg`, `steamroom::messages::EMsg`, `steamroom::generated::CMsgClientLogon`, `steamroom::client::PROTOCOL_VERSION`, `steamroom::connection`, `steamroom::transport::websocket::WebSocketTransport`. (Verify each is not used elsewhere in `main.rs` before removing; if used, leave it.)

- [ ] **Step 2: Replace `connect_and_login` with a builder-driven dispatcher**

Delete the existing `connect_and_login` function (current `main.rs` lines 108–243). Delete `authenticate_credentials` (lines 284–364), `authenticate_qr` (lines 366–397), `build_token_logon` (lines 399–410), `build_anon_logon` (lines 412–421). Insert in their place:

```rust
async fn connect_and_login(auth: &AuthOptions) -> Result<SteamClient<LoggedIn>, CliError> {
    let builder = LoginBuilder::new()
        .device_name(auth.device_name.as_deref().unwrap_or("steamroom"));

    // --use-steam-token: prefer local Steam install's cached token.
    if auth.use_steam_token {
        let username = auth.username.clone().or_else(|| {
            let dir = steamroom_client::steam_creds::steam_dir()?;
            steamroom_client::steam_creds::detect_username(&dir)
        });
        let cached = username.as_deref().and_then(|u| {
            info!("extracting cached Steam token for {u}...");
            steamroom_client::steam_creds::extract_token(u)
        });
        if let Some(cred) = cached {
            info!("using cached Steam token for {}", cred.account_name);
            return Ok(builder
                .with_refresh_token(cred.account_name, cred.refresh_token)
                .login()
                .await?);
        }
        warn!("failed to extract Steam token, falling back to normal auth");
        if let Some(u) = username
            && let Some(token) = load_saved_token(&u)
        {
            info!("using saved refresh token for {u}");
            return Ok(builder.with_refresh_token(u, token).login().await?);
        }
        return Ok(builder.anonymous().login().await?);
    }

    // -u/--username given: try saved token, then QR, then password.
    if let Some(ref username) = auth.username {
        if let Some(token) = load_saved_token(username) {
            info!("using saved refresh token for {username}");
            return Ok(builder.with_refresh_token(username, token).login().await?);
        }
        if auth.qr {
            return drive_qr_flow(builder, username).await;
        }
        return drive_credentials_flow(builder, username, auth).await;
    }

    // Auto-detect Steam user with a saved token.
    if let Some((username, token)) = detect_steam_user() {
        info!("auto-detected Steam user: {username}");
        return Ok(builder.with_refresh_token(username, token).login().await?);
    }

    // Last resort: anonymous.
    Ok(builder.anonymous().login().await?)
}
```

- [ ] **Step 3: Add `drive_credentials_flow` (TTY: password prompts, code prompts, token save)**

After the new `connect_and_login`, add:

```rust
async fn drive_credentials_flow(
    builder: LoginBuilder,
    username: &str,
    auth: &AuthOptions,
) -> Result<SteamClient<LoggedIn>, CliError> {
    for attempt in 0..3 {
        let password = if attempt == 0 {
            auth.password.clone().unwrap_or_else(|| {
                rpassword::prompt_password(format!("Password for {username}: "))
                    .unwrap_or_default()
            })
        } else {
            eprintln!("Invalid password, try again ({}/3)", attempt + 1);
            rpassword::prompt_password(format!("Password for {username}: "))
                .unwrap_or_default()
        };

        let credentials = LoginBuilder::new()
            .device_name(auth.device_name.as_deref().unwrap_or("steamroom"))
            .with_credentials(username, password);
        let flow = match credentials.begin().await {
            Ok(f) => f,
            Err(LoginError::InvalidPassword) => continue,
            Err(e) => return Err(e.into()),
        };

        let approved = match flow {
            CredentialsLoginFlow::Approved(a) => a,
            CredentialsLoginFlow::NeedsGuardCode(mut challenge) => loop {
                let prompt = guard_prompt(challenge.allowed_kinds());
                let kind = preferred_kind(challenge.allowed_kinds());
                let code = rpassword::prompt_password(prompt).unwrap_or_default();
                match challenge.submit_code(&code, kind).await {
                    Ok(a) => break a,
                    Err((c, LoginError::InvalidGuardCode)) => {
                        eprintln!("Invalid Steam Guard code, try again.");
                        challenge = c;
                    }
                    Err((_, e)) => return Err(e.into()),
                }
            },
            CredentialsLoginFlow::NeedsMobileConfirm(mobile) => {
                info!("confirm login on your Steam mobile app...");
                mobile.wait_for_confirmation().await?
            }
        };

        let tokens = approved.tokens();
        save_token(
            tokens.account_name.as_deref().unwrap_or(username),
            &tokens.refresh_token,
        );
        return Ok(approved.finish().await?);

        // (unreachable) — the loop only `continue`s on InvalidPassword above.
    }
    // Three attempts exhausted.
    Err(CliError::Login(LoginError::InvalidPassword))
}

fn guard_prompt(kinds: &[GuardType]) -> &'static str {
    if kinds.contains(&GuardType::DeviceCode) {
        "Steam Guard code (from authenticator app): "
    } else if kinds.contains(&GuardType::EmailCode) {
        "Steam Guard code (from email): "
    } else {
        "Steam Guard code: "
    }
}

fn preferred_kind(kinds: &[GuardType]) -> GuardType {
    if kinds.contains(&GuardType::DeviceCode) {
        GuardType::DeviceCode
    } else if kinds.contains(&GuardType::EmailCode) {
        GuardType::EmailCode
    } else {
        kinds.first().copied().unwrap_or(GuardType::DeviceCode)
    }
}
```

- [ ] **Step 4: Add `drive_qr_flow` (TTY: QR render, token save)**

After `drive_credentials_flow`, add:

```rust
async fn drive_qr_flow(
    builder: LoginBuilder,
    username: &str,
) -> Result<SteamClient<LoggedIn>, CliError> {
    info!("generating QR code...");
    let flow = builder.with_qr().begin().await?;

    let url = flow.challenge_url();
    let qr = qrcode::QrCode::new(url.as_bytes())
        .map_err(|e| CliError::Io(std::io::Error::other(e)))?;
    let rendered = qr.render::<qrcode::render::unicode::Dense1x2>().build();
    eprintln!("{rendered}");
    eprintln!("Scan this QR code with the Steam mobile app");
    eprintln!("Or open: {url}");

    let approved = flow.wait_for_scan().await?;
    let tokens = approved.tokens();
    save_token(
        tokens.account_name.as_deref().unwrap_or(username),
        &tokens.refresh_token,
    );
    Ok(approved.finish().await?)
}
```

- [ ] **Step 5: Verify it builds**

Run: `cargo build -p steamroom-cli`
Expected: clean. May still have unused-import warnings — fix them by removing dead imports.

- [ ] **Step 6: Verify clippy is happy**

Run: `cargo clippy -p steamroom-cli -p steamroom-client --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 7: Run full test suite**

Run: `cargo test --workspace`
Expected: all existing tests pass plus the new login tests.

- [ ] **Step 8: Manual smoke test — anonymous logon path**

Spacewar (app 480) is the canonical free app and works anonymously.

Run: `cargo run -p steamroom-cli -- info --app 480`
Expected: Anonymous login succeeds; app metadata prints.

- [ ] **Step 9: Commit**

```bash
jj describe -m "refactor(cli): use LoginBuilder for authentication"
jj new
```

---

## Task 16: Self-review and documentation polish

- [ ] **Step 1: Look at the new module with fresh eyes**

Read `crates/steamroom-client/src/login/mod.rs` end-to-end. Check that:
- The doc comment at the top of `mod.rs` is accurate and useful.
- `LoginBuilder` and `PreparedLoginBuilder` have rustdoc.
- The `pub use` re-exports cover everything a caller needs to import.
- There are no `// TODO`, `// FIXME`, or dead `pub` items.

- [ ] **Step 2: Verify the public API surface**

Run: `cargo doc -p steamroom-client --no-deps`
Expected: clean. Open `target/doc/steamroom_client/login/index.html` and verify each public type is documented. (Visually inspect; no automation here.)

- [ ] **Step 3: Run full check one more time**

Run: `cargo test --workspace && cargo clippy --workspace --all-targets -- -D warnings`
Expected: clean.

- [ ] **Step 4: Manual smoke test — saved-token path (if a saved token exists)**

If `~/.depotdownloader/tokens.json` exists for any account from prior CLI use:

Run: `cargo run -p steamroom-cli -- --username <that_user> info --app 480`
Expected: "using saved refresh token for …" → login succeeds.

If no saved token, skip this step.

- [ ] **Step 5: Commit any documentation tweaks**

```bash
jj describe -m "docs(login): polish module documentation"
jj new
```

(If nothing changed in step 1, skip the commit. `jj st` will show no changes.)

---

## Follow-ups (intentionally deferred)

- **Replay-fixture integration tests.** The spec mentions integration tests using `PreparedLoginBuilder::new(client)` + `steamroom::transport::replay` against recorded fixtures. Recording auth fixtures involves real account credentials and tokens, so it's deliberately separated from this change. The new `PreparedLoginBuilder` makes the necessary entry point available; recording + adding the tests is a follow-up.
- **steamroom-ffi migration.** `steamroom-ffi/src/inner.rs` currently has its own `do_connect_anon` / `do_connect_token`. It can move to the new builder once this lands; out of scope here.
- **Refresh-token rotation.** If Steam returns a rotated refresh token in `CLIENT_LOG_ON_RESPONSE`, we don't surface it today and this change doesn't add it.

## Latent-bug note (worth recording in commit msg or PR description)

The existing CLI's password-retry loop matches `LogonFailed(InvalidPassword)`. That variant never fires in this flow — Steam rejects bad passwords at `BeginAuthSessionViaCredentials`, which surfaces as `ServiceMethodFailed(InvalidPassword)`. The new builder maps that case to `LoginError::InvalidPassword`, so Task 15's retry loop is the first one that will actually retry on bad passwords. (Worth mentioning in the PR description if reviewers expect bug-for-bug parity.)

## Verification checklist (run before declaring done)

- [ ] `cargo build --workspace` — clean
- [ ] `cargo test --workspace` — all passing, including new login tests
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` — clean
- [ ] `cargo run -p steamroom-cli -- info --app 480` — anonymous login works
- [ ] Old freeform login functions (`connect_and_login` minus its new builder-driven body, `authenticate_credentials`, `authenticate_qr`, `build_token_logon`, `build_anon_logon`) are gone from `steamroom-cli/src/main.rs`
- [ ] New module structure matches the **File Structure** section
- [ ] No `Co-Authored-By:` trailers on any commits
- [ ] All commits use conventional-commit subjects via `jj`
