//! Structured, typed view over an app's PICS product info KV tree.
//!
//! [`AppDetails::from_key_values`] walks the raw [`KeyValue`] once and lifts
//! the commonly-needed fields (name, type, depots, branches) into typed
//! values so callers do not re-implement KV traversal. The full tree is
//! retained as [`AppDetails::key_values`] for fields not modeled here.

use crate::depot::AppId;
use crate::depot::BuildId;
use crate::depot::DepotId;
use crate::depot::ManifestId;
use crate::types::key_value::KeyValue;
use crate::types::key_value::KvValue;

/// Typed summary of an app's PICS product info.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct AppDetails {
    pub app_id: AppId,
    /// `common/name`.
    pub name: Option<String>,
    /// `common/type` (e.g. "game", "dlc", "tool").
    pub app_type: Option<String>,
    pub depots: Vec<Depot>,
    pub branches: Vec<Branch>,
    /// The full KV tree, for fields not lifted into typed fields above.
    pub key_values: KeyValue,
}

/// A single depot within an app.
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Depot {
    pub id: DepotId,
    pub name: Option<String>,
    pub config: DepotConfig,
    /// Set when this depot is shared from another app (`depotfromapp`).
    pub depot_from_app: Option<AppId>,
    /// `sharedinstall` flag.
    pub shared_install: bool,
    /// Manifest for each branch this depot publishes.
    pub manifests: Vec<DepotManifestInfo>,
}

/// Platform/config constraints on a depot (`depots/<id>/config`).
#[derive(Clone, Debug, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct DepotConfig {
    /// Operating systems from `oslist`, e.g. `["windows", "macos"]`.
    pub os_list: Vec<String>,
    /// `osarch`, e.g. "64".
    pub os_arch: Option<String>,
    /// `language`.
    pub language: Option<String>,
    /// `lowviolence` flag.
    pub low_violence: bool,
}

/// The manifest a depot publishes on one branch
/// (`depots/<id>/manifests/<branch>`).
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct DepotManifestInfo {
    pub branch: String,
    pub manifest_id: ManifestId,
    /// Installed size in bytes (`size`).
    pub size: Option<u64>,
    /// Download size in bytes (`download`).
    pub download_size: Option<u64>,
}

/// A build branch (`depots/branches/<name>`).
#[derive(Clone, Debug)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Branch {
    pub name: String,
    pub build_id: Option<BuildId>,
    pub description: Option<String>,
    /// `pwdrequired` flag: a password is needed to access this branch.
    pub pwd_required: bool,
    /// `timeupdated`, epoch seconds.
    pub time_updated: Option<u64>,
    /// `timebuildupdated`, epoch seconds.
    pub time_built: Option<u64>,
}

impl AppDetails {
    /// Parse an app's root KV tree (the value keyed by its app ID) into a
    /// typed summary. Missing fields become `None`/empty rather than errors;
    /// malformed input degrades to the same.
    pub fn from_key_values(app_id: AppId, kv: KeyValue) -> Self {
        let name = kv_str(kv.get("common").and_then(|c| c.get("name")));
        let app_type = kv_str(kv.get("common").and_then(|c| c.get("type")));

        let depots_kv = kv.get("depots");
        let depots = depots_kv.map(parse_depots).unwrap_or_default();
        let branches = depots_kv
            .and_then(|d| d.get("branches"))
            .map(parse_branches)
            .unwrap_or_default();

        Self {
            app_id,
            name,
            app_type,
            depots,
            branches,
            key_values: kv,
        }
    }

    /// The depot with the given ID, if present.
    pub fn depot(&self, id: DepotId) -> Option<&Depot> {
        self.depots.iter().find(|d| d.id == id)
    }

    /// The valid depot with the lowest ID, if any. Useful as a default when
    /// a caller does not specify one.
    pub fn first_depot(&self) -> Option<&Depot> {
        self.depots
            .iter()
            .filter(|d| d.id != DepotId::INVALID)
            .min_by_key(|d| d.id.0)
    }

    /// The branch with the given name, if present.
    pub fn branch(&self, name: &str) -> Option<&Branch> {
        self.branches.iter().find(|b| b.name == name)
    }
}

impl Depot {
    /// Whether this depot is a redistributable shared from another app.
    pub fn is_shared(&self) -> bool {
        self.depot_from_app.is_some() || self.shared_install
    }

    /// The manifest this depot publishes on `branch`, if any.
    pub fn manifest(&self, branch: &str) -> Option<&DepotManifestInfo> {
        self.manifests.iter().find(|m| m.branch == branch)
    }
}

fn parse_depots(depots_kv: &KeyValue) -> Vec<Depot> {
    let KvValue::Children(ref map) = depots_kv.value else {
        return Vec::new();
    };
    map.iter()
        .filter_map(|(key, depot)| {
            let id = DepotId(key.parse().ok()?);
            Some(parse_depot(id, depot))
        })
        .collect()
}

fn parse_depot(id: DepotId, depot: &KeyValue) -> Depot {
    let config = depot.get("config").map(parse_depot_config).unwrap_or_default();
    let depot_from_app = kv_u64(depot.get("depotfromapp")).map(|v| AppId(v as u32));
    let shared_install = depot.get("sharedinstall").is_some();
    let manifests = depot
        .get("manifests")
        .map(parse_depot_manifests)
        .unwrap_or_default();

    Depot {
        id,
        name: kv_str(depot.get("name")),
        config,
        depot_from_app,
        shared_install,
        manifests,
    }
}

