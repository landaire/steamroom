# Login builder for `steamroom-client`

## Background

Today, login logic is spread across several freeform functions in `steamroom-cli/src/main.rs`:

- `connect_and_login` — CM server discovery, transport selection, encryption handshake, auth-method dispatch
- `authenticate_credentials` — RSA exchange, password encryption, `BeginAuthSessionViaCredentials`, 2FA, polling
- `authenticate_qr` — QR session, polling
- `build_token_logon`, `build_anon_logon` — `CMsgClientLogon` construction

The protocol primitives in `steamroom::client::SteamClient<Encrypted>` (e.g. `begin_auth_session_via_credentials`, `poll_auth_session`, `submit_steam_guard_code`) are sound. What's missing is an orchestration layer that:

1. Drives the full sequence (discover → connect → encrypt → auth → logon)
2. Handles the 2FA branch the user has to walk through
3. Lives in a place other consumers (FFI, future SDK users) can use

This spec defines that layer.

## Scope

In:

- A `LoginBuilder` in `steamroom-client` that handles CM server discovery, transport, encryption handshake, the OAuth-like auth flow, and the final CM logon.
- Typestate-based state machine for the 2FA branch (credentials and QR).
- Refactor of `steamroom-cli` to use the new builder; freeform login functions delete.

Out:

- Token persistence inside the builder. The builder is pure (in-memory). The CLI continues to use `steamroom_client::credentials::TokenStore` to save/load tokens.
- Changes to `steamroom::auth` types (`AuthSession`, `QrAuthSession`, `AuthTokens`, `GuardType`) — they're re-exported as-is.
- FFI migration. The FFI (`steamroom-ffi/src/inner.rs`) keeps its existing inlined login logic for now; migration is a follow-up.
- Refresh-token rotation handling on logon-with-token. If Steam returns a rotated refresh token in `CLIENT_LOG_ON_RESPONSE`, we don't surface it today and won't in this change.
- Sans-io refactor. The whole of `steamroom` is async-only today and a sans-io conversion is a project of its own; doing it just for login would force this layer to bypass / reimplement the existing async client methods. The public state types here (`CredentialsLoginFlow`, `GuardChallenge`, `ApprovedAuth`, etc.) are plain owned values whose methods happen to be async — a future sans-io refactor at the `SteamClient` level can swap the driving without changing the typestate shape callers use.

## Crate placement

`steamroom-client/src/login.rs` (new module).

Rationale: `steamroom` is documented as "low-level building blocks"; `steamroom-client` is documented as "high-level orchestration". The pieces this builder ties together (`TokenStore`, `steam_creds`) already live in `steamroom-client`. Keeps `steamroom`'s dependency surface lean.

## Design decisions

