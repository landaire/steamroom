use crate::enums::EResultError;
use crate::messages::EMsg;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    #[error(transparent)]
    Connection(#[from] ConnectionError),

    #[error(transparent)]
    Crypto(#[from] CryptoError),

    #[error("protobuf decode: {0}")]
    ProtobufDecode(#[from] prost::DecodeError),

    #[error(transparent)]
    Manifest(#[from] ManifestError),

    #[error("http: {0}")]
    Http(#[from] reqwest::Error),

    #[error("CDN returned HTTP {}", status.as_u16())]
    CdnStatus {
        status: reqwest::StatusCode,
        retry_after: Option<u64>,
    },

    #[error("io: {0}")]
    Io(#[from] std::io::Error),

    #[error(transparent)]
    Parse(#[from] ParseError),
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ConnectionError {
    #[error("unexpected emsg: expected {expected:?}, got {got:?}")]
    UnexpectedEMsg { expected: EMsg, got: EMsg },

    #[error("bad magic: {0:#010x}")]
    BadMagic(u32),

    #[error("packet too short: {len} bytes")]
    PacketTooShort { len: usize },

    #[error("DNS resolution failed")]
    DnsResolutionFailed,

    #[error("encryption failed")]
    EncryptionFailed,

    #[error("logon failed: {0:?}")]
    LogonFailed(EResultError),

    #[error("service method failed: {0:?}")]
    ServiceMethodFailed(EResultError),

    #[error("depot access denied: depot {0}")]
    DepotAccessDenied(u32),

    #[error("server response missing required field: {0}")]
    MissingField(&'static str),

    #[error(transparent)]
    Parse(#[from] ParseError),

    #[error("disconnected")]
    Disconnected,

    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum CryptoError {
    #[error("invalid key length: {0}")]
    InvalidKeyLength(usize),

    #[error("decryption failed")]
    DecryptionFailed,

    #[error("invalid padding")]
    InvalidPadding,

    #[error("rsa: {0}")]
    Rsa(rsa::Error),
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ManifestError {
    #[error("invalid section magic: {0:#010x}")]
    InvalidMagic(u32),

    #[error("missing payload section")]
    MissingSection,

    #[error("checksum mismatch: expected {expected:#010x}, got {got:#010x}")]
    ChecksumMismatch { expected: u32, got: u32 },

    #[error("failed to decode protobuf section")]
    ProtobufDecode(#[from] prost::DecodeError),

    #[error("filename decryption failed")]
    DecryptFailed(#[from] CryptoError),

    #[error("invalid base64 in encrypted filename")]
    Base64Decode(#[from] base64::DecodeError),

    #[error("decrypted filename is not valid UTF-8")]
    InvalidFilename,
}

#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ParseError {
    #[error("unexpected EOF")]
    UnexpectedEof,

    #[error("invalid protobuf header")]
    InvalidProtobufHeader,

    #[error("parse error: {0}")]
    Winnow(String),
}
