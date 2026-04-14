//! Extract cached credentials from a local Steam installation.
//!
//! On Windows, refresh tokens are stored in `%LOCALAPPDATA%\Steam\local.vdf`
//! as DPAPI-encrypted blobs keyed by a hash, with the account name as entropy.
//!
//! This module provides a platform-agnostic [`SteamCredentials`] trait and
//! a platform-specific implementation via [`detect()`].

use std::path::PathBuf;

/// A cached Steam credential extracted from the local Steam installation.
#[derive(Clone, Debug)]
pub struct CachedToken {
    pub account_name: String,
    pub refresh_token: String,
}

/// Find the Steam installation directory.
pub fn steam_dir() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        windows::steam_dir()
    }
    #[cfg(target_os = "linux")]
    {
        let home = dirs_next::home_dir()?;
        for candidate in [home.join(".steam/steam"), home.join(".local/share/Steam")] {
            if candidate.join("config").join("loginusers.vdf").exists() {
                return Some(candidate);
            }
        }
        None
    }
    #[cfg(target_os = "macos")]
    {
        let home = dirs_next::home_dir()?;
        let candidate = home.join("Library/Application Support/Steam");
        if candidate.join("config").join("loginusers.vdf").exists() {
            return Some(candidate);
        }
        None
    }
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    {
        None
    }
}

/// A locally known Steam user from `loginusers.vdf`.
#[derive(Clone, Debug)]
pub struct LocalUser {
    pub steam_id: String,
    pub account_name: String,
    pub persona_name: String,
    pub most_recent: bool,
}

/// List all Steam users from `loginusers.vdf`.
pub fn list_users(steam_dir: &std::path::Path) -> Vec<LocalUser> {
    let Ok(content) = std::fs::read_to_string(steam_dir.join("config").join("loginusers.vdf"))
    else {
        return vec![];
    };
    let Ok(kv) = steamroom::types::key_value::parse_text_kv(&content) else {
        return vec![];
    };
    let mut users = Vec::new();
    if let steamroom::types::key_value::KvValue::Children(ref map) = kv.value {
        for (steam_id, user_kv) in map {
            let account_name = user_kv
                .get("AccountName")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let persona_name = user_kv
                .get("PersonaName")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let most_recent = user_kv.get("mostrecent").and_then(|v| v.as_str()) == Some("1");
            users.push(LocalUser {
                steam_id: steam_id.clone(),
                account_name,
                persona_name,
                most_recent,
            });
        }
    }
    users
}

/// Detect the most recently logged-in Steam username from `loginusers.vdf`.
pub fn detect_username(steam_dir: &std::path::Path) -> Option<String> {
    let login_users =
        std::fs::read_to_string(steam_dir.join("config").join("loginusers.vdf")).ok()?;
    let kv = steamroom::types::key_value::parse_text_kv(&login_users).ok()?;
    if let steamroom::types::key_value::KvValue::Children(ref users) = kv.value {
        for user_kv in users.values() {
            let most_recent = user_kv
                .get("mostrecent")
                .and_then(|v| v.as_str())
                .unwrap_or("0");
            if most_recent == "1" {
                return user_kv
                    .get("AccountName")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
            }
        }
    }
    None
}

/// Read the hex-encoded encrypted token blob from a `local.vdf` file.
/// Traverses `MachineUserConfigStore/Software/valve/Steam/ConnectCache`.
fn read_connect_cache_blob(path: &std::path::Path) -> Option<String> {
    let content = std::fs::read_to_string(path).ok()?;
    let kv = steamroom::types::key_value::parse_text_kv(&content).ok()?;
    let software = kv.get("Software")?;
    let valve = software.get("valve").or_else(|| software.get("Valve"))?;
    let steam = valve.get("Steam")?;
    let cache = steam.get("ConnectCache")?;
    if let steamroom::types::key_value::KvValue::Children(ref entries) = cache.value {
        for entry in entries.values() {
            if let Some(hex) = entry.as_str() {
                return Some(hex.to_string());
            }
        }
    }
    None
}

/// A depot decryption key cached in Steam's `config.vdf`.
#[derive(Clone, Debug)]
pub struct LocalDepotKey {
    pub depot_id: steamroom::depot::DepotId,
    pub key: steamroom::depot::DepotKey,
}

/// Locally installed app metadata from appmanifest ACF files.
#[derive(Clone, Debug)]
pub struct LocalApp {
    pub app_id: u32,
    pub name: String,
    pub depot_ids: Vec<u32>,
}