| Decision | Choice | Why |
|---|---|---|
| Scope | Builder owns discovery → transport → encryption → auth → logon | User wants a single high-level entry point. |
| Escape hatch | Separate `PreparedLoginBuilder` constructed from a pre-built `SteamClient<Encrypted>` | Lets capture/replay tests and other callers skip discovery/transport. Typestate split makes the conflict between BYO-client and transport-policy methods unrepresentable. |
| 2FA | Typestate state machine (no callbacks) | Cleaner for FFI / sync wrappers; "can't forget a step" property; user explicitly requested. |
| Per-method vs unified flow enum | Per-method (`CredentialsLoginFlow`, `QrLoginFlow`) | QR has no concept of guard codes; password has no `challenge_url`. No impossible states. |
| Token persistence | Caller owns it. Tokens exposed via `ApprovedAuth::tokens()` before the final logon step. | Builder stays pure; caller decides where to store. |
| Transport policy | Default = TCP-then-WS (matches today's CLI). Configurable via `prefer_protocol` / `allow_protocol_fallback`. | FFI / WASM targets may need to force WS. |
| Guard-code retry | `submit_code` returns `Result<ApprovedAuth, (GuardChallenge, LoginError)>` | Wrong code shouldn't force restarting the whole RSA exchange. |
| OS type | `ClientOs(i32)` newtype with documented constants and `::new(i32)` escape hatch | Steam's `EOSType` has hundreds of entries; enumerating is futile, but a bare `i32` parameter is opaque. |

## Public API

### Builders

Two top-level builder types. Required auth-method inputs (`account`, `password`,
`refresh_token`) are taken inline by the transition methods, so there's no
"forgot a required field" failure mode — the type doesn't let you reach a
terminal step without committing to an auth method that takes the right inputs.

The BYO-client path and the auto-discovery path are split into two distinct
builder types so that transport-policy methods (`prefer_protocol`,
`allow_protocol_fallback`) are not callable when they'd be meaningless.

#### `LoginBuilder` (auto-discovery path)

```rust
pub struct LoginBuilder { /* private */ }

impl LoginBuilder {
    pub fn new() -> Self;

    pub fn device_name(self, name: impl Into<String>) -> Self;
    pub fn cell_id(self, id: u32) -> Self;
    pub fn login_id(self, id: u32) -> Self;
    pub fn client_os(self, os: ClientOs) -> Self;

    pub fn prefer_protocol(self, p: steamroom::connection::Protocol) -> Self;
    pub fn allow_protocol_fallback(self, allow: bool) -> Self;

    // Terminal: choose an auth method.
    pub fn anonymous(self) -> AnonymousLogin;
    pub fn with_refresh_token(self,
        account: impl Into<String>,
        refresh_token: impl Into<String>,
    ) -> TokenLogin;
    pub fn with_credentials(self,
        account: impl Into<String>,
        password: impl Into<String>,
    ) -> CredentialsLogin;
    pub fn with_qr(self) -> QrLogin;
}

impl Default for LoginBuilder { /* … */ }
```

#### `PreparedLoginBuilder` (BYO-client path)

```rust
pub struct PreparedLoginBuilder { /* private */ }

impl PreparedLoginBuilder {
    /// Build on top of an already-encrypted client (e.g. one from the
    /// capture/replay transport, or constructed manually with a non-default
    /// transport).
    pub fn new(client: SteamClient<Encrypted>) -> Self;

    pub fn device_name(self, name: impl Into<String>) -> Self;
    pub fn cell_id(self, id: u32) -> Self;
    pub fn login_id(self, id: u32) -> Self;
    pub fn client_os(self, os: ClientOs) -> Self;

    // No prefer_protocol / allow_protocol_fallback — the transport is already
    // chosen by the caller.

    // Same terminal methods as LoginBuilder.
    pub fn anonymous(self) -> AnonymousLogin;
    pub fn with_refresh_token(self, /* … */) -> TokenLogin;
    pub fn with_credentials(self, /* … */) -> CredentialsLogin;
    pub fn with_qr(self) -> QrLogin;
}
```

The terminal types (`AnonymousLogin`, `TokenLogin`, `CredentialsLogin`,
`QrLogin`) are the same regardless of which builder produced them — they hold
either a `TransportConfig::Auto { … }` or a `TransportConfig::Provided(client)`
internally. Implementation can share the terminal-method bodies via a small
private trait or free function; the duplication is only at the signature
level.

### Terminal flows (no state machine)

```rust
pub struct AnonymousLogin { /* private */ }
impl AnonymousLogin {
    pub async fn login(self) -> Result<SteamClient<LoggedIn>, LoginError>;
}

pub struct TokenLogin { /* private */ }
impl TokenLogin {
    /// The refresh token is placed in `CMsgClientLogon::access_token`.
    /// (Confusing wire name — but that's the field Steam expects.)
    pub async fn login(self) -> Result<SteamClient<LoggedIn>, LoginError>;
}
```

### Credentials flow

```rust
pub struct CredentialsLogin { /* private */ }
impl CredentialsLogin {
    /// Connect (or accept BYO client), fetch the RSA pubkey, RSA-encrypt the
    /// password, and call BeginAuthSessionViaCredentials. CLIENT_HELLO is sent
    /// later by `ApprovedAuth::finish()` as part of the logon.
    pub async fn begin(self) -> Result<CredentialsLoginFlow, LoginError>;
}

#[non_exhaustive]
pub enum CredentialsLoginFlow {
    Approved(ApprovedAuth),
    NeedsGuardCode(GuardChallenge),
    NeedsMobileConfirm(MobileChallenge),
}

pub struct GuardChallenge { /* private */ }
impl GuardChallenge {
    pub fn allowed_kinds(&self) -> &[GuardType];

    /// Submit a code. On `InvalidGuardCode`, returns the live challenge so
    /// the caller can prompt again without restarting the RSA exchange.
    pub async fn submit_code(self, code: &str, kind: GuardType)
        -> Result<ApprovedAuth, (GuardChallenge, LoginError)>;
}

pub struct MobileChallenge { /* private */ }
impl MobileChallenge {
    /// Block until the user approves on their Steam mobile app. Polls
    /// `PollAuthSessionStatus` at the interval Steam returned.
    pub async fn wait_for_confirmation(self) -> Result<ApprovedAuth, LoginError>;
}
```

### Approved authentication (shared by credentials and QR)

```rust
pub struct ApprovedAuth { /* private */ }
impl ApprovedAuth {
    /// Inspect tokens before committing to the logon. Caller can persist the
    /// refresh token here if desired.
    pub fn tokens(&self) -> &AuthTokens;

    /// Send CMsgClientLogon with the access token; wait for CLIENT_LOG_ON_RESPONSE.
    pub async fn finish(self) -> Result<SteamClient<LoggedIn>, LoginError>;
}
```

### QR flow

```rust
pub struct QrLogin { /* private */ }
impl QrLogin {
    /// Connect (or accept BYO client) and call BeginAuthSessionViaQR.
    pub async fn begin(self) -> Result<QrLoginFlow, LoginError>;
}

pub struct QrLoginFlow { /* private */ }
impl QrLoginFlow {
    /// The URL to render as a QR code. The caller picks the renderer
    /// (the CLI uses the `qrcode` crate).
    pub fn challenge_url(&self) -> &str;

    pub fn allowed_kinds(&self) -> &[GuardType];

    /// Block until the user scans + approves on their Steam mobile app.
    pub async fn wait_for_scan(self) -> Result<ApprovedAuth, LoginError>;
}
```

### Supporting types

```rust
/// Steam's `client_os_type` value (its internal EOSType — hundreds of entries
/// covering OS + version combinations). Use `ClientOs::new` for values not
/// listed as constants here. Steam doesn't reject unknown values.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct ClientOs(i32);

impl ClientOs {
    pub const fn new(value: i32) -> Self;
    pub const fn value(self) -> i32;

    /// Windows 11 (wire value 20). The default; matches the current CLI.
    pub const WINDOWS_11: Self;
}

impl Default for ClientOs { /* WINDOWS_11 */ }
```

```rust
// Re-exported from steamroom::auth for caller convenience.
pub use steamroom::auth::{AuthTokens, GuardType};
```

### Errors

```rust
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum LoginError {
    #[error("transport / connection error: {0}")]
    Transport(#[from] steamroom::Error),

    #[error("logon rejected by Steam: {0:?}")]
    LogonFailed(steamroom::enums::EResultError),

    #[error("invalid password")]
    InvalidPassword,

    #[error("two-factor code rejected")]
    InvalidGuardCode,

    #[error("Steam response missing field: {0}")]
    MissingField(&'static str),

    #[error("no CM servers available")]
    NoCmServers,
}
```

`InvalidPassword` and `InvalidGuardCode` are promoted to top-level variants because they're the expected user-recoverable cases. Today the CLI matches against three nested layers of error to retry on `InvalidPassword`.

## CLI refactor

`steamroom-cli/src/main.rs`:

- Delete `connect_and_login`, `authenticate_credentials`, `authenticate_qr`, `build_token_logon`, `build_anon_logon`.
- New `connect_and_login` is a small dispatcher that picks an auth method based on `AuthOptions` and delegates to the builder.
- New CLI-local helpers `drive_credentials_flow` and `drive_qr_flow` for the parts that genuinely need a TTY: `rpassword` prompts, QR rendering, calling `TokenStore::save` after a successful auth.

Existing CLI behavior preserved:

- Three-attempt retry on `InvalidPassword`
- Saved-token short-circuit before prompting for password
- `--use-steam-token` falls back to normal auth on extraction failure
- Auto-detection of Steam user when no `--username` given
- Anonymous logon as the last-resort default

The `lib.rs` of `steamroom-client` gains `pub mod login;`. No other modules change.

## Testing

- **Unit tests** for builder construction and the state-machine transitions where no I/O happens.
- **Integration tests** using `PreparedLoginBuilder::new(client)` plus the existing `steamroom::transport::replay` machinery to exercise the credentials and QR flows end-to-end against recorded fixtures.
- The CLI gains no new testable surface — it becomes a thin adapter over the builder.

## Migration notes

- No public API breakage in `steamroom`. The `auth` module's existing types are re-exported by `steamroom-client::login` so callers don't need both imports.
- `steamroom-ffi` is unaffected by this change. A follow-up can migrate `connect_anonymous` and `connect_with_token` to the new builder once it's settled.
