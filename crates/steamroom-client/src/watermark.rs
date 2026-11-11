//! Steam config-store watermark encoding and decoding.
//!
//! Watermarking binds a value to an account or installation and detects corruption. It does not
//! provide confidentiality.

use steamroom::types::SteamId;
use thiserror::Error;

/// Largest payload accepted by Steam's config store.
pub const MAX_PAYLOAD_SIZE: usize = 0x80000;

const FRAME_OVERHEAD: usize = 8;
const ACCOUNT_SCOPE_MARKER: u32 = 1;
const MACHINE_SCOPE_MARKER: u32 = 2;
const WATERMARK_STEP: u32 = 0x25fe_6761;
const WATERMARK_TABLE: [u32; 17] = [
    0x1739_a3b0,
    0xb890_7fe1,
    0x8290_d3b7,
    0x7283_9cd0,
    0x242d_f096,
    0x3829_750b,
    0x38de_7a77,
    0x72f0_924c,
    0x4478_3927,
    0x0192_5372,
    0x2090_2714,
    0x2758_5920,
    0x2789_0632,
    0x8291_0476,
    0x7290_6721,
    0x2879_8904,
    0x7859_2700,
];

/// Seed for account-local watermarked values.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AccountWatermarkSeed(u32);

impl AccountWatermarkSeed {
    /// Construct a seed from the 32-bit Steam account ID.
    pub const fn from_account_id(account_id: u32) -> Self {
        Self(account_id)
    }

    /// Construct a seed from a full Steam ID.
    pub const fn from_steam_id(steam_id: SteamId) -> Self {
        Self(steam_id.account_id())
    }

    /// Return the 32-bit Steam account ID.
    pub const fn account_id(self) -> u32 {
        self.0
    }
}

/// Seed for machine-local watermarked values.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MachineWatermarkSeed(u32);

impl MachineWatermarkSeed {
    /// Construct a seed from the config store's `MTBF` value.
    pub const fn from_mtbf(mtbf: u32) -> Self {
        Self(mtbf)
    }

    /// Return the config store's `MTBF` value.
    pub const fn mtbf(self) -> u32 {
        self.0
    }
}

/// Scope marker stored in a watermarked value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WatermarkScopeKind {
    Account,
    Machine,
}

/// Scope and seed used to encode or decode a watermarked value.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WatermarkScope {
    Account(AccountWatermarkSeed),
    Machine(MachineWatermarkSeed),
}

impl WatermarkScope {
    /// Return the marker kind associated with this seeded scope.
    pub const fn kind(self) -> WatermarkScopeKind {
        match self {
            Self::Account(_) => WatermarkScopeKind::Account,
            Self::Machine(_) => WatermarkScopeKind::Machine,
        }
    }

    const fn marker(self) -> u32 {
        match self {
            Self::Account(_) => ACCOUNT_SCOPE_MARKER,
            Self::Machine(_) => MACHINE_SCOPE_MARKER,
        }
    }

    const fn seed(self) -> u32 {
        match self {
            Self::Account(seed) => seed.account_id(),
            Self::Machine(seed) => seed.mtbf(),
        }
    }
}

/// Decoded config-store value and the scope recorded in its frame.
///
/// The payload can contain arbitrary binary data.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DecodedWatermark {
    /// Scope marker read from the frame.
    pub scope: WatermarkScopeKind,
    /// Decoded binary payload.
    pub payload: Vec<u8>,
}

/// Failure while inspecting, encoding, or decoding a watermarked value.
#[derive(Debug, Error, Eq, PartialEq)]
#[non_exhaustive]
pub enum WatermarkError {
    #[error("watermarked value is too short: got {actual} bytes, need at least {minimum}")]
    InputTooShort { actual: usize, minimum: usize },

    #[error("watermarked payload is too large: got {actual} bytes, maximum is {maximum}")]
    PayloadTooLarge { actual: usize, maximum: usize },

    #[error("unknown watermark scope marker {marker}")]
    UnknownScope { marker: u32 },

    #[error(
        "watermark scope mismatch: encoded value is {encoded:?}, supplied scope is {provided:?}"
    )]
    ScopeMismatch {
        encoded: WatermarkScopeKind,
        provided: WatermarkScopeKind,
    },

    #[error("watermark checksum mismatch: expected {expected:#010x}, got {actual:#010x}")]
    ChecksumMismatch { expected: u32, actual: u32 },

    #[error("hex input has odd length {length}")]
    OddHexLength { length: usize },

    #[error("invalid hex digit at byte {index}: {value:#04x}")]
    InvalidHexDigit { index: usize, value: u8 },
}

