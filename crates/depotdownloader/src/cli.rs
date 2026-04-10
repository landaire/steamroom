use clap::{Parser, Subcommand, ValueEnum};
use steam::depot::{AppId, CellId, DepotId, ManifestId};

#[derive(Parser, Debug)]
#[command(name = "ddl", about = "Steam depot downloader")]
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

#[derive(Parser, Debug)]
pub struct CompatCli {
    // Legacy CLI compatibility fields
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

    #[arg(long)]
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
    pub branch: Option<String>,
    #[arg(long)]
    pub branch_password: Option<String>,
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

#[derive(Copy, Clone, Debug, ValueEnum)]
pub enum OutputFormat {
    Table,
    Json,
    Plain,
}
