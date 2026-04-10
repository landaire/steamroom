mod cli;
mod download;
mod errors;

use clap::Parser;
use cli::*;
use errors::CliError;
use steam::depot::*;
use steam::types::KeyValue;

#[derive(Clone, Debug)]
pub struct BranchInfo {
    pub name: String,
    pub build_id: Option<BuildId>,
    pub time_updated: Option<u64>,
    pub password_required: bool,
    pub description: Option<String>,
}

#[derive(Clone, Debug)]
pub struct DepotFilter {
    pub depot_ids: Option<Vec<DepotId>>,
    pub os: Option<String>,
    pub arch: Option<String>,
    pub language: Option<String>,
    pub all_platforms: bool,
    pub all_architectures: bool,
    pub all_languages: bool,
}

#[derive(Clone, Debug)]
pub struct DepotInfo {
    pub depot_id: DepotId,
    pub name: Option<String>,
    pub config: Option<serde_json::Value>,
}

#[derive(Clone, Debug)]
pub struct DepotManifestEntry {
    pub depot_id: DepotId,
    pub manifest_id: ManifestId,
    pub branch: String,
}

#[tokio::main]
async fn main() -> Result<(), CliError> {
    let cli = Cli::parse();
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    match cli.command {
        Command::Info(args) => run_info(args).await,
        Command::Manifests(args) => run_manifests(args).await,
        Command::Files(args) => run_files(args).await,
        Command::Download(args) => run_download(args).await,
        Command::Workshop(args) => run_workshop(args).await,
    }
}

async fn connect_and_login() -> Result<(), CliError> {
    todo!()
}

async fn authenticate_credentials() -> Result<(), CliError> {
    todo!()
}

async fn authenticate_qr() -> Result<(), CliError> {
    todo!()
}

fn build_logon_body() -> Vec<u8> {
    todo!()
}

async fn try_connect_login() -> Result<(), CliError> {
    todo!()
}

async fn discover_servers() -> Result<(), CliError> {
    todo!()
}

async fn discover_branches(_kv: &KeyValue) -> Result<Vec<BranchInfo>, CliError> {
    todo!()
}

async fn discover_build_id() -> Result<Option<ManifestId>, CliError> {
    todo!()
}

async fn discover_depots_filtered() -> Result<Vec<DepotInfo>, CliError> {
    todo!()
}

async fn discover_manifests_for_branch() -> Result<Vec<DepotManifestEntry>, CliError> {
    todo!()
}

async fn discover_manifest_id() -> Result<Option<ManifestId>, CliError> {
    todo!()
}

async fn get_app_info() -> Result<(), CliError> {
    todo!()
}

fn parse_app_kv(_data: &[u8]) -> Result<KeyValue, CliError> {
    todo!()
}

async fn resolve_manifest_id() -> Result<ManifestId, CliError> {
    todo!()
}

async fn run_download(_args: DownloadArgs) -> Result<(), CliError> {
    todo!()
}

async fn run_files(_args: FilesArgs) -> Result<(), CliError> {
    todo!()
}

async fn run_info(_args: InfoArgs) -> Result<(), CliError> {
    todo!()
}

async fn run_manifests(_args: ManifestsArgs) -> Result<(), CliError> {
    todo!()
}

async fn run_workshop(_args: WorkshopArgs) -> Result<(), CliError> {
    todo!()
}

fn fmt_size(bytes: u64) -> String {
    todo!()
}

fn fmt_timestamp(ts: u32) -> String {
    todo!()
}

fn fmt_timestamp_u64(ts: u64) -> String {
    todo!()
}