/// A beta branch password hash stored in Steam's `config.vdf`.
/// The hash is a 32-byte server-issued token. The first 4 bytes are a version
/// marker. Only the first 32 bytes of the hex-decoded configstore value are used.
#[derive(Clone, Debug)]
pub struct LocalBetaHash {
    pub app_id: u32,
    pub branch: String,
    pub hash: [u8; 32],
}

/// Information extracted from Steam's local `config.vdf`.
#[derive(Clone, Debug, Default)]
pub struct LocalConfig {
    pub depot_keys: Vec<LocalDepotKey>,
    pub beta_hashes: Vec<LocalBetaHash>,
}

/// Parse Steam's `config.vdf` and extract cached depot keys and beta hashes.
pub fn read_config(steam_dir: &std::path::Path) -> Option<LocalConfig> {
    let path = steam_dir.join("config").join("config.vdf");
    let content = std::fs::read_to_string(&path).ok()?;
    let kv = steamroom::types::key_value::parse_text_kv(&content).ok()?;
    let software = kv.get("Software").or_else(|| kv.get("software"))?;
    let valve = software.get("valve").or_else(|| software.get("Valve"))?;
    let steam = valve.get("Steam").or_else(|| valve.get("steam"))?;

    let mut config = LocalConfig::default();

    if let Some(depots) = steam.get("depots")
        && let steamroom::types::key_value::KvValue::Children(ref map) = depots.value
    {
        for (id_str, depot_kv) in map {
            let Ok(id) = id_str.parse::<u32>() else {
                continue;
            };
            if let Some(hex) = depot_kv.get("DecryptionKey").and_then(|v| v.as_str())
                && let Some(bytes) = steamroom::util::hex::decode(hex)
                && bytes.len() == 32
            {
                let mut key = [0u8; 32];
                key.copy_from_slice(&bytes);
                config.depot_keys.push(LocalDepotKey {
                    depot_id: steamroom::depot::DepotId(id),
                    key: steamroom::depot::DepotKey(key),
                });
            }
        }
    }

    if let Some(apps) = steam.get("apps")
        && let steamroom::types::key_value::KvValue::Children(ref map) = apps.value
    {
        for (id_str, app_kv) in map {
            let Ok(app_id) = id_str.parse::<u32>() else {
                continue;
            };
            if let steamroom::types::key_value::KvValue::Children(ref fields) = app_kv.value {
                for (key, val) in fields {
                    if let Some(branch) = key.strip_prefix("betahash_")
                        && let Some(hex_str) = val.as_str()
                    {
                        // First 64 hex chars = 32 bytes of hash
                        let decode_str = &hex_str[..hex_str.len().min(64)];
                        if let Some(bytes) = steamroom::util::hex::decode(decode_str)
                            && bytes.len() == 32
                        {
                            let mut hash = [0u8; 32];
                            hash.copy_from_slice(&bytes);
                            config.beta_hashes.push(LocalBetaHash {
                                app_id,
                                branch: branch.to_string(),
                                hash,
                            });
                        }
                    }
                }
            }
        }
    }

    Some(config)
}

/// Look up a depot decryption key from a parsed local config.
pub fn find_depot_key(
    config: &LocalConfig,
    depot_id: steamroom::depot::DepotId,
) -> Option<steamroom::depot::DepotKey> {
    config
        .depot_keys
        .iter()
        .find(|k| k.depot_id == depot_id)
        .map(|k| k.key.clone())
}

/// Scan all Steam library folders for installed app manifests.
/// Returns app metadata including name and installed depot IDs.
pub fn scan_installed_apps(steam_dir: &std::path::Path) -> Vec<LocalApp> {
    let mut apps = Vec::new();
    let library_folders = list_library_folders(steam_dir);
    for folder in &library_folders {
        let steamapps = folder.join("steamapps");
        let Ok(entries) = std::fs::read_dir(&steamapps) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            if !name_str.starts_with("appmanifest_") || !name_str.ends_with(".acf") {
                continue;
            }
            if let Some(app) = parse_app_manifest(&entry.path()) {
                apps.push(app);
            }
        }
    }
    apps
}

