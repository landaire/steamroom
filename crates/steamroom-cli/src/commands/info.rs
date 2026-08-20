//! `steamroom info`: pretty-print app metadata (name, type, depots,
//! branches) for a given app ID.

use crate::cli::InfoArgs;
use crate::cli::OutputFormat;
use crate::commands::shared::fetch_app_details;
use crate::commands::shared::fmt_relative;
use crate::commands::shared::fmt_size;
use crate::commands::shared::kv_to_json;
use crate::errors::CliError;
use crate::sink::JobSink;
use std::sync::Arc;
use steamroom::apps::Depot;
use steamroom::client::LoggedIn;
use steamroom::client::SteamClient;
use steamroom::depot::*;
use tabled::builder::Builder as TableBuilder;
use tabled::settings::Style;
use tokio_util::sync::CancellationToken;

pub async fn run_info(
    args: InfoArgs,
    client: SteamClient<LoggedIn>,
    sink: Arc<dyn JobSink>,
    _cancel: CancellationToken,
) -> Result<(), CliError> {
    let app_id = AppId(args.app);
    let details = fetch_app_details(&client, app_id).await?;

    if args.format == Some(OutputFormat::Json) {
        sink.stdout_line(&serde_json::to_string_pretty(&kv_to_json(&details.key_values))?);
        return Ok(());
    }

    sink.stdout_line(&format!("App ID:  {}", app_id));
    sink.stdout_line(&format!("Name:    {}", details.name.as_deref().unwrap_or("(unknown)")));
    sink.stdout_line(&format!("Type:    {}", details.app_type.as_deref().unwrap_or("(unknown)")));

    let branch_name = "public";

    let mut depot_rows: Vec<[String; 4]> = Vec::new();
    let mut redist_rows: Vec<[String; 4]> = Vec::new();
    for depot in &details.depots {
        let os = &depot.config.os_list;

        // Apply --os filter
        if let Some(ref filter_os) = args.os
            && !os.is_empty()
            && !os.iter().any(|o| o.eq_ignore_ascii_case(filter_os))
        {
            continue;
        }

        let row = [
            depot.id.to_string(),
            depot_config_summary(depot),
            depot
                .manifest(branch_name)
                .and_then(|m| m.size)
                .map(fmt_size)
                .unwrap_or_default(),
            depot
                .manifest(branch_name)
                .and_then(|m| m.download_size)
                .map(fmt_size)
                .unwrap_or_default(),
        ];
        if depot.is_shared() {
            redist_rows.push(row);
        } else {
            depot_rows.push(row);
        }
    }

    let print_depot_table = |label: &str, rows: &[[String; 4]]| {
        if rows.is_empty() {
            return;
        }
        sink.stdout_line("");
        sink.stdout_line(&format!("{label}:"));
        let mut builder = TableBuilder::new();
        builder.push_record(["ID", "CONFIGURATION", "SIZE", "DL."]);
        for r in rows {
            builder.push_record(r);
        }
        let table = builder
            .build()
            .with(Style::blank())
            .with(tabled::settings::Padding::new(0, 2, 0, 0))
            .with(
                tabled::settings::Modify::new(tabled::settings::object::Columns::new(2..4))
                    .with(tabled::settings::Alignment::right()),
            )
            .to_string();
        for line in table.lines() {
            sink.stdout_line(&format!("  {line}"));
        }
    };

    print_depot_table("Depots", &depot_rows);
    if args.show_all {
        print_depot_table("Redistributables", &redist_rows);
    }

    if !details.branches.is_empty() {
        // (time_updated_epoch, row) so we can sort most-recent first.
        let mut branch_entries: Vec<(u64, [String; 5])> = Vec::new();
        for branch in &details.branches {
            let build_id = branch
                .build_id
                .map(|b| b.to_string())
                .unwrap_or_else(|| "-".to_string());
            let desc = branch.description.as_deref().unwrap_or("").trim();
            let trimmed_desc = if desc.len() > 40 {
                format!("{}...", &desc[..37])
            } else {
                desc.to_string()
            };
            let time_built_epoch = branch.time_built.or(branch.time_updated).unwrap_or(0);
            let time_updated_epoch = branch.time_updated.unwrap_or(0);
            let time_built = if time_built_epoch > 0 {
                fmt_relative(time_built_epoch)
            } else {
                String::new()
            };
            let time_updated = if time_updated_epoch > 0 {
                fmt_relative(time_updated_epoch)
            } else {
                String::new()
            };
            let mut name_str = branch.name.clone();
            if branch.pwd_required {
                name_str.push_str(" [password]");
            }
            branch_entries.push((
                time_updated_epoch,
                [name_str, trimmed_desc, build_id, time_built, time_updated],
            ));
        }

        branch_entries.sort_by_key(|b| std::cmp::Reverse(b.0));
        let branch_rows: Vec<[String; 5]> = branch_entries.into_iter().map(|(_, r)| r).collect();

        sink.stdout_line("");
        sink.stdout_line("Branches:");
        let mut builder = TableBuilder::new();
        builder.push_record(["NAME", "DESCRIPTION", "BUILD", "TIME BUILT", "TIME UPDATED"]);
        for r in &branch_rows {
            builder.push_record(r);
        }
        let branch_table = builder
            .build()
            .with(Style::blank())
            .with(tabled::settings::Padding::new(0, 2, 0, 0))
            .to_string();
        for line in branch_table.lines() {
            sink.stdout_line(&format!("  {line}"));
        }
    }

    Ok(())
}

/// Human-readable one-line summary of a depot's platform/config and name.
fn depot_config_summary(depot: &Depot) -> String {
    let mut parts = Vec::new();
    if !depot.config.os_list.is_empty() {
        parts.push(
            depot
                .config
                .os_list
                .iter()
                .map(|o| match o.as_str() {
                    "windows" => "Windows",
                    "macos" => "macOS",
                    "linux" => "Linux",
                    other => other,
                })
                .collect::<Vec<_>>()
                .join(", "),
        );
    }
    if let Some(ref arch) = depot.config.os_arch {
        parts.push(format!("{arch}-bit"));
    }
    if let Some(ref lang) = depot.config.language {
        let cap = lang.get(..1).map(|c| c.to_uppercase()).unwrap_or_default()
            + lang.get(1..).unwrap_or("");
        parts.push(cap);
    }
    if depot.config.low_violence {
        parts.push("Low Violence".to_string());
    }
    if depot.shared_install {
        parts.push("Shared Install".to_string());
    }
    if let Some(from_app) = depot.depot_from_app {
        parts.push(format!("from app {from_app}"));
    }
    if let Some(ref name) = depot.name {
        parts.push(name.clone());
    }
    parts.join(", ")
}
