use clap::{Parser, Subcommand, ValueEnum};
use steamroom::depot::CellId;

#[derive(Parser, Debug)]
#[command(name = "steamroom", about = "Steam depot downloader")]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,

    #[command(flatten)]
    pub auth: AuthOptions,

    #[arg(long)]
    pub debug: bool,

    #[arg(long)]
    pub bytes: bool,

    #[arg(long)]
    pub raw_errors: bool,

    #[arg(long)]
    pub cell_id: Option<u32>,

    #[arg(long)]
    pub max_downloads: Option<usize>,

    #[arg(long)]
    pub capture: Option<std::path::PathBuf>,
}

/// Legacy flat-argument CLI compatible with the original DepotDownloader.
/// Activated with DD_COMPAT=1 environment variable.
#[derive(Parser, Debug)]
#[command(name = "steamroom")]
pub struct CompatCli {
    #[arg(long = "app", short = 'a')]
    pub app_id: Option<u32>,
    #[arg(long = "depot", short = 'd')]
    pub depot_id: Option<u32>,
    #[arg(long = "manifest", short = 'm')]
    pub manifest_id: Option<u64>,
    #[arg(long = "username", short = 'u')]
    pub username: Option<String>,
    #[arg(long = "password", short = 'p')]
    pub password: Option<String>,
    #[arg(long = "dir")]
    pub output: Option<std::path::PathBuf>,
    #[arg(long = "branch", short = 'b')]
    pub branch: Option<String>,
    #[arg(long = "betapassword")]
    pub beta_password: Option<String>,
    #[arg(long)]
    pub qr: bool,
    #[arg(long = "remember-password")]
    pub remember_password: bool,
    #[arg(long = "filelist")]
    pub filelist: Option<std::path::PathBuf>,
    #[arg(long = "regex")]
    pub file_regex: Option<String>,
    #[arg(long)]
    pub verify: bool,
    #[arg(long)]
    pub os: Option<String>,
    #[arg(long)]
    pub arch: Option<String>,
    #[arg(long)]
    pub language: Option<String>,
    #[arg(long = "max-downloads")]
    pub max_downloads: Option<usize>,
    #[arg(long = "cell-id")]
    pub cell_id: Option<u32>,
}

impl CompatCli {
    pub fn into_cli(self) -> Cli {
        let app = self.app_id.unwrap_or(0);
        Cli {
            command: Command::Download(DownloadArgs {
                app,
                depot: self.depot_id,
                manifest: self.manifest_id,
                filelist: self.filelist,
                file_regex: self.file_regex,
                output: self.output,
                verify: self.verify,
                os: self.os,
                arch: self.arch,
                language: self.language,
                login_id: None,
                all_platforms: false,
                all_architectures: false,
                all_languages: false,
                lancache: false,
                max_downloads: self.max_downloads,
                branch: self.branch,
                branch_password: self.beta_password,
                capture: None,
            }),
            auth: AuthOptions {
                username: self.username,
                password: self.password,
                qr: self.qr,
                remember_password: self.remember_password,
                device_name: None,
            },
            debug: false,
            bytes: false,
            raw_errors: false,
            cell_id: self.cell_id,
            max_downloads: self.max_downloads,
            capture: None,
        }
    }
}

#[derive(Debug)]
pub struct Options {
    pub action: Action,
    pub auth: AuthOptions,
    pub debug: bool,
    pub raw_bytes: bool,
    pub cell_id: Option<CellId>,
    pub max_downloads: Option<usize>,
    pub capture: Option<std::path::PathBuf>,
    pub raw_errors: bool,
}

#[derive(Parser, Debug)]
pub struct AuthOptions {
    #[arg(short, long, env = "STEAM_USER")]
    pub username: Option<String>,

    #[arg(short, long, env = "STEAM_PASS")]
    pub password: Option<String>,

    #[arg(long)]
    pub qr: bool,

    #[arg(long)]
    pub remember_password: bool,

    #[arg(long, env = "DD_DEVICE_NAME")]
    pub device_name: Option<String>,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    Download(DownloadArgs),
    Files(FilesArgs),
    Info(InfoArgs),
    Manifests(ManifestsArgs),
    Workshop(WorkshopArgs),
}

pub type Action = Command;

#[derive(Parser, Debug)]
pub struct DownloadArgs {
    #[arg(long)]
    pub app: u32,
    #[arg(long)]
    pub depot: Option<u32>,
    #[arg(long)]
    pub manifest: Option<u64>,
    #[arg(long)]
    pub filelist: Option<std::path::PathBuf>,
    #[arg(long)]
    pub file_regex: Option<String>,
    #[arg(long, short)]
    pub output: Option<std::path::PathBuf>,
    #[arg(long)]
    pub verify: bool,
    #[arg(long)]
    pub os: Option<String>,
    #[arg(long)]
    pub arch: Option<String>,
    #[arg(long)]
    pub language: Option<String>,
    #[arg(long)]
    pub login_id: Option<u32>,
    #[arg(long)]
    pub all_platforms: bool,
    #[arg(long)]
    pub all_architectures: bool,
    #[arg(long)]
    pub all_languages: bool,
    #[arg(long)]
    pub lancache: bool,
    #[arg(long)]
    pub max_downloads: Option<usize>,
    #[arg(long)]
    pub branch: Option<String>,
    #[arg(long)]
    pub branch_password: Option<String>,
    #[arg(long)]
    pub capture: Option<std::path::PathBuf>,
}

#[derive(Parser, Debug)]
pub struct FilesArgs {
    #[arg(long)]
    pub app: u32,
    #[arg(long)]
    pub depot: Option<u32>,
    #[arg(long)]
    pub manifest: Option<u64>,
    #[arg(long)]
    pub branch: Option<String>,
    #[arg(long)]
    pub branch_password: Option<String>,
    #[arg(long)]
    pub os: Option<String>,
    #[arg(long, value_enum)]
    pub format: Option<OutputFormat>,
    #[arg(long)]
    pub raw: bool,
}

#[derive(Parser, Debug)]
pub struct InfoArgs {
    #[arg(long)]
    pub app: u32,
    #[arg(long, value_enum)]
    pub format: Option<OutputFormat>,
}

#[derive(Parser, Debug)]
pub struct ManifestsArgs {
    #[arg(long)]
    pub app: u32,
    #[arg(long)]
    pub branch: Option<String>,
    #[arg(long)]
    pub branch_password: Option<String>,
    #[arg(long, value_enum)]
    pub format: Option<OutputFormat>,
}

#[derive(Parser, Debug)]
pub struct WorkshopArgs {
    #[arg(long)]
    pub app: u32,
    #[arg(long)]
    pub item: u64,
    #[arg(long, short)]
    pub output: Option<std::path::PathBuf>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum OutputFormat {
    Table,
    Json,
    Plain,
}
