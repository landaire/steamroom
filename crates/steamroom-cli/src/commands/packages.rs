//! `steamroom packages`: query Steam package (sub) details by ID.

use crate::cli::OutputFormat;
use crate::cli::PackagesArgs;
use crate::commands::shared::kv_to_json;
use crate::commands::shared::parse_package_kv;
use crate::errors::CliError;
use crate::sink::JobSink;
use std::sync::Arc;
use steamroom::client::LoggedIn;
use steamroom::client::SteamClient;
use steamroom::depot::*;
use steamroom::types::key_value::KeyValue;
use steamroom::types::key_value::KvValue;
use tabled::builder::Builder as TableBuilder;
use tabled::settings::Style;
use tokio_util::sync::CancellationToken;
use tracing::info;

pub async fn run_packages(
    args: PackagesArgs,
    client: SteamClient<LoggedIn>,
    sink: Arc<dyn JobSink>,
    _cancel: CancellationToken,
) -> Result<(), CliError> {
    let ids: Vec<PackageId> = args.packages.iter().map(|&id| PackageId(id)).collect();
    info!("querying {} package(s)...", ids.len());
    let packages = client.pics_get_package_info(&ids).await?;

    for pkg in &packages {
        let pkg_id = pkg.package_id.map(|p| p.0).unwrap_or(0);

        let kv = pkg
            .kv_data
            .as_ref()
            .and_then(|data| parse_package_kv(data).ok());

        if args.format == Some(OutputFormat::Json) {
            let json = if let Some(ref kv) = kv {
                kv_to_json(kv)
            } else {
                serde_json::json!({"packageid": pkg_id})
            };
            sink.stdout_line(&serde_json::to_string_pretty(&json)?);
            continue;
        }

        let Some(ref kv) = kv else {
            sink.stdout_line(&format!("Package: {pkg_id}"));
            sink.stdout_line("  (no data)");
            sink.stdout_line("");
            continue;
        };

        // Helper to get a field as string regardless of KV type
        let get_str = |kv: &KeyValue, key: &str| -> Option<String> {
            let v = kv.get(key)?;
            v.as_str()
                .map(|s| s.to_string())
                .or_else(|| v.as_i32().map(|i| i.to_string()))
                .or_else(|| v.as_u64().map(|i| i.to_string()))
        };

        let name = get_str(kv, "name");
        let billing_type = get_str(kv, "billingtype")
            .and_then(|v| v.parse::<i32>().ok())
            .map(steamroom::enums::EBillingType::from_i32);
        let license_type = get_str(kv, "licensetype")
            .and_then(|v| v.parse::<i32>().ok())
            .map(steamroom::enums::ELicenseType::from_i32);
        let status = get_str(kv, "status")
            .and_then(|v| v.parse::<i32>().ok())
            .map(steamroom::enums::EPackageStatus::from_i32);
        let pkg_id_str = get_str(kv, "packageid").unwrap_or_else(|| pkg_id.to_string());

        sink.stdout_line(&format!("Package: {pkg_id_str}"));
        if let Some(ref name) = name {
            sink.stdout_line(&format!("Name:    {name}"));
        }
        if let Some(bt) = billing_type {
            sink.stdout_line(&format!("Billing: {bt}"));
        }
        if let Some(lt) = license_type {
            sink.stdout_line(&format!("License: {lt}"));
        }
        if let Some(st) = status {
            sink.stdout_line(&format!("Status:  {st}"));
        }

        // App IDs included in this package
        let kv_children_to_strings = |kv: &KeyValue, key: &str| -> Vec<String> {
            let Some(node) = kv.get(key) else {
                return vec![];
            };
            let KvValue::Children(ref map) = node.value else {
                return vec![];
            };
            map.values()
                .filter_map(|v| {
                    v.as_str()
                        .map(|s| s.to_string())
                        .or_else(|| v.as_i32().map(|i| i.to_string()))
                        .or_else(|| v.as_u64().map(|i| i.to_string()))
                })
                .collect()
        };

        let app_ids = kv_children_to_strings(kv, "appids");
        if !app_ids.is_empty() {
            sink.stdout_line(&format!("Apps:    {}", app_ids.join(", ")));
        }

        let depot_ids = kv_children_to_strings(kv, "depotids");
        if !depot_ids.is_empty() {
            sink.stdout_line(&format!("Depots:  {}", depot_ids.join(", ")));
        }

        // Extended info
        if let Some(extended) = kv.get("extended")
            && let KvValue::Children(ref map) = extended.value
            && !map.is_empty()
        {
            sink.stdout_line("");
            let mut rows: Vec<[String; 2]> = Vec::new();
            for (k, v) in map {
                let val = v
                    .as_str()
                    .map(|s| s.to_string())
                    .or_else(|| v.as_i32().map(|i| i.to_string()))
                    .or_else(|| v.as_u64().map(|i| i.to_string()))
                    .unwrap_or_default();
                rows.push([k.clone(), val]);
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

        sink.stdout_line("");
    }

    Ok(())
}