/// Read the scope marker without validating or decoding the payload.
pub fn encoded_scope(encoded: &[u8]) -> Result<WatermarkScopeKind, WatermarkError> {
    const MARKER_LENGTH: usize = 4;

    if encoded.len() < MARKER_LENGTH {
        return Err(WatermarkError::InputTooShort {
            actual: encoded.len(),
            minimum: MARKER_LENGTH,
        });
    }

    let marker = u32::from_le_bytes(encoded[..MARKER_LENGTH].try_into().expect("checked length"));
    scope_from_marker(marker)
}

/// Read the scope marker from a hex-encoded config value.
pub fn encoded_scope_hex(encoded: &str) -> Result<WatermarkScopeKind, WatermarkError> {
    const MARKER_HEX_LENGTH: usize = 8;

    if !encoded.len().is_multiple_of(2) {
        return Err(WatermarkError::OddHexLength {
            length: encoded.len(),
        });
    }
    if encoded.len() < MARKER_HEX_LENGTH {
        return Err(WatermarkError::InputTooShort {
            actual: encoded.len() / 2,
            minimum: MARKER_HEX_LENGTH / 2,
        });
    }

    let mut marker = [0; 4];
    for (index, byte) in marker.iter_mut().enumerate() {
        *byte = decode_hex_byte(encoded.as_bytes(), index)?;
    }

    scope_from_marker(u32::from_le_bytes(marker))
}

/// Encode an arbitrary payload using Steam's config-store watermark format.
pub fn encode(payload: &[u8], scope: WatermarkScope) -> Result<Vec<u8>, WatermarkError> {
    let mut encoded = Vec::with_capacity(payload.len() + FRAME_OVERHEAD);
    encode_with(payload, scope, |bytes| encoded.extend_from_slice(bytes))?;
    Ok(encoded)
}

/// Encode a payload as the hexadecimal text stored in `config.vdf`.
pub fn encode_hex(payload: &[u8], scope: WatermarkScope) -> Result<String, WatermarkError> {
    let mut encoded = String::with_capacity((payload.len() + FRAME_OVERHEAD) * 2);
    encode_with(payload, scope, |bytes| append_hex(&mut encoded, bytes))?;
    Ok(encoded)
}

/// Decode an arbitrary binary config-store watermark frame.
pub fn decode(encoded: &[u8], scope: WatermarkScope) -> Result<DecodedWatermark, WatermarkError> {
    validate_frame_length(encoded.len())?;
    let encoded_scope = encoded_scope(encoded)?;
    if encoded_scope != scope.kind() {
        return Err(WatermarkError::ScopeMismatch {
            encoded: encoded_scope,
            provided: scope.kind(),
        });
    }

    decode_owned(encoded.to_vec(), scope)
}

/// Decode an owned binary frame without allocating a second payload buffer.
pub fn decode_owned(
    mut encoded: Vec<u8>,
    scope: WatermarkScope,
) -> Result<DecodedWatermark, WatermarkError> {
    let payload_length = validate_frame_length(encoded.len())?;
    let encoded_scope = encoded_scope(&encoded)?;
    if encoded_scope != scope.kind() {
        return Err(WatermarkError::ScopeMismatch {
            encoded: encoded_scope,
            provided: scope.kind(),
        });
    }

    let payload_end = 4 + payload_length;
    let complete_words_end = 4 + payload_length / 4 * 4;
    let mut state = WatermarkState::new(scope.seed());
    let mut checksum = 0u32;
    for offset in (4..complete_words_end).step_by(4) {
        let word = u32::from_le_bytes(
            encoded[offset..offset + 4]
                .try_into()
                .expect("four-byte chunk"),
        ) ^ state.next_mask();
        checksum = checksum.wrapping_add(word);
        encoded[offset..offset + 4].copy_from_slice(&word.to_le_bytes());
    }

    let expected = checksum ^ state.next_mask();
    let actual = u32::from_le_bytes(
        encoded[payload_end..]
            .try_into()
            .expect("four-byte checksum"),
    );
    if actual != expected {
        return Err(WatermarkError::ChecksumMismatch { expected, actual });
    }

    encoded.copy_within(4..payload_end, 0);
    encoded.truncate(payload_length);
    Ok(DecodedWatermark {
        scope: encoded_scope,
        payload: encoded,
    })
}

/// Decode a hexadecimal value read directly from `config.vdf`.
pub fn decode_hex(
    encoded: &str,
    scope: WatermarkScope,
) -> Result<DecodedWatermark, WatermarkError> {
    if !encoded.len().is_multiple_of(2) {
        return Err(WatermarkError::OddHexLength {
            length: encoded.len(),
        });
    }

    validate_frame_length(encoded.len() / 2)?;
    decode_owned(decode_hex_bytes(encoded.as_bytes())?, scope)
}

