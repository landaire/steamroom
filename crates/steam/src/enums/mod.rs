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

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct DepotFileFlags(pub u32);

impl DepotFileFlags {
    pub const NONE: Self = Self(0);
    pub const EXECUTABLE: Self = Self(1);
    pub const DIRECTORY: Self = Self(2);
    pub const HIDDEN: Self = Self(4);
    pub const READ_ONLY: Self = Self(8);

    pub fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub fn is_directory(self) -> bool {
        self.contains(Self::DIRECTORY)
    }

    pub fn is_executable(self) -> bool {
        self.contains(Self::EXECUTABLE)
    }
}
