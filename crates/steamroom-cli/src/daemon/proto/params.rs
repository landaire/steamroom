//! Per-command argument shadows of the clap-derived structs in `cli.rs`.
//! These are rkyv-archivable: `PathBuf` becomes `String`, `Regex` becomes
//! a raw pattern string, clap enums become their wire mirrors.

use super::OutputFormat;
use rkyv::Archive;
use rkyv::Deserialize;
use rkyv::Serialize;

#[derive(Archive, Serialize, Deserialize, Debug, Clone)]
#[rkyv(derive(Debug))]
pub struct DownloadParams {
    pub app: u32,
    pub depot: Option<u32>,
    pub manifest: Option<u64>,
    pub filelist: Option<String>,
    pub file_regex: Option<String>,
    pub output: Option<String>,
    pub verify: bool,
    pub os: Option<String>,
    pub arch: Option<String>,
    pub language: Option<String>,
    pub login_id: Option<u32>,
    pub all_platforms: bool,
    pub all_architectures: bool,
    pub all_languages: bool,
    pub lancache: bool,
    pub max_downloads: Option<u32>,
    pub branch: Option<String>,
    pub branch_password: Option<String>,
    pub local_keys: bool,
    pub non_atomic: bool,
    pub save_manifests: bool,
    pub bytes: bool,
}

#[derive(Archive, Serialize, Deserialize, Debug, Clone)]
#[rkyv(derive(Debug))]
pub struct InfoParams {
    pub app: u32,
    pub format: Option<OutputFormat>,
    pub os: Option<String>,
    pub show_all: bool,
}

#[derive(Archive, Serialize, Deserialize, Debug, Clone)]
#[rkyv(derive(Debug))]
pub struct FilesParams {
    pub app: Option<u32>,
    pub depot: Option<u32>,
    pub manifest: Option<u64>,
    pub manifest_file: Option<String>,
    pub depot_key: Option<String>,
    pub branch: Option<String>,
    pub branch_password: Option<String>,
    pub os: Option<String>,
    pub format: Option<OutputFormat>,
    pub raw: bool,
    pub bytes: bool,
}

#[derive(Archive, Serialize, Deserialize, Debug, Clone)]
#[rkyv(derive(Debug))]
pub struct ManifestsParams {
    pub app: u32,
    pub branch: Option<String>,
    pub branch_password: Option<String>,
    pub format: Option<OutputFormat>,
}

#[derive(Archive, Serialize, Deserialize, Debug, Clone)]
#[rkyv(derive(Debug))]
pub struct DiffParams {
    pub app: u32,
    pub depot: u32,
    pub from: u64,
    pub to: u64,
    pub branch: Option<String>,
    pub format: Option<OutputFormat>,
}

#[derive(Archive, Serialize, Deserialize, Debug, Clone)]
#[rkyv(derive(Debug))]
pub struct PackagesParams {
    pub packages: Vec<u32>,
    pub format: Option<OutputFormat>,
}

#[derive(Archive, Serialize, Deserialize, Debug, Clone)]
#[rkyv(derive(Debug))]
pub struct SaveManifestParams {
    pub app: u32,
    pub depot: u32,
    pub manifest: Option<u64>,
    pub branch: Option<String>,
    pub output: String,
}

#[derive(Archive, Serialize, Deserialize, Debug, Clone)]
#[rkyv(derive(Debug))]
pub struct WorkshopParams {
    pub app: u32,
    pub item: u64,
    pub output: Option<String>,
}

#[derive(Archive, Serialize, Deserialize, Debug, Clone)]
#[rkyv(derive(Debug))]
pub struct LocalInfoParams {
    pub format: Option<OutputFormat>,
    pub user: Option<String>,
    pub users: bool,
}

fn pathbuf_to_string(p: std::path::PathBuf) -> String {
    p.to_string_lossy().into_owned()
}