fn encode_with(
    payload: &[u8],
    scope: WatermarkScope,
    mut append: impl FnMut(&[u8]),
) -> Result<(), WatermarkError> {
    validate_payload_size(payload.len())?;
    append(&scope.marker().to_le_bytes());

    let mut state = WatermarkState::new(scope.seed());
    let mut checksum = 0u32;
    let mut words = payload.chunks_exact(4);
    for chunk in &mut words {
        let word = u32::from_le_bytes(chunk.try_into().expect("four-byte chunk"));
        checksum = checksum.wrapping_add(word);
        append(&(word ^ state.next_mask()).to_le_bytes());
    }

    append(words.remainder());
    append(&(checksum ^ state.next_mask()).to_le_bytes());
    Ok(())
}

fn append_hex(encoded: &mut String, bytes: &[u8]) {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";

    for byte in bytes {
        encoded.push(DIGITS[(byte >> 4) as usize] as char);
        encoded.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
}

fn validate_frame_length(length: usize) -> Result<usize, WatermarkError> {
    if length < FRAME_OVERHEAD {
        return Err(WatermarkError::InputTooShort {
            actual: length,
            minimum: FRAME_OVERHEAD,
        });
    }

    let payload_length = length - FRAME_OVERHEAD;
    validate_payload_size(payload_length)?;
    Ok(payload_length)
}

fn validate_payload_size(length: usize) -> Result<(), WatermarkError> {
    if length > MAX_PAYLOAD_SIZE {
        return Err(WatermarkError::PayloadTooLarge {
            actual: length,
            maximum: MAX_PAYLOAD_SIZE,
        });
    }

    Ok(())
}

fn scope_from_marker(marker: u32) -> Result<WatermarkScopeKind, WatermarkError> {
    match marker {
        ACCOUNT_SCOPE_MARKER => Ok(WatermarkScopeKind::Account),
        MACHINE_SCOPE_MARKER => Ok(WatermarkScopeKind::Machine),
        marker => Err(WatermarkError::UnknownScope { marker }),
    }
}

fn decode_hex_bytes(encoded: &[u8]) -> Result<Vec<u8>, WatermarkError> {
    if !encoded.len().is_multiple_of(2) {
        return Err(WatermarkError::OddHexLength {
            length: encoded.len(),
        });
    }

    let mut decoded = Vec::with_capacity(encoded.len() / 2);
    for index in 0..encoded.len() / 2 {
        decoded.push(decode_hex_byte(encoded, index)?);
    }

    Ok(decoded)
}

fn decode_hex_byte(encoded: &[u8], index: usize) -> Result<u8, WatermarkError> {
    let offset = index * 2;
    let high = hex_nibble(encoded[offset], offset)?;
    let low = hex_nibble(encoded[offset + 1], offset + 1)?;
    Ok(high << 4 | low)
}

fn hex_nibble(value: u8, index: usize) -> Result<u8, WatermarkError> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'a'..=b'f' => Ok(value - b'a' + 10),
        b'A'..=b'F' => Ok(value - b'A' + 10),
        value => Err(WatermarkError::InvalidHexDigit { index, value }),
    }
}

struct WatermarkState {
    mask: u32,
    sequence: u32,
}

impl WatermarkState {
    fn new(seed: u32) -> Self {
        Self {
            mask: 0,
            sequence: seed,
        }
    }

