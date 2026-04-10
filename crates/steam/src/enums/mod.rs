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
    Invalid,
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
    Unknown,
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