fn list_library_folders(steam_dir: &std::path::Path) -> Vec<PathBuf> {
    let mut folders = vec![steam_dir.to_path_buf()];
    let vdf_path = steam_dir.join("steamapps").join("libraryfolders.vdf");
    let Ok(content) = std::fs::read_to_string(&vdf_path) else {
        return folders;
    };
    let Ok(kv) = steamroom::types::key_value::parse_text_kv(&content) else {
        return folders;
    };
    if let steamroom::types::key_value::KvValue::Children(ref map) = kv.value {
        for entry in map.values() {
            if let Some(path) = entry.get("path").and_then(|v| v.as_str()) {
                let p = PathBuf::from(path);
                if p != steam_dir {
                    folders.push(p);
                }
            }
        }
    }
    folders
}

fn parse_app_manifest(path: &std::path::Path) -> Option<LocalApp> {
    let content = std::fs::read_to_string(path).ok()?;
    let kv = steamroom::types::key_value::parse_text_kv(&content).ok()?;
    let app_id: u32 = kv.get("appid")?.as_str()?.parse().ok()?;
    let name = kv
        .get("name")
        .and_then(|v| v.as_str())
        .unwrap_or("(unknown)")
        .to_string();

    let mut depot_ids = Vec::new();
    if let Some(depots) = kv.get("InstalledDepots")
        && let steamroom::types::key_value::KvValue::Children(ref map) = depots.value
    {
        for id_str in map.keys() {
            if let Ok(id) = id_str.parse::<u32>() {
                depot_ids.push(id);
            }
        }
    }

    Some(LocalApp {
        app_id,
        name,
        depot_ids,
    })
}

/// Look up a cached beta branch hash for a given app and branch.
pub fn find_beta_hash(config: &LocalConfig, app_id: u32, branch: &str) -> Option<[u8; 32]> {
    config
        .beta_hashes
        .iter()
        .find(|bh| bh.app_id == app_id && bh.branch == branch)
        .map(|bh| bh.hash)
}

