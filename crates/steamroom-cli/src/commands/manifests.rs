//! `steamroom manifests`: list current manifest IDs for each depot in an
//! app, for a given branch.

use crate::cli::ManifestsArgs;
use crate::cli::OutputFormat;
use crate::commands::shared::fetch_app_details;
use crate::errors::CliError;
use crate::sink::JobSink;
use std::sync::Arc;
use steamroom::client::LoggedIn;
use steamroom::client::SteamClient;
use steamroom::depot::*;
use tabled::builder::Builder as TableBuilder;
use tabled::settings::Style;
use tokio_util::sync::CancellationToken;

pub async fn run_manifests(
    args: ManifestsArgs,
    client: SteamClient<LoggedIn>,
    sink: Arc<dyn JobSink>,
    _cancel: CancellationToken,
) -> Result<(), CliError> {
    let app_id = AppId(args.app);
    let details = fetch_app_details(&client, app_id).await?;
    let branch = args.branch.as_deref().unwrap_or("public");

    // Error only when the app has no depots section at all; a section that
    // carries branches but no numeric depots still yields empty output.
    if details.key_values.get("depots").is_none() {
        return Err(CliError::NoDepots);
    }

    if args.format == Some(OutputFormat::Json) {
        let entries: Vec<_> = details
            .depots
            .iter()
            .filter_map(|depot| {
                let manifest = depot.manifest(branch)?;
                Some(serde_json::json!({
                    "depot": depot.id.0,
                    "manifest": manifest.manifest_id.0.to_string(),
                }))
            })
            .collect();
        sink.stdout_line(&serde_json::to_string_pretty(&entries)?);
        return Ok(());
    }

    sink.stdout_line(&format!("Manifests for branch '{branch}':"));
    sink.stdout_line("");

    let mut builder = TableBuilder::new();
    for depot in &details.depots {
        let gid = depot
            .manifest(branch)
            .map(|m| m.manifest_id.to_string())
            .unwrap_or_else(|| "--".to_string());
        builder.push_record([
            format!("depot {}", depot.id),
            format!("-> {gid}"),
            depot.name.clone().unwrap_or_default(),
        ]);
    }
    let table = builder
        .build()
        .with(Style::blank())
        .with(tabled::settings::Padding::new(0, 2, 0, 0))
        .to_string();
    for line in table.lines() {
        sink.stdout_line(&format!("  {line}"));
    }

    Ok(())
}
