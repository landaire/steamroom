//! `steamroom files`: list files in a depot manifest, either from a local
//! manifest file or by fetching from the Steam CDN.

use crate::cli::FilesArgs;
use crate::cli::OutputFormat;
use crate::commands::shared::decompress_manifest;
use crate::commands::shared::fetch_app_kv;
use crate::commands::shared::find_first_depot;
use crate::commands::shared::find_manifest_for_depot;
use crate::commands::shared::fmt_size;
use crate::commands::shared::fmt_timestamp;
use crate::commands::shared::resolve_depot_key;
use crate::errors::CliError;
use crate::sink::JobSink;
use std::sync::Arc;
use steamroom::cdn::CdnClient;
use steamroom::client::LoggedIn;
use steamroom::client::SteamClient;
use steamroom::depot::manifest::DepotManifest;
use steamroom::depot::*;
use tabled::builder::Builder as TableBuilder;
use tabled::settings::Style;
use tokio_util::sync::CancellationToken;

pub async fn run_files(
    args: FilesArgs,
    client: Option<SteamClient<LoggedIn>>,
    sink: Arc<dyn JobSink>,
    _cancel: CancellationToken,
) -> Result<(), CliError> {
    let raw_bytes = args.bytes;

    let manifest = if let Some(ref path) = args.manifest_file {
        // Local manifest file
        let data = std::fs::read(path)?;
        let manifest_bytes = if data.len() > 2 && data[0] == 0x50 && data[1] == 0x4B {
            decompress_manifest(&data)?
        } else {
            data
        };
        let mut m = DepotManifest::parse(&manifest_bytes)?;
        if m.filenames_encrypted && !args.raw {
            let key = resolve_depot_key(&args)?;
            m.decrypt_filenames(&key)?;
        }
        m
    } else {
        // Fetch from Steam
        let client = client.ok_or_else(|| {
            CliError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "fetching manifest from Steam requires an authenticated client",
            ))
        })?;
        let app_id = AppId(args.app.ok_or(CliError::NoDepots)?);
        let kv = fetch_app_kv(&client, app_id).await?;
        let branch = args.branch.as_deref().unwrap_or("public");

        let depot_id = args
            .depot
            .map(DepotId)
            .or_else(|| kv.get("depots").and_then(|d| find_first_depot(d).ok()))
            .ok_or(CliError::NoDepots)?;

        let manifest_id = args
            .manifest
            .map(ManifestId)
            .or_else(|| {
                kv.get("depots")
                    .and_then(|d| find_manifest_for_depot(d, depot_id, branch).ok())
            })
            .ok_or(CliError::ManifestNotFound {
                depot: depot_id.0,
                branch: branch.to_string(),
            })?;

        let depot_key = client.get_depot_decryption_key(depot_id, app_id).await?;
        let request_code = client
            .get_manifest_request_code(app_id, depot_id, manifest_id, Some(branch), None)
            .await?
            .unwrap_or(0);

        let cdn_servers = client.get_cdn_servers(CellId(0), Some(5)).await?;
        let cdn_server = cdn_servers.first().ok_or(CliError::NoCdnServers)?;
        let cdn = CdnClient::new().map_err(CliError::Steam)?;
        let manifest_data = cdn
            .download_manifest(cdn_server, depot_id, manifest_id, request_code, None)
            .await?;
        let manifest_bytes = decompress_manifest(&manifest_data)?;
        let mut m = DepotManifest::parse(&manifest_bytes)?;
        if m.filenames_encrypted && !args.raw {
            m.decrypt_filenames(&depot_key)?;
        }
        m
    };

    if args.format == Some(OutputFormat::Json) {
        let entries: Vec<serde_json::Value> = manifest
            .files
            .iter()
            .map(|f| {
                serde_json::json!({
                    "filename": &f.filename,
                    "size": f.size,
                    "flags": f.flags,
                    "chunks": f.chunks.len(),
                })
            })
            .collect();
        sink.stdout_line(&serde_json::to_string_pretty(&entries)?);
        return Ok(());
    }

    if args.format == Some(OutputFormat::Plain) {
        for file in &manifest.files {
            sink.stdout_line(&file.filename);
        }
    } else {
        let total_size: u64 = manifest.files.iter().map(|f| f.size).sum();
        let file_count = manifest.files.len();
        let created = manifest
            .creation_time
            .map(|t| fmt_timestamp(t as u64))
            .unwrap_or_else(|| "-".into());

        sink.stdout_line(&format!(
            "Depot:    {}",
            manifest
                .depot_id
                .map(|d| d.0.to_string())
                .unwrap_or("-".into())
        ));
        sink.stdout_line(&format!(
            "Manifest: {}",
            manifest
                .manifest_id
                .map(|m| m.0.to_string())
                .unwrap_or("-".into())
        ));
        sink.stdout_line(&format!("Created:  {}", created));
        sink.stdout_line(&format!("Size:     {}", fmt_size(total_size)));
        sink.stdout_line(&format!("Files:    {}", file_count));
        sink.stdout_line("");

        let file_rows: Vec<[String; 3]> = manifest
            .files
            .iter()
            .map(|f| {
                let is_dir = steamroom::enums::DepotFileFlags(f.flags).is_directory();
                let name = if is_dir {
                    format!("{}/", f.filename)
                } else {
                    f.filename.clone()
                };
                let size_str = if raw_bytes {
                    f.size.to_string()
                } else {
                    fmt_size(f.size)
                };
                [name, size_str, f.chunks.len().to_string()]
            })
            .collect();

        let mut builder = TableBuilder::new();
        builder.push_record(["FILENAME", "SIZE", "CHUNKS"]);
        for r in &file_rows {
            builder.push_record(r);
        }
        let table = builder
            .build()
            .with(Style::blank())
            .with(tabled::settings::Padding::new(0, 2, 0, 0))
            .with(
                tabled::settings::Modify::new(tabled::settings::object::Columns::new(1..))
                    .with(tabled::settings::Alignment::right()),
            )
            .to_string();
        for line in table.lines() {
            sink.stdout_line(line);
        }
    }

    Ok(())
}