/// Extract the cached refresh token for the given account name.
///
/// Returns `None` if Steam is not installed, no token is cached, or
/// decryption fails (e.g. on a platform that doesn't support it yet).
pub fn extract_token(account_name: &str) -> Option<CachedToken> {
    #[cfg(windows)]
    {
        windows::extract_token(account_name)
    }
    #[cfg(any(target_os = "linux", target_os = "macos"))]
    {
        unix::extract_token(account_name)
    }
    #[cfg(not(any(windows, target_os = "linux", target_os = "macos")))]
    {
        let _ = account_name;
        None
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
mod unix {
    use super::*;
    use std::path::Path;

    pub fn extract_token(account_name: &str) -> Option<CachedToken> {
        let steam_dir = super::steam_dir()?;
        let local_vdf = find_local_vdf(&steam_dir)?;
        let encrypted_hex = super::read_connect_cache_blob(&local_vdf)?;
        let encrypted = steamroom::util::hex::decode(&encrypted_hex)?;
        let decrypted = decrypt_token_blob(&encrypted, account_name)?;
        let token = String::from_utf8(decrypted).ok()?;
        if !token.starts_with("eyA") {
            return None;
        }
        Some(CachedToken {
            account_name: account_name.to_string(),
            refresh_token: token,
        })
    }

    fn find_local_vdf(steam_dir: &Path) -> Option<PathBuf> {
        let path = steam_dir.join("local.vdf");
        if path.exists() {
            return Some(path);
        }
        let path = steam_dir.join("config").join("local.vdf");
        if path.exists() {
            return Some(path);
        }
        None
    }

    /// Decrypt token blob: key = SHA256(account_name), format = ECB(IV) || CBC(data).
    fn decrypt_token_blob(data: &[u8], account_name: &str) -> Option<Vec<u8>> {
        use sha2::Digest;

        if data.len() < 32 {
            return None;
        }

        let key = sha2::Sha256::digest(account_name.as_bytes());
        let iv = steamroom::crypto::symmetric_decrypt_ecb_nopad(&data[..16], &key).ok()?;
        steamroom::crypto::symmetric_decrypt_cbc(&data[16..], &key, &iv).ok()
    }
}

#[cfg(windows)]
mod windows {
    use super::*;
    use std::path::Path;

    pub fn steam_dir() -> Option<PathBuf> {
        let path = reg_query_string(r"Software\Valve\Steam", "SteamPath")?;
        Some(PathBuf::from(path))
    }

    fn reg_query_string(subkey: &str, value_name: &str) -> Option<String> {
        use std::ptr;

        #[link(name = "advapi32")]
        unsafe extern "system" {
            fn RegOpenKeyExA(
                hkey: isize,
                lp_sub_key: *const u8,
                ul_options: u32,
                sam_desired: u32,
                phk_result: *mut isize,
            ) -> i32;
            fn RegQueryValueExA(
                hkey: isize,
                lp_value_name: *const u8,
                lp_reserved: *mut u32,
                lp_type: *mut u32,
                lp_data: *mut u8,
                lpcb_data: *mut u32,
            ) -> i32;
            fn RegCloseKey(hkey: isize) -> i32;
        }

        use std::ffi::CString;

        const HKEY_CURRENT_USER: isize = 0x80000001u32 as isize;
        const KEY_READ: u32 = 0x20019;
        const ERROR_SUCCESS: i32 = 0;

        let subkey_cstr = CString::new(subkey).ok()?;
        let value_cstr = CString::new(value_name).ok()?;

        let mut hkey: isize = 0;
        let result = unsafe {
            RegOpenKeyExA(
                HKEY_CURRENT_USER,
                subkey_cstr.as_ptr().cast(),
                0,
                KEY_READ,
                &mut hkey,
            )
        };
        if result != ERROR_SUCCESS {
            return None;
        }

        let mut buf = vec![0u8; 1024];
        let mut buf_len = buf.len() as u32;
        let mut reg_type: u32 = 0;

        let result = unsafe {
            RegQueryValueExA(
                hkey,
                value_cstr.as_ptr().cast(),
                ptr::null_mut(),
                &mut reg_type,
                buf.as_mut_ptr(),
                &mut buf_len,
            )
        };
        unsafe { RegCloseKey(hkey) };

        if result != ERROR_SUCCESS || buf_len == 0 {
            return None;
        }

        // Strip null terminator
        let len = buf_len as usize;
        let end = if len > 0 && buf[len - 1] == 0 {
            len - 1
        } else {
            len
        };
        String::from_utf8(buf[..end].to_vec()).ok()
    }

    pub fn extract_token(account_name: &str) -> Option<CachedToken> {
        let local_vdf_path = local_vdf_path()?;
        let encrypted_hex = super::read_connect_cache_blob(&local_vdf_path)?;
        let encrypted = steamroom::util::hex::decode(&encrypted_hex)?;
        let decrypted = dpapi_decrypt(&encrypted, account_name.as_bytes())?;
        let token = String::from_utf8(decrypted).ok()?;
        if !token.starts_with("eyA") {
            return None;
        }
        Some(CachedToken {
            account_name: account_name.to_string(),
            refresh_token: token,
        })
    }

    fn local_vdf_path() -> Option<PathBuf> {
        let local_app_data = std::env::var("LOCALAPPDATA").ok()?;
        let path = Path::new(&local_app_data).join("Steam").join("local.vdf");
        if path.exists() { Some(path) } else { None }
    }

    fn dpapi_decrypt(encrypted: &[u8], entropy: &[u8]) -> Option<Vec<u8>> {
        use std::ffi::c_void;
        use std::ptr;

        #[repr(C)]
        struct DataBlob {
            cb_data: u32,
            pb_data: *mut u8,
        }

        #[link(name = "crypt32")]
        unsafe extern "system" {
            fn CryptUnprotectData(
                p_data_in: *const DataBlob,
                pp_sz_data_descr: *mut *mut u16,
                p_optional_entropy: *const DataBlob,
                pv_reserved: *mut c_void,
                p_prompt_struct: *mut c_void,
                dw_flags: u32,
                p_data_out: *mut DataBlob,
            ) -> i32;
        }

        #[link(name = "kernel32")]
        unsafe extern "system" {
            fn LocalFree(h_mem: *mut u8) -> *mut u8;
        }

        let input = DataBlob {
            cb_data: encrypted.len() as u32,
            pb_data: encrypted.as_ptr() as *mut u8,
        };
        let entropy_blob = DataBlob {
            cb_data: entropy.len() as u32,
            pb_data: entropy.as_ptr() as *mut u8,
        };
        let mut output = DataBlob {
            cb_data: 0,
            pb_data: ptr::null_mut(),
        };

        const CRYPTPROTECT_UI_FORBIDDEN: u32 = 0x1;

        let result = unsafe {
            CryptUnprotectData(
                &input,
                ptr::null_mut(),
                &entropy_blob,
                ptr::null_mut(),
                ptr::null_mut(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        };

        if result == 0 {
            return None;
        }

        let decrypted =
            unsafe { std::slice::from_raw_parts(output.pb_data, output.cb_data as usize) }.to_vec();

        unsafe {
            LocalFree(output.pb_data);
        }

        Some(decrypted)
    }
}
