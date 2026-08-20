use crate::types::SteamId;
use std::time::Duration;

/// Identifier for an in-progress authentication session, returned by
/// `BeginAuthSession*` and echoed back to `PollAuthSessionStatus` and
/// `UpdateAuthSessionWithSteamGuardCode`. Distinct from the CM session id
/// assigned at logon; the two are not interchangeable.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub struct AuthClientId(u64);

impl AuthClientId {
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    pub const fn raw(self) -> u64 {
        self.0
    }
}

/// Server-suggested delay between successive `PollAuthSessionStatus` calls.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct PollInterval(Duration);

impl PollInterval {
    /// Fallback used when Steam omits the `interval` field.
    pub const DEFAULT: Self = Self(Duration::from_millis(2500));

    /// Upper bound on a server-supplied interval. Real values are a few
    /// seconds; capping keeps a malformed field from wedging the poll loop.
    const MAX_SECS: f32 = 3600.0;

    /// Interpret Steam's `interval` field, which is seconds on the wire. The
    /// value is untrusted: non-finite or negative falls back to [`DEFAULT`],
    /// and absurdly large values are capped, so a malformed field can neither
    /// panic `Duration::from_secs_f32` nor stall polling.
    ///
    /// [`DEFAULT`]: PollInterval::DEFAULT
    pub fn from_secs_f32(secs: f32) -> Self {
        if !secs.is_finite() || secs < 0.0 {
            return Self::DEFAULT;
        }
        Self(Duration::from_secs_f32(secs.min(Self::MAX_SECS)))
    }

    pub const fn as_duration(self) -> Duration {
        self.0
    }
}

#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct AuthSession {
    pub client_id: Option<AuthClientId>,
    pub request_id: Option<Vec<u8>>,
    pub poll_interval: Option<PollInterval>,
    pub allowed_confirmations: Vec<GuardType>,
    pub steam_id: Option<SteamId>,
}

#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct QrAuthSession {
    pub client_id: Option<AuthClientId>,
    pub request_id: Option<Vec<u8>>,
    pub challenge_url: Option<String>,
    pub poll_interval: Option<PollInterval>,
    pub allowed_confirmations: Vec<GuardType>,
}

#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct AuthTokens {
    pub access_token: String,
    pub refresh_token: String,
    pub account_name: Option<String>,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum GuardType {
    None,
    /// One-time code delivered by email; submitted via `UpdateAuthSessionWithSteamGuardCode`.
    EmailCode,
    /// One-time code from the mobile authenticator; submitted via `UpdateAuthSessionWithSteamGuardCode`.
    DeviceCode,
    /// Out-of-band approval in the Steam mobile app; no code, the session is
    /// confirmed by polling `PollAuthSessionStatus`.
    DeviceConfirmation,
    /// Out-of-band approval via an email link; like `DeviceConfirmation`, no
    /// code is submitted.
    EmailConfirmation,
    /// A confirmation kind this client does not model, carrying its raw proto
    /// value. Retained rather than dropped so that "Steam offered only methods
    /// we cannot drive" stays distinguishable from "no 2FA required".
    Unknown(i32),
}

impl GuardType {
    /// Map a proto `EAuthSessionGuardType`. Unrecognized values become
    /// [`GuardType::Unknown`] rather than being discarded.
    pub fn from_proto(value: i32) -> Self {
        match value {
            1 => Self::None,
            2 => Self::EmailCode,
            3 => Self::DeviceCode,
            4 => Self::DeviceConfirmation,
            5 => Self::EmailConfirmation,
            other => Self::Unknown(other),
        }
    }

    pub fn to_proto(self) -> i32 {
        match self {
            Self::None => 1,
            Self::EmailCode => 2,
            Self::DeviceCode => 3,
            Self::DeviceConfirmation => 4,
            Self::EmailConfirmation => 5,
            Self::Unknown(other) => other,
        }
    }

    /// True if this is an out-of-band confirmation the user approves elsewhere
    /// (mobile app or email link), completed by polling rather than by
    /// submitting a code.
    pub fn is_confirmation(self) -> bool {
        matches!(self, Self::DeviceConfirmation | Self::EmailConfirmation)
    }

    /// True if this requires the user to submit a Steam Guard code.
    pub fn is_code(self) -> bool {
        matches!(self, Self::EmailCode | Self::DeviceCode)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn guard_type_proto_round_trips() {
        for gt in [
            GuardType::None,
            GuardType::EmailCode,
            GuardType::DeviceCode,
            GuardType::DeviceConfirmation,
            GuardType::EmailConfirmation,
            GuardType::Unknown(6),
        ] {
            assert_eq!(GuardType::from_proto(gt.to_proto()), gt);
        }
    }

    #[test]
    fn email_confirmation_is_not_dropped() {
        // Regression: proto value 5 (EmailConfirmation) was previously unmapped
        // and silently filtered out of allowed_confirmations.
        assert_eq!(GuardType::from_proto(5), GuardType::EmailConfirmation);
    }

    #[test]
    fn unknown_proto_values_are_retained() {
        // Retained (not dropped) so an all-unsupported response stays
        // distinguishable from a genuine no-2FA response.
        assert_eq!(GuardType::from_proto(0), GuardType::Unknown(0));
        assert_eq!(GuardType::from_proto(6), GuardType::Unknown(6));
    }

    #[test]
    fn code_and_confirmation_partition_is_disjoint() {
        for gt in [
            GuardType::None,
            GuardType::EmailCode,
            GuardType::DeviceCode,
            GuardType::DeviceConfirmation,
            GuardType::EmailConfirmation,
            GuardType::Unknown(6),
        ] {
            assert!(!(gt.is_code() && gt.is_confirmation()));
        }
        assert!(GuardType::DeviceCode.is_code());
        assert!(GuardType::DeviceConfirmation.is_confirmation());
        // Neither `None` nor an unknown kind is a code or a confirmation.
        assert!(!GuardType::None.is_code() && !GuardType::None.is_confirmation());
        assert!(!GuardType::Unknown(6).is_code() && !GuardType::Unknown(6).is_confirmation());
    }

    #[test]
    fn poll_interval_default_and_round_trip() {
        assert_eq!(
            PollInterval::DEFAULT.as_duration(),
            std::time::Duration::from_millis(2500)
        );
        assert_eq!(
            PollInterval::from_secs_f32(2.5).as_duration(),
            std::time::Duration::from_secs_f32(2.5)
        );
    }

    #[test]
    fn poll_interval_rejects_hostile_wire_values() {
        // Non-finite / negative fall back to the default instead of panicking
        // Duration::from_secs_f32.
        assert_eq!(PollInterval::from_secs_f32(f32::NAN), PollInterval::DEFAULT);
        assert_eq!(
            PollInterval::from_secs_f32(f32::INFINITY),
            PollInterval::DEFAULT
        );
        assert_eq!(PollInterval::from_secs_f32(-1.0), PollInterval::DEFAULT);
        // Absurdly large values are capped rather than wedging the poll loop.
        assert_eq!(
            PollInterval::from_secs_f32(1e30).as_duration(),
            std::time::Duration::from_secs_f32(PollInterval::MAX_SECS)
        );
    }
}
