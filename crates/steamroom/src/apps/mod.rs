/// Structured view over an app's PICS product info.
mod details;

pub use details::AppDetails;
pub use details::Branch;
pub use details::Depot;
pub use details::DepotConfig;
pub use details::DepotManifestInfo;

use crate::depot::AppId;
use crate::depot::PackageId;
use crate::types::key_value::KeyValue;
use crate::types::key_value::TextKvError;
use crate::types::key_value::parse_binary_kv;
use crate::types::key_value::parse_text_kv;

#[derive(Clone, Debug)]
pub struct AccessToken {
    pub app_id: AppId,
    pub token: u64,
}

#[derive(Clone, Debug)]
pub struct AppInfo {
    pub app_id: Option<AppId>,
    pub change_number: Option<u32>,
    pub kv_data: Option<Vec<u8>>,
}

#[derive(Clone, Debug)]
pub struct PackageInfo {
    pub package_id: Option<PackageId>,
    pub change_number: Option<u32>,
    pub kv_data: Option<Vec<u8>>,
}

#[derive(Clone, Debug)]
pub struct BetaBranch {
    pub name: Option<String>,
    pub password: Option<String>,
    pub description: Option<String>,
}

/// Failure decoding a PICS KV payload into a [`KeyValue`] tree.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum KvDecodeError {
    /// No KV payload was available (the app/package was unknown, or the
    /// response omitted the buffer).
    #[error("no KV payload available")]
    Missing,

    #[error("binary KV decode: {0}")]
    Binary(#[from] std::io::Error),

    #[error("text KV decode: {0}")]
    Text(#[from] TextKvError),
}

/// Decode a raw PICS KV buffer, auto-detecting the binary vs text encoding.
///
/// Binary KV blobs begin with a `0x00` subsection tag; anything else is
/// treated as UTF-8 text KV.
fn decode_kv(data: &[u8]) -> Result<KeyValue, KvDecodeError> {
    if data.first() == Some(&0x00) {
        Ok(parse_binary_kv(data)?)
    } else {
        Ok(parse_text_kv(&String::from_utf8_lossy(data))?)
    }
}

impl AppInfo {
    /// Decode this app's KV payload into a [`KeyValue`] tree.
    pub fn key_values(&self) -> Result<KeyValue, KvDecodeError> {
        let data = self.kv_data.as_deref().ok_or(KvDecodeError::Missing)?;
        decode_kv(data)
    }

    /// Decode this app's KV payload into a structured [`AppDetails`].
    pub fn details(&self) -> Result<AppDetails, KvDecodeError> {
        let app_id = self.app_id.ok_or(KvDecodeError::Missing)?;
        Ok(AppDetails::from_key_values(app_id, self.key_values()?))
    }
}

impl PackageInfo {
    /// Decode this package's KV payload into a [`KeyValue`] tree.
    ///
    /// Package payloads carry a 4-byte header (package ID) ahead of the
    /// binary KV blob; it is stripped before decoding.
    pub fn key_values(&self) -> Result<KeyValue, KvDecodeError> {
        let data = self.kv_data.as_deref().ok_or(KvDecodeError::Missing)?;
        let kv_data = if data.len() > 4 && data[0] != 0x00 {
            &data[4..]
        } else {
            data
        };
        decode_kv(kv_data)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binary_kv() -> Vec<u8> {
        // tag=None "480" { tag=String "name" "Spacewar" } End
        let mut data = vec![0x00];
        data.extend_from_slice(b"480\0");
        data.push(0x01);
        data.extend_from_slice(b"name\0");
        data.extend_from_slice(b"Spacewar\0");
        data.push(0x08);
        data
    }

    #[test]
    fn app_key_values_detects_binary() {
        let info = AppInfo {
            app_id: Some(AppId(480)),
            change_number: None,
            kv_data: Some(binary_kv()),
        };
        let kv = info.key_values().unwrap();
        assert_eq!(kv.get("name").and_then(|n| n.as_str()), Some("Spacewar"));
    }

    #[test]
    fn app_key_values_detects_text() {
        let info = AppInfo {
            app_id: Some(AppId(480)),
            change_number: None,
            kv_data: Some(br#""480" { "name" "Spacewar" }"#.to_vec()),
        };
        let kv = info.key_values().unwrap();
        assert_eq!(kv.get("name").and_then(|n| n.as_str()), Some("Spacewar"));
    }

    #[test]
    fn app_key_values_missing_payload() {
        let info = AppInfo {
            app_id: Some(AppId(480)),
            change_number: None,
            kv_data: None,
        };
        assert!(matches!(info.key_values(), Err(KvDecodeError::Missing)));
    }

    #[test]
    fn package_key_values_strips_header() {
        // 4-byte package-id header ahead of the binary KV blob.
        let mut data = vec![0xEF, 0xBE, 0xAD, 0xDE];
        data.extend_from_slice(&binary_kv());
        let info = PackageInfo {
            package_id: Some(PackageId(1)),
            change_number: None,
            kv_data: Some(data),
        };
        let kv = info.key_values().unwrap();
        assert_eq!(kv.get("name").and_then(|n| n.as_str()), Some("Spacewar"));
    }
}