impl From<crate::cli::DownloadArgs> for DownloadParams {
    // `a.capture` is intentionally dropped: --capture is rejected at parse
    // time when --use-daemon is set, so this path only runs with capture=None.
    fn from(a: crate::cli::DownloadArgs) -> Self {
        Self {
            app: a.app,
            depot: a.depot,
            manifest: a.manifest,
            filelist: a.filelist.map(pathbuf_to_string),
            file_regex: a.file_regex,
            output: a.output.map(pathbuf_to_string),
            verify: a.verify,
            os: a.os,
            arch: a.arch,
            language: a.language,
            login_id: a.login_id,
            all_platforms: a.all_platforms,
            all_architectures: a.all_architectures,
            all_languages: a.all_languages,
            lancache: a.lancache,
            // Saturate at u32::MAX: real CDN parallelism caps are far below this.
            max_downloads: a
                .max_downloads
                .map(|n| u32::try_from(n).unwrap_or(u32::MAX)),
            branch: a.branch,
            branch_password: a.branch_password,
            local_keys: a.local_keys,
            non_atomic: a.non_atomic,
            save_manifests: a.save_manifests,
            bytes: a.bytes,
        }
    }
}

impl From<crate::cli::InfoArgs> for InfoParams {
    fn from(a: crate::cli::InfoArgs) -> Self {
        Self {
            app: a.app,
            format: a.format.map(Into::into),
            os: a.os,
            show_all: a.show_all,
        }
    }
}

impl From<crate::cli::FilesArgs> for FilesParams {
    fn from(a: crate::cli::FilesArgs) -> Self {
        Self {
            app: a.app,
            depot: a.depot,
            manifest: a.manifest,
            manifest_file: a.manifest_file.map(pathbuf_to_string),
            depot_key: a.depot_key,
            branch: a.branch,
            branch_password: a.branch_password,
            os: a.os,
            format: a.format.map(Into::into),
            raw: a.raw,
            bytes: a.bytes,
        }
    }
}

impl From<crate::cli::ManifestsArgs> for ManifestsParams {
    fn from(a: crate::cli::ManifestsArgs) -> Self {
        Self {
            app: a.app,
            branch: a.branch,
            branch_password: a.branch_password,
            format: a.format.map(Into::into),
        }
    }
}

impl From<crate::cli::DiffArgs> for DiffParams {
    fn from(a: crate::cli::DiffArgs) -> Self {
        Self {
            app: a.app,
            depot: a.depot,
            from: a.from,
            to: a.to,
            branch: a.branch,
            format: a.format.map(Into::into),
        }
    }
}

impl From<crate::cli::PackagesArgs> for PackagesParams {
    fn from(a: crate::cli::PackagesArgs) -> Self {
        Self {
            packages: a.packages,
            format: a.format.map(Into::into),
        }
    }
}

impl From<crate::cli::SaveManifestArgs> for SaveManifestParams {
    fn from(a: crate::cli::SaveManifestArgs) -> Self {
        Self {
            app: a.app,
            depot: a.depot,
            manifest: a.manifest,
            branch: a.branch,
            output: pathbuf_to_string(a.output),
        }
    }
}

impl From<crate::cli::WorkshopArgs> for WorkshopParams {
    fn from(a: crate::cli::WorkshopArgs) -> Self {
        Self {
            app: a.app,
            item: a.item,
            output: a.output.map(pathbuf_to_string),
        }
    }
}

impl From<crate::cli::LocalInfoArgs> for LocalInfoParams {
    fn from(a: crate::cli::LocalInfoArgs) -> Self {
        Self {
            format: a.format.map(Into::into),
            user: a.user,
            users: a.users,
        }
    }
}

impl From<DownloadParams> for crate::cli::DownloadArgs {
    fn from(p: DownloadParams) -> Self {
        Self {
            app: p.app,
            depot: p.depot,
            manifest: p.manifest,
            filelist: p.filelist.map(std::path::PathBuf::from),
            file_regex: p.file_regex,
            output: p.output.map(std::path::PathBuf::from),
            verify: p.verify,
            os: p.os,
            arch: p.arch,
            language: p.language,
            login_id: p.login_id,
            all_platforms: p.all_platforms,
            all_architectures: p.all_architectures,
            all_languages: p.all_languages,
            lancache: p.lancache,
            max_downloads: p.max_downloads.map(|n| n as usize),
            branch: p.branch,
            branch_password: p.branch_password,
            local_keys: p.local_keys,
            non_atomic: p.non_atomic,
            save_manifests: p.save_manifests,
            capture: None, // --capture is rejected at parse time under --use-daemon
            bytes: p.bytes,
        }
    }
}

