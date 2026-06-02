//! `steamroom manifests`: list current manifest IDs for each depot in an
//! app, for a given branch.

use crate::cli::ManifestsArgs;
use crate::cli::OutputFormat;
use crate::commands::shared::fetch_app_kv;
use crate::errors::CliError;
use crate::sink::JobSink;
use steamroom::client::LoggedIn;
use steamroom::client::SteamClient;
use steamroom::depot::*;
use steamroom::types::key_value::KvValue;
use tabled::builder::Builder as TableBuilder;
use tabled::settings::Style;
use tokio_util::sync::CancellationToken;

pub async fn run_manifests(
    args: ManifestsArgs,
    client: SteamClient<LoggedIn>,
    sink: &dyn JobSink,
    _cancel: CancellationToken,
) -> Result<(), CliError> {
    let app_id = AppId(args.app);
    let kv = fetch_app_kv(&client, app_id).await?;
    let branch = args.branch.as_deref().unwrap_or("public");

    let depots = kv.get("depots").ok_or(CliError::NoDepots)?;

    if args.format == Some(OutputFormat::Json) {
        let mut entries = Vec::new();
        if let KvValue::Children(ref map) = depots.value {
            for (key, depot) in map {
                let Ok(depot_id) = key.parse::<u32>() else {
                    continue;
                };
                if let Some(manifests) = depot.get("manifests")
                    && let Some(branch_kv) = manifests.get(branch)
                {
                    let gid = branch_kv
                        .get("gid")
                        .and_then(|g| g.as_str())
                        .or_else(|| branch_kv.as_str());
                    if let Some(manifest_id) = gid {
                        entries.push(serde_json::json!({
                            "depot": depot_id,
                            "manifest": manifest_id,
                        }));
                    }
                }
            }
        }
        sink.stdout_line(&serde_json::to_string_pretty(&entries)?);
        return Ok(());
    }

    sink.stdout_line(&format!("Manifests for branch '{branch}':"));
    sink.stdout_line("");

    if let KvValue::Children(ref map) = depots.value {
        let mut rows: Vec<[String; 3]> = Vec::new();
        for (key, depot) in map {
            let Ok(depot_id) = key.parse::<u32>() else {
                continue;
            };
            let dname = depot.get("name").and_then(|n| n.as_str()).unwrap_or("");
            if let Some(manifests) = depot.get("manifests") {
                if let Some(branch_kv) = manifests.get(branch) {
                    let gid = branch_kv
                        .get("gid")
                        .and_then(|g| g.as_str())
                        .or_else(|| branch_kv.as_str())
                        .unwrap_or("--");
                    rows.push([
                        format!("depot {depot_id}"),
                        format!("-> {gid}"),
                        dname.to_string(),
                    ]);
                } else {
                    rows.push([
                        format!("depot {depot_id}"),
                        "-> --".to_string(),
                        dname.to_string(),
                    ]);
                }
            } else {
                rows.push([
                    format!("depot {depot_id}"),
                    "-> --".to_string(),
                    dname.to_string(),
                ]);
            }
        }

        let mut builder = TableBuilder::new();
        for r in &rows {
            builder.push_record(r);
        }
        let table = builder
            .build()
            .with(Style::blank())
            .with(tabled::settings::Padding::new(0, 2, 0, 0))
            .to_string();
        for line in table.lines() {
            sink.stdout_line(&format!("  {line}"));
        }
    }

    Ok(())
}
