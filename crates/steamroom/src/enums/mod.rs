#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum EAccountType {
    Invalid,
    Individual,
    Multiseat,
    GameServer,
    AnonGameServer,
    Pending,
    ContentServer,
    Clan,
    Chat,
    AnonUser,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum EAuthTokenPlatformType {
    Unknown,
    SteamClient,
    WebBrowser,
    MobileApp,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum EOSType {
    LinuxUnknown,
    MacOSUnknown,
    WindowsUnknown,
    Windows11,
    Windows10,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum EResultError {
    Fail,
    NoConnection,
    InvalidPassword,
    LoggedInElsewhere,
    InvalidProtocolVer,
    InvalidParam,
    FileNotFound,
    Busy,
    InvalidState,
    InvalidName,
    InvalidEmail,
    DuplicateName,
    AccessDenied,
    Timeout,
    Banned,
    AccountNotFound,
    InvalidSteamID,
    ServiceUnavailable,
    NotLoggedOn,
    Pending,
    EncryptionFailure,
    InsufficientPrivilege,
    LimitExceeded,
    Revoked,
    Expired,
    AlreadyRedeemed,
    DuplicateRequest,
    AlreadyOwned,
    IPNotFound,
    PersistFailed,
    LockingFailed,
    LogonSessionReplaced,
    RateLimitExceeded,
    TwoFactorRequired,
    LoginDeniedThrottle,
    TwoFactorCodeMismatch,
    TwoFactorActivationCodeMismatch,
    Unknown(i32),
}

/// Check a raw Steam EResult code. Returns `Ok(())` for success (1),
/// `Err(EResultError)` for everything else. There is no intermediate type
/// to forget to check.
const ERESULT_OK: i32 = 1;

pub fn eresult(code: i32) -> Result<(), EResultError> {
    if code == ERESULT_OK {
        Ok(())
    } else {
        Err(EResultError::from_code(code))
    }
}

impl EResultError {
    pub fn from_code(code: i32) -> Self {
        match code {
            2 => Self::Fail,
            3 => Self::NoConnection,
            5 => Self::InvalidPassword,
            6 => Self::LoggedInElsewhere,
            7 => Self::InvalidProtocolVer,
            8 => Self::InvalidParam,
            9 => Self::FileNotFound,
            10 => Self::Busy,
            11 => Self::InvalidState,
            12 => Self::InvalidName,
            13 => Self::InvalidEmail,
            14 => Self::DuplicateName,
            15 => Self::AccessDenied,
            16 => Self::Timeout,
            17 => Self::Banned,
            18 => Self::AccountNotFound,
            19 => Self::InvalidSteamID,
            20 => Self::ServiceUnavailable,
            21 => Self::NotLoggedOn,
            22 => Self::Pending,
            23 => Self::EncryptionFailure,
            24 => Self::InsufficientPrivilege,
            25 => Self::LimitExceeded,
            26 => Self::Revoked,
            27 => Self::Expired,
            28 => Self::AlreadyRedeemed,
            29 => Self::DuplicateRequest,
            30 => Self::AlreadyOwned,
            31 => Self::IPNotFound,
            32 => Self::PersistFailed,
            33 => Self::LockingFailed,
            34 => Self::LogonSessionReplaced,
            84 => Self::RateLimitExceeded,
            85 => Self::TwoFactorRequired,
            88 => Self::TwoFactorCodeMismatch,
            89 => Self::TwoFactorActivationCodeMismatch,
            other => Self::Unknown(other),
        }
    }
}

impl std::fmt::Display for EResultError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unknown(code) => write!(f, "unknown error (code {code})"),
            other => write!(f, "{other:?}"),
        }
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ESessionPersistence {
    Ephemeral,
    Persistent,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum EUniverse {
    Invalid,
    Public,
    Beta,
    Internal,
    Dev,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum ManifestMagic {
    PayloadV5,
    Metadata,
    Signature,
    EndOfManifest,
    V4,
}

bitflags::bitflags! {
    /// Per-file flags from a depot manifest (`FileMapping.flags`), i.e. Steam's
    /// `EDepotFileFlag` bitfield.
    ///
    /// The bit values are not a plain sequence of powers of two starting at 1:
    /// `Directory` is `0x40`, not `0x2`. They are taken from the Steam content
    /// client rather than a third-party SDK. `Symlink` (`0x200`) is the bit an
    /// assertion in `contentmanifest.cpp` tests (`m_unFlags & k_EDepotFileFlagSymlink`),
    /// and `Directory` (`0x40`) matches the flag carried by real directory
    /// entries in live manifests; the rest follow the same contiguous order.
    #[derive(Copy, Clone, Debug, Eq, PartialEq)]
    pub struct DepotFileFlags: u32 {
        const USER_CONFIG = 0x1;
        const VERSIONED_USER_CONFIG = 0x2;
        const ENCRYPTED = 0x4;
        const READ_ONLY = 0x8;
        const HIDDEN = 0x10;
        const EXECUTABLE = 0x20;
        const DIRECTORY = 0x40;
        const CUSTOM_EXECUTABLE = 0x80;
        const INSTALL_SCRIPT = 0x100;
        const SYMLINK = 0x200;
    }
}

impl DepotFileFlags {
    pub fn is_directory(self) -> bool {
        self.contains(Self::DIRECTORY)
    }

    pub fn is_executable(self) -> bool {
        self.contains(Self::EXECUTABLE)
    }

    pub fn is_symlink(self) -> bool {
        self.contains(Self::SYMLINK)
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
#[non_exhaustive]
pub enum EBillingType {
    NoCost,
    Store,
    BillMonthly,
    CdKey,
    GuestPass,
    HardwarePromo,
    Gift,
    FreeWeekend,
    OemKey,
    StoreOrCdKey,
    FreeOnDemand,
    FreeCommercial,
    Unknown(i32),
}

impl EBillingType {
    pub fn from_i32(v: i32) -> Self {
        match v {
            0 => Self::NoCost,
            1 => Self::Store,
            2 => Self::BillMonthly,
            3 => Self::CdKey,
            4 => Self::GuestPass,
            5 => Self::HardwarePromo,
            6 => Self::Gift,
            7 => Self::FreeWeekend,
            8 => Self::OemKey,
            10 => Self::StoreOrCdKey,
            12 => Self::FreeOnDemand,
            14 => Self::FreeCommercial,
            other => Self::Unknown(other),
        }
    }
}

impl std::fmt::Display for EBillingType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoCost => write!(f, "No Cost"),
            Self::Store => write!(f, "Store"),
            Self::BillMonthly => write!(f, "Bill Monthly"),
            Self::CdKey => write!(f, "CD Key"),
            Self::GuestPass => write!(f, "Guest Pass"),
            Self::HardwarePromo => write!(f, "Hardware Promo"),
            Self::Gift => write!(f, "Gift"),
            Self::FreeWeekend => write!(f, "Free Weekend"),
            Self::OemKey => write!(f, "OEM Key"),
            Self::StoreOrCdKey => write!(f, "Store or CD Key"),
            Self::FreeOnDemand => write!(f, "Free on Demand"),
            Self::FreeCommercial => write!(f, "Free Commercial"),
            Self::Unknown(v) => write!(f, "Unknown ({v})"),
        }
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
#[non_exhaustive]
pub enum ELicenseType {
    NoLicense,
    SinglePurchase,
    SinglePurchaseLimitedUse,
    RecurringCharge,
    RecurringLimitedUse,
    LimitUseDelayedActivation,
    Unknown(i32),
}

impl ELicenseType {
    pub fn from_i32(v: i32) -> Self {
        match v {
            0 => Self::NoLicense,
            1 => Self::SinglePurchase,
            2 => Self::SinglePurchaseLimitedUse,
            3 => Self::RecurringCharge,
            6 => Self::RecurringLimitedUse,
            7 => Self::LimitUseDelayedActivation,
            other => Self::Unknown(other),
        }
    }
}

impl std::fmt::Display for ELicenseType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoLicense => write!(f, "No License"),
            Self::SinglePurchase => write!(f, "Single Purchase"),
            Self::SinglePurchaseLimitedUse => write!(f, "Single Purchase (Limited Use)"),
            Self::RecurringCharge => write!(f, "Recurring Charge"),
            Self::RecurringLimitedUse => write!(f, "Recurring (Limited Use)"),
            Self::LimitUseDelayedActivation => write!(f, "Limit Use, Delayed Activation"),
            Self::Unknown(v) => write!(f, "Unknown ({v})"),
        }
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
#[non_exhaustive]
pub enum EPackageStatus {
    Available,
    Preorder,
    Unavailable,
    Unknown(i32),
}

impl EPackageStatus {
    pub fn from_i32(v: i32) -> Self {
        match v {
            0 => Self::Available,
            1 => Self::Preorder,
            2 => Self::Unavailable,
            other => Self::Unknown(other),
        }
    }
}

impl std::fmt::Display for EPackageStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Available => write!(f, "Available"),
            Self::Preorder => write!(f, "Preorder"),
            Self::Unavailable => write!(f, "Unavailable"),
            Self::Unknown(v) => write!(f, "Unknown ({v})"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The two bit values pinned to a primary source: Directory (0x40, observed
    // on real directory entries) and Symlink (0x200, the bit Steam's own
    // contentmanifest.cpp asserts). The contiguous layout follows from them.
    #[test]
    fn depot_file_flag_values_match_steam() {
        assert_eq!(DepotFileFlags::DIRECTORY.bits(), 0x40);
        assert_eq!(DepotFileFlags::SYMLINK.bits(), 0x200);
        assert_eq!(DepotFileFlags::EXECUTABLE.bits(), 0x20);
        assert_eq!(DepotFileFlags::READ_ONLY.bits(), 0x8);
        assert_eq!(DepotFileFlags::HIDDEN.bits(), 0x10);
    }

    #[test]
    fn directory_entry_is_recognized() {
        // A manifest directory entry carries flag 0x40.
        assert!(DepotFileFlags::from_bits_retain(0x40).is_directory());
        assert!(!DepotFileFlags::from_bits_retain(0x40).is_executable());
        // A plain file (no flags) is neither.
        assert!(!DepotFileFlags::from_bits_retain(0).is_directory());
        // An executable file carries flag 0x20, not the directory bit.
        assert!(DepotFileFlags::from_bits_retain(0x20).is_executable());
        assert!(!DepotFileFlags::from_bits_retain(0x20).is_directory());
        // Unmodeled bits are preserved, not truncated.
        assert!(DepotFileFlags::from_bits_retain(0x40 | 0x8000).is_directory());
    }
}