fn parse_depot_config(config: &KeyValue) -> DepotConfig {
    let os_list = kv_str(config.get("oslist"))
        .map(|s| {
            s.split(',')
                .map(|o| o.trim().to_string())
                .filter(|o| !o.is_empty())
                .collect()
        })
        .unwrap_or_default();
    DepotConfig {
        os_list,
        os_arch: kv_str(config.get("osarch")),
        language: kv_str(config.get("language")),
        low_violence: kv_str(config.get("lowviolence")).as_deref() == Some("1"),
    }
}

fn parse_depot_manifests(manifests: &KeyValue) -> Vec<DepotManifestInfo> {
    let KvValue::Children(ref map) = manifests.value else {
        return Vec::new();
    };
    map.iter()
        .filter_map(|(branch, entry)| {
            // A branch entry is either a subsection with `gid`/`size`/
            // `download`, or a bare string that is the manifest ID.
            let manifest_id = ManifestId(
                kv_u64(entry.get("gid"))
                    .or_else(|| entry.as_str().and_then(|s| s.parse().ok()))?,
            );
            Some(DepotManifestInfo {
                branch: branch.clone(),
                manifest_id,
                size: kv_u64(entry.get("size")),
                download_size: kv_u64(entry.get("download")),
            })
        })
        .collect()
}

fn parse_branches(branches: &KeyValue) -> Vec<Branch> {
    let KvValue::Children(ref map) = branches.value else {
        return Vec::new();
    };
    map.iter()
        .map(|(name, branch)| Branch {
            name: name.clone(),
            build_id: kv_u64(branch.get("buildid")).map(|v| BuildId(v as u32)),
            description: kv_str(branch.get("description")),
            pwd_required: kv_str(branch.get("pwdrequired")).as_deref() == Some("1"),
            time_updated: kv_u64(branch.get("timeupdated")),
            time_built: kv_u64(branch.get("timebuildupdated")),
        })
        .collect()
}

/// Read a KV node as a string, coercing integer scalars to their decimal
/// form (PICS mixes string and int encodings for the same fields).
fn kv_str(kv: Option<&KeyValue>) -> Option<String> {
    let kv = kv?;
    kv.as_str()
        .map(str::to_owned)
        .or_else(|| kv.as_i32().map(|i| i.to_string()))
        .or_else(|| kv.as_u64().map(|i| i.to_string()))
}

fn kv_u64(kv: Option<&KeyValue>) -> Option<u64> {
    let kv = kv?;
    kv.as_u64()
        .or_else(|| kv.as_i32().and_then(|i| u64::try_from(i).ok()))
        .or_else(|| kv.as_str().and_then(|s| s.parse().ok()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::key_value::parse_text_kv;

    const APP_KV: &str = r#"
"480"
{
    "common" { "name" "Spacewar" "type" "game" }
    "depots"
    {
        "branches"
        {
            "public" { "buildid" "1234" "timeupdated" "1600000000" }
            "beta"
            {
                "buildid" "5678"
                "pwdrequired" "1"
                "description" "test branch"
                "timeupdated" "1600000001"
                "timebuildupdated" "1600000002"
            }
        }
        "1007"
        {
            "name" "Spacewar Content"
            "config" { "oslist" "windows,macos" "osarch" "64" "language" "english" }
            "manifests" { "public" { "gid" "111" "size" "2048" "download" "1024" } }
        }
        "1008"
        {
            "depotfromapp" "440"
            "manifests" { "public" "222" }
        }
    }
}
"#;

    fn details() -> AppDetails {
        let kv = parse_text_kv(APP_KV).unwrap();
        AppDetails::from_key_values(AppId(480), kv)
    }

    #[test]
    fn parses_common_and_depots() {
        let d = details();
        assert_eq!(d.name.as_deref(), Some("Spacewar"));
        assert_eq!(d.app_type.as_deref(), Some("game"));
        // "branches" must not be mistaken for a depot.
        assert_eq!(d.depots.len(), 2);
        assert_eq!(d.first_depot().map(|d| d.id), Some(DepotId(1007)));
    }

    #[test]
    fn parses_depot_config_and_manifest() {
        let d = details();
        let content = d.depot(DepotId(1007)).unwrap();
        assert_eq!(content.name.as_deref(), Some("Spacewar Content"));
        assert_eq!(content.config.os_list, ["windows", "macos"]);
        assert_eq!(content.config.os_arch.as_deref(), Some("64"));
        assert_eq!(content.config.language.as_deref(), Some("english"));
        assert!(!content.is_shared());
        let m = content.manifest("public").unwrap();
        assert_eq!(m.manifest_id, ManifestId(111));
        assert_eq!(m.size, Some(2048));
        assert_eq!(m.download_size, Some(1024));
    }

    #[test]
    fn parses_shared_depot_and_bare_manifest() {
        let d = details();
        let shared = d.depot(DepotId(1008)).unwrap();
        assert_eq!(shared.depot_from_app, Some(AppId(440)));
        assert!(shared.is_shared());
        // Branch entry is a bare manifest-id string, not a subsection.
        let m = shared.manifest("public").unwrap();
        assert_eq!(m.manifest_id, ManifestId(222));
        assert_eq!(m.size, None);
    }

    #[test]
    fn parses_branches() {
        let d = details();
        let beta = d.branch("beta").unwrap();
        assert_eq!(beta.build_id, Some(BuildId(5678)));
        assert!(beta.pwd_required);
        assert_eq!(beta.description.as_deref(), Some("test branch"));
        assert_eq!(beta.time_updated, Some(1600000001));
        assert_eq!(beta.time_built, Some(1600000002));

        let public = d.branch("public").unwrap();
        assert!(!public.pwd_required);
        assert_eq!(public.time_updated, Some(1600000000));
        assert_eq!(public.time_built, None);
    }
}
