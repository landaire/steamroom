//! `steamroom info`: pretty-print app metadata (name, type, depots,
//! branches) for a given app ID.

use crate::cli::InfoArgs;
use crate::cli::OutputFormat;
use crate::commands::shared::fetch_app_kv;
use crate::commands::shared::fmt_relative;
use crate::commands::shared::fmt_size;
use crate::commands::shared::kv_to_json;
use crate::errors::CliError;
use crate::sink::JobSink;
use std::sync::Arc;
use steamroom::client::LoggedIn;
use steamroom::client::SteamClient;
use steamroom::depot::*;
use steamroom::types::key_value::KvValue;
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
    let kv = fetch_app_kv(&client, app_id).await?;

    if args.format == Some(OutputFormat::Json) {
        sink.stdout_line(&serde_json::to_string_pretty(&kv_to_json(&kv))?);
        return Ok(());
    }

    let name = kv
        .get("common")
        .and_then(|c| c.get("name"))
        .and_then(|n| n.as_str())
        .unwrap_or("(unknown)");
    let app_type = kv
        .get("common")
        .and_then(|c| c.get("type"))
        .and_then(|t| t.as_str())
        .unwrap_or("(unknown)");

    sink.stdout_line(&format!("App ID:  {}", app_id));
    sink.stdout_line(&format!("Name:    {name}"));
    sink.stdout_line(&format!("Type:    {app_type}"));

    let branch_name = "public";

    if let Some(depots) = kv.get("depots")
        && let KvValue::Children(ref map) = depots.value
    {
        let mut depot_rows: Vec<[String; 4]> = Vec::new();
        let mut redist_rows: Vec<[String; 4]> = Vec::new();
        for key in map.keys() {
            let Ok(_) = key.parse::<u32>() else {
                continue;
            };
            let depot = &map[key];
            let dname = depot.get("name").and_then(|n| n.as_str()).unwrap_or("");
            let is_redist =
                depot.get("depotfromapp").is_some() || depot.get("sharedinstall").is_some();

            let os = depot
                .get("config")
                .and_then(|c| c.get("oslist"))
                .and_then(|o| o.as_str())
                .unwrap_or("");

            // Apply --os filter
            if let Some(ref filter_os) = args.os
                && !os.is_empty()
                && !os
                    .split(',')
                    .any(|o| o.trim().eq_ignore_ascii_case(filter_os))
            {
                continue;
            }

            let mut config_parts = Vec::new();
            if !os.is_empty() {
                config_parts.push(
                    os.split(',')
                        .map(|o| match o.trim() {
                            "windows" => "Windows",
                            "macos" => "macOS",
                            "linux" => "Linux",
                            other => other,
                        })
                        .collect::<Vec<_>>()
                        .join(", "),
                );
            }
            if let Some(arch) = depot
                .get("config")
                .and_then(|c| c.get("osarch"))
                .and_then(|a| a.as_str())
            {
                config_parts.push(format!("{arch}-bit"));
            }
            if let Some(lang) = depot
                .get("config")
                .and_then(|c| c.get("language"))
                .and_then(|l| l.as_str())
            {
                let cap = lang.get(..1).map(|c| c.to_uppercase()).unwrap_or_default()
                    + lang.get(1..).unwrap_or("");
                config_parts.push(cap);
            }
            if depot
                .get("config")
                .and_then(|c| c.get("lowviolence"))
                .and_then(|l| l.as_str())
                == Some("1")
            {
                config_parts.push("Low Violence".to_string());
            }
            if depot.get("sharedinstall").is_some() {
                config_parts.push("Shared Install".to_string());
            }
            if let Some(from_app) = depot.get("depotfromapp").and_then(|d| d.as_str()) {
                config_parts.push(format!("from app {from_app}"));
            }
            if !dname.is_empty() {
                config_parts.push(dname.to_string());
            }
            let config_str = config_parts.join(", ");

            let size_str = depot
                .get("manifests")
                .and_then(|m| m.get(branch_name))
                .and_then(|b| b.get("size"))
                .and_then(|s| s.as_str())
                .and_then(|s| s.parse::<u64>().ok())
                .map(fmt_size)
                .unwrap_or_default();
            let dl_str = depot
                .get("manifests")
                .and_then(|m| m.get(branch_name))
                .and_then(|b| b.get("download"))
                .and_then(|s| s.as_str())
                .and_then(|s| s.parse::<u64>().ok())
                .map(fmt_size)
                .unwrap_or_default();

            let row = [key.clone(), config_str, size_str, dl_str];
            if is_redist {
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

        if let Some(branches) = depots.get("branches")
            && let KvValue::Children(ref bmap) = branches.value
        {
            let mut branch_entries: Vec<(u64, [String; 5])> = Vec::new();
            for (bname, branch) in bmap {
                let build_id = branch
                    .get("buildid")
                    .and_then(|b| b.as_str())
                    .unwrap_or("-")
                    .to_string();
                let desc = branch
                    .get("description")
                    .and_then(|d| d.as_str())
                    .unwrap_or("")
                    .trim();
                let trimmed_desc = if desc.len() > 40 {
                    format!("{}...", &desc[..37])
                } else {
                    desc.to_string()
                };
                let time_built_epoch = branch
                    .get("timebuildupdated")
                    .or_else(|| branch.get("timeupdated"))
                    .and_then(|t| t.as_str())
                    .and_then(|s| s.parse::<u64>().ok())
                    .unwrap_or(0);
                let time_updated_epoch = branch
                    .get("timeupdated")
                    .and_then(|t| t.as_str())
                    .and_then(|s| s.parse::<u64>().ok())
                    .unwrap_or(0);
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
                let pwd = branch.get("pwdrequired").and_then(|p| p.as_str()) == Some("1");
                let mut name_str = bname.clone();
                if pwd {
                    name_str.push_str(" [password]");
                }
                branch_entries.push((
                    time_updated_epoch,
                    [name_str, trimmed_desc, build_id, time_built, time_updated],
                ));
            }

            // Sort by most recently updated first
            branch_entries.sort_by(|a, b| b.0.cmp(&a.0));
            let branch_rows: Vec<[String; 5]> =
                branch_entries.into_iter().map(|(_, r)| r).collect();

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
    }

    Ok(())
}