    fn next_mask(&mut self) -> u32 {
        self.mask = self.mask.wrapping_add(WATERMARK_STEP);
        self.sequence = self.sequence.wrapping_add(1);
        self.mask ^ WATERMARK_TABLE[(self.sequence % WATERMARK_TABLE.len() as u32) as usize]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ACCOUNT_VECTOR: &str = "01000000b7d70dc8673c0b14c6a5b925ea8cf0dcf71d54e0741903df74";
    const MACHINE_VECTOR: &str = "02000000a2c4d301c9bbd5730000011f2549";

    #[test]
    fn decodes_known_account_scope_value() {
        let decoded = decode_hex(
            ACCOUNT_VECTOR,
            WatermarkScope::Account(AccountWatermarkSeed::from_account_id(123456789)),
        )
        .unwrap();

        assert_eq!(decoded.scope, WatermarkScopeKind::Account);
        assert_eq!(decoded.payload, b"account-scoped secret");
    }

    #[test]
    fn decodes_known_machine_scope_value() {
        let decoded = decode_hex(
            MACHINE_VECTOR,
            WatermarkScope::Machine(MachineWatermarkSeed::from_mtbf(1082155420)),
        )
        .unwrap();

        assert_eq!(decoded.scope, WatermarkScopeKind::Machine);
        assert_eq!(decoded.payload, b"US\0\0\0\0\0\0\0\0");
    }

    #[test]
    fn encoding_matches_known_account_scope_value() {
        let encoded = encode_hex(
            b"account-scoped secret",
            WatermarkScope::Account(AccountWatermarkSeed::from_account_id(123456789)),
        )
        .unwrap();

        assert_eq!(encoded, ACCOUNT_VECTOR);
    }

    #[test]
    fn encoding_matches_known_machine_scope_value() {
        let encoded = encode_hex(
            b"US\0\0\0\0\0\0\0\0",
            WatermarkScope::Machine(MachineWatermarkSeed::from_mtbf(1082155420)),
        )
        .unwrap();

        assert_eq!(encoded, MACHINE_VECTOR);
    }

    #[test]
    fn round_trips_word_boundaries_in_both_scopes() {
        let scopes = [
            WatermarkScope::Account(AccountWatermarkSeed::from_account_id(123456789)),
            WatermarkScope::Machine(MachineWatermarkSeed::from_mtbf(1082155420)),
        ];

        for scope in scopes {
            for payload in [
                &b""[..],
                &b"a"[..],
                &b"ab"[..],
                &b"abc"[..],
                &b"abcd"[..],
                &b"abcde"[..],
            ] {
                let encoded = encode(payload, scope).unwrap();
                let decoded = decode(&encoded, scope).unwrap();
                assert_eq!(decoded.payload, payload);
                assert_eq!(decoded.scope, scope.kind());
            }
        }
    }

    #[test]
    fn reports_scope_and_checksum_errors() {
        let account_scope =
            WatermarkScope::Account(AccountWatermarkSeed::from_account_id(123456789));
        let machine_scope = WatermarkScope::Machine(MachineWatermarkSeed::from_mtbf(1082155420));
        let encoded = steamroom::util::hex::decode(ACCOUNT_VECTOR).unwrap();

        assert_eq!(
            decode(&encoded, machine_scope),
            Err(WatermarkError::ScopeMismatch {
                encoded: WatermarkScopeKind::Account,
                provided: WatermarkScopeKind::Machine,
            })
        );

        assert!(matches!(
            decode(
                &encoded,
                WatermarkScope::Account(AccountWatermarkSeed::from_account_id(123456790)),
            ),
            Err(WatermarkError::ChecksumMismatch { .. })
        ));

        let mut corrupted = encoded;
        *corrupted.last_mut().unwrap() ^= 1;
        assert!(matches!(
            decode(&corrupted, account_scope),
            Err(WatermarkError::ChecksumMismatch { .. })
        ));
    }

    #[test]
    fn reports_literal_hex_errors() {
        assert_eq!(
            decode_hex(
                "010",
                WatermarkScope::Account(AccountWatermarkSeed::from_account_id(1)),
            ),
            Err(WatermarkError::OddHexLength { length: 3 })
        );
        assert_eq!(
            encoded_scope_hex("0100000z"),
            Err(WatermarkError::InvalidHexDigit {
                index: 7,
                value: b'z',
            })
        );
        assert_eq!(
            encoded_scope(&0xfeed_beefu32.to_le_bytes()),
            Err(WatermarkError::UnknownScope {
                marker: 0xfeed_beef,
            })
        );
    }

    #[test]
    fn reports_short_and_oversized_frames() {
        let scope = WatermarkScope::Machine(MachineWatermarkSeed::from_mtbf(1));
        assert_eq!(
            decode(&[0; FRAME_OVERHEAD - 1], scope),
            Err(WatermarkError::InputTooShort {
                actual: FRAME_OVERHEAD - 1,
                minimum: FRAME_OVERHEAD,
            })
        );

        let mut oversized = vec![0; MAX_PAYLOAD_SIZE + FRAME_OVERHEAD + 1];
        oversized[..4].copy_from_slice(&MACHINE_SCOPE_MARKER.to_le_bytes());
        assert_eq!(
            decode(&oversized, scope),
            Err(WatermarkError::PayloadTooLarge {
                actual: MAX_PAYLOAD_SIZE + 1,
                maximum: MAX_PAYLOAD_SIZE,
            })
        );
    }

    #[test]
    fn enforces_steam_payload_limit() {
        let scope = WatermarkScope::Machine(MachineWatermarkSeed::from_mtbf(1));
        assert!(encode(&vec![0; MAX_PAYLOAD_SIZE], scope).is_ok());
        assert_eq!(
            encode(&vec![0; MAX_PAYLOAD_SIZE + 1], scope),
            Err(WatermarkError::PayloadTooLarge {
                actual: MAX_PAYLOAD_SIZE + 1,
                maximum: MAX_PAYLOAD_SIZE,
            })
        );
    }
}
