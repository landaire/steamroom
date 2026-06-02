//! `steamroom diff`: compare two manifests for the same depot and print
//! added, removed, and changed files.

use crate::cli::DiffArgs;
use crate::cli::OutputFormat;
use crate::commands::shared::fetch_manifest;
use crate::commands::shared::fmt_size;
use crate::commands::shared::fmt_timestamp;
use crate::errors::CliError;
use crate::sink::JobSink;
use steamroom::client::LoggedIn;
use steamroom::client::SteamClient;
use steamroom::depot::*;
use tabled::builder::Builder as TableBuilder;
use tabled::settings::Style;
use tokio_util::sync::CancellationToken;
use tracing::info;

pub async fn run_diff(
    args: DiffArgs,
    client: SteamClient<LoggedIn>,
    sink: &dyn JobSink,
    _cancel: CancellationToken,
) -> Result<(), CliError> {
    let app_id = AppId(args.app);
    let depot_id = DepotId(args.depot);
    let from_id = ManifestId(args.from);
    let to_id = ManifestId(args.to);
    let branch = args.branch.as_deref();

    info!("fetching old manifest {from_id}...");
    let old = fetch_manifest(&client, app_id, depot_id, from_id, branch).await?;
    info!("fetching new manifest {to_id}...");
    let new = fetch_manifest(&client, app_id, depot_id, to_id, branch).await?;

    // Build lookup maps: filename -> (size, sha)
    let old_files: std::collections::HashMap<&str, &steamroom::depot::manifest::ManifestFile> =
        old.files.iter().map(|f| (f.filename.as_str(), f)).collect();
    let new_files: std::collections::HashMap<&str, &steamroom::depot::manifest::ManifestFile> =
        new.files.iter().map(|f| (f.filename.as_str(), f)).collect();

    let mut added: Vec<&steamroom::depot::manifest::ManifestFile> = Vec::new();
    let mut removed: Vec<&steamroom::depot::manifest::ManifestFile> = Vec::new();
    let mut changed: Vec<(
        &steamroom::depot::manifest::ManifestFile,
        &steamroom::depot::manifest::ManifestFile,
    )> = Vec::new();

    for (name, new_file) in &new_files {
        match old_files.get(name) {
            None => added.push(new_file),
            Some(old_file) => {
                if old_file.sha_content != new_file.sha_content || old_file.size != new_file.size {
                    changed.push((old_file, new_file));
                }
            }
        }
    }
    for (name, old_file) in &old_files {
        if !new_files.contains_key(name) {
            removed.push(old_file);
        }
    }

    added.sort_by_key(|f| &f.filename);
    removed.sort_by_key(|f| &f.filename);
    changed.sort_by_key(|(_, f)| &f.filename);

    if args.format == Some(OutputFormat::Json) {
        let json = serde_json::json!({
            "from": args.from,
            "to": args.to,
            "added": added.iter().map(|f| serde_json::json!({"filename": &f.filename, "size": f.size})).collect::<Vec<_>>(),
            "removed": removed.iter().map(|f| serde_json::json!({"filename": &f.filename, "size": f.size})).collect::<Vec<_>>(),
            "changed": changed.iter().map(|(old, new)| serde_json::json!({"filename": &new.filename, "old_size": old.size, "new_size": new.size})).collect::<Vec<_>>(),
        });
        sink.stdout_line(&serde_json::to_string_pretty(&json)?);
        return Ok(());
    }

    let old_epoch = old.creation_time.unwrap_or(0) as u64;
    let new_epoch = new.creation_time.unwrap_or(0) as u64;
    let span = if new_epoch > old_epoch {
        let secs = new_epoch - old_epoch;
        let dur = jiff::SignedDuration::from_secs(secs as i64);
        let hours = dur.as_hours();
        let days = hours / 24;
        if days >= 365 {
            let years = days / 365;
            let months = (days % 365) / 30;
            if months > 0 {
                format!("{years}y {months}mo")
            } else {
                format!("{years}y")
            }
        } else if days >= 30 {
            let months = days / 30;
            let rem = days % 30;
            if rem > 0 {
                format!("{months}mo {rem}d")
            } else {
                format!("{months}mo")
            }
        } else if days > 0 {
            format!("{days}d")
        } else {
            format!("{hours}h")
        }
    } else {
        String::new()
    };

    sink.stdout_line(&format!("Depot:  {depot_id}"));
    sink.stdout_line(&format!("From:   {from_id} ({})", fmt_timestamp(old_epoch)));
    sink.stdout_line(&format!("To:     {to_id} ({})", fmt_timestamp(new_epoch)));
    if !span.is_empty() {
        sink.stdout_line(&format!("Delta:  {span} apart"));
    }
    sink.stdout_line("");

    if added.is_empty() && removed.is_empty() && changed.is_empty() {
        sink.stdout_line("No differences.");
        return Ok(());
    }

    let mut rows: Vec<[String; 3]> = Vec::new();

    for f in &added {
        rows.push([format!("+ {}", f.filename), fmt_size(f.size), String::new()]);
    }
    for f in &removed {
        rows.push([format!("- {}", f.filename), fmt_size(f.size), String::new()]);
    }
    for (old, new) in &changed {
        let size_diff = new.size as i64 - old.size as i64;
        let diff_str = if size_diff > 0 {
            format!("+{}", fmt_size(size_diff as u64))
        } else if size_diff < 0 {
            format!("-{}", fmt_size((-size_diff) as u64))
        } else {
            "content".to_string()
        };
        rows.push([format!("~ {}", new.filename), fmt_size(new.size), diff_str]);
    }

    let mut builder = TableBuilder::new();
    builder.push_record(["FILE", "SIZE", "DELTA"]);
    for r in &rows {
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

    sink.stdout_line("");
    sink.stdout_line(&format!(
        "{} added, {} removed, {} changed",
        added.len(),
        removed.len(),
        changed.len()
    ));

    Ok(())
}