impl From<InfoParams> for crate::cli::InfoArgs {
    fn from(p: InfoParams) -> Self {
        Self {
            app: p.app,
            format: p.format.map(Into::into),
            os: p.os,
            show_all: p.show_all,
        }
    }
}

impl From<FilesParams> for crate::cli::FilesArgs {
    fn from(p: FilesParams) -> Self {
        Self {
            app: p.app,
            depot: p.depot,
            manifest: p.manifest,
            manifest_file: p.manifest_file.map(std::path::PathBuf::from),
            depot_key: p.depot_key,
            branch: p.branch,
            branch_password: p.branch_password,
            os: p.os,
            format: p.format.map(Into::into),
            raw: p.raw,
            bytes: p.bytes,
        }
    }
}

impl From<ManifestsParams> for crate::cli::ManifestsArgs {
    fn from(p: ManifestsParams) -> Self {
        Self {
            app: p.app,
            branch: p.branch,
            branch_password: p.branch_password,
            format: p.format.map(Into::into),
        }
    }
}

impl From<DiffParams> for crate::cli::DiffArgs {
    fn from(p: DiffParams) -> Self {
        Self {
            app: p.app,
            depot: p.depot,
            from: p.from,
            to: p.to,
            branch: p.branch,
            format: p.format.map(Into::into),
        }
    }
}

impl From<PackagesParams> for crate::cli::PackagesArgs {
    fn from(p: PackagesParams) -> Self {
        Self {
            packages: p.packages,
            format: p.format.map(Into::into),
        }
    }
}

impl From<SaveManifestParams> for crate::cli::SaveManifestArgs {
    fn from(p: SaveManifestParams) -> Self {
        Self {
            app: p.app,
            depot: p.depot,
            manifest: p.manifest,
            branch: p.branch,
            output: std::path::PathBuf::from(p.output),
        }
    }
}

impl From<WorkshopParams> for crate::cli::WorkshopArgs {
    fn from(p: WorkshopParams) -> Self {
        Self {
            app: p.app,
            item: p.item,
            output: p.output.map(std::path::PathBuf::from),
        }
    }
}

impl From<LocalInfoParams> for crate::cli::LocalInfoArgs {
    fn from(p: LocalInfoParams) -> Self {
        Self {
            format: p.format.map(Into::into),
            user: p.user,
            users: p.users,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rkyv::rancor;

    #[test]
    fn download_params_round_trip() {
        let p = DownloadParams {
            app: 730,
            depot: Some(731),
            manifest: Some(1234),
            filelist: Some("files.txt".into()),
            file_regex: Some(r"\.dll$".into()),
            output: Some("out".into()),
            verify: true,
            os: None,
            arch: None,
            language: None,
            login_id: None,
            all_platforms: false,
            all_architectures: false,
            all_languages: false,
            lancache: false,
            max_downloads: Some(16),
            branch: Some("public".into()),
            branch_password: None,
            local_keys: false,
            non_atomic: false,
            save_manifests: false,
            bytes: false,
        };
        let bytes = rkyv::to_bytes::<rancor::Error>(&p).unwrap();
        let back = rkyv::from_bytes::<DownloadParams, rancor::Error>(&bytes).unwrap();
        assert_eq!(back.app, 730);
        assert_eq!(back.depot, Some(731));
        assert_eq!(back.file_regex.as_deref(), Some(r"\.dll$"));
        assert_eq!(back.max_downloads, Some(16));
    }

    #[test]
    fn packages_params_round_trip_with_vec() {
        let p = PackagesParams {
            packages: vec![1, 2, 3],
            format: Some(OutputFormat::Json),
        };
        let bytes = rkyv::to_bytes::<rancor::Error>(&p).unwrap();
        let back = rkyv::from_bytes::<PackagesParams, rancor::Error>(&bytes).unwrap();
        assert_eq!(back.packages, vec![1, 2, 3]);
        assert!(matches!(back.format, Some(OutputFormat::Json)));
    }
}
