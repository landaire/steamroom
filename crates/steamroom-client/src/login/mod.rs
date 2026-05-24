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
