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
// `pub use steamroom::auth::{AuthTokens, GuardType};` — restored once available
