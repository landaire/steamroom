//! `steamroom local-info`: dump locally cached depot keys, beta hashes,
//! and active Steam user from `config.vdf`. Does not connect to Steam.

use crate::cli::LocalInfoArgs;
use crate::cli::OutputFormat;
use crate::errors::CliError;
use crate::sink::JobSink;
use tabled::builder::Builder as TableBuilder;
use tabled::settings::Style;
use tokio_util::sync::CancellationToken;
use tracing::info;

pub async fn run_local_info(
    args: LocalInfoArgs,
    sink: &dyn JobSink,
    _cancel: CancellationToken,
) -> Result<(), CliError> {
    let steam_dir = steamroom_client::steam_creds::steam_dir().ok_or(CliError::SteamNotFound)?;

    if args.users {
        let users = steamroom_client::steam_creds::list_users(&steam_dir);
        if args.format == Some(OutputFormat::Json) {
            let json: Vec<serde_json::Value> = users
                .iter()
                .map(|u| {
                    serde_json::json!({
                        "steam_id": u.steam_id,
                        "account_name": u.account_name,
                        "persona_name": u.persona_name,
                        "most_recent": u.most_recent,
                    })
                })
                .collect();
            sink.stdout_line(&serde_json::to_string_pretty(&json)?);
        } else {
            for u in &users {
                let active = if u.most_recent { " (active)" } else { "" };
                sink.stdout_line(&format!("{}{active}", u.account_name));
            }
        }
        return Ok(());
    }

    info!("reading {}", steam_dir.join("config/config.vdf").display());

    let config =
        steamroom_client::steam_creds::read_config(&steam_dir).ok_or(CliError::SteamNotFound)?;

    let apps = steamroom_client::steam_creds::scan_installed_apps(&steam_dir);
    let mut depot_to_app: std::collections::HashMap<u32, (u32, &str)> =
        std::collections::HashMap::new();
    let mut app_names: std::collections::HashMap<u32, &str> = std::collections::HashMap::new();
    for app in &apps {
        app_names.insert(app.app_id, &app.name);
        for &did in &app.depot_ids {
            depot_to_app.insert(did, (app.app_id, &app.name));
        }
    }

    if args.format == Some(OutputFormat::Json) {
        let depot_keys: Vec<serde_json::Value> = config
            .depot_keys
            .iter()
            .map(|dk| {
                let (app_id, app_name) =
                    depot_to_app.get(&dk.depot_id.0).copied().unwrap_or((0, ""));
                serde_json::json!({
                    "depot_id": dk.depot_id.0,
                    "app_id": app_id,
                    "app_name": app_name,
                    "key": steamroom::util::hex::encode(&dk.key.0),
                })
            })
            .collect();
        let beta_hashes: Vec<serde_json::Value> = config
            .beta_hashes
            .iter()
            .map(|bh| {
                serde_json::json!({
                    "app_id": bh.app_id,
                    "app_name": app_names.get(&bh.app_id).copied().unwrap_or(""),
                    "branch": bh.branch,
                })
            })
            .collect();
        let username = args
            .user
            .clone()
            .or_else(|| steamroom_client::steam_creds::detect_username(&steam_dir));
        let has_token = username
            .as_deref()
            .and_then(steamroom_client::steam_creds::extract_token)
            .is_some();
        let output = serde_json::json!({
            "steam_dir": steam_dir.to_string_lossy(),
            "user": username,
            "has_cached_token": has_token,
            "depot_keys": depot_keys,
            "beta_hashes": beta_hashes,
        });
        sink.stdout_line(&serde_json::to_string_pretty(&output)?);
        return Ok(());
    }

    let display_user = args
        .user
        .clone()
        .or_else(|| steamroom_client::steam_creds::detect_username(&steam_dir));

    sink.stdout_line(&format!("Steam directory: {}", steam_dir.display()));
    if let Some(ref username) = display_user {
        let has_token = steamroom_client::steam_creds::extract_token(username).is_some();
        sink.stdout_line(&format!("User:            {username}"));
        sink.stdout_line(&format!(
            "Cached token:    {}",
            if has_token { "yes" } else { "no" }
        ));
    }
    sink.stdout_line("");

    if !config.depot_keys.is_empty() {
        sink.stdout_line(&format!(
            "Cached depot decryption keys ({}):",
            config.depot_keys.len()
        ));
        sink.stdout_line("");
        let mut sorted_keys: Vec<_> = config
            .depot_keys
            .iter()
            .map(|dk| {
                let (app_id, app_name) =
                    depot_to_app.get(&dk.depot_id.0).copied().unwrap_or((0, ""));
                (app_name, app_id, dk)
            })
            .collect();
        sorted_keys.sort_by(|a, b| {
            a.0.cmp(b.0)
                .then(a.1.cmp(&b.1))
                .then(a.2.depot_id.0.cmp(&b.2.depot_id.0))
        });

        let mut table = TableBuilder::default();
        table.push_record(["DEPOT", "APP", "NAME", "KEY"]);
        for (app_name, app_id, dk) in &sorted_keys {
            table.push_record([
                dk.depot_id.0.to_string(),
                if *app_id > 0 {
                    app_id.to_string()
                } else {
                    String::new()
                },
                app_name.to_string(),
                steamroom::util::hex::encode(&dk.key.0),
            ]);
        }
        let rendered = table
            .build()
            .with(Style::blank())
            .with(tabled::settings::Padding::new(0, 2, 0, 0))
            .to_string();
        for line in rendered.lines() {
            sink.stdout_line(line);
        }
        sink.stdout_line("");
    }

    if !config.beta_hashes.is_empty() {
        sink.stdout_line("Cached beta branch passwords:");
        sink.stdout_line("");
        let mut table = TableBuilder::default();
        table.push_record(["APP", "NAME", "BRANCH"]);
        for bh in &config.beta_hashes {
            let name = app_names.get(&bh.app_id).copied().unwrap_or("");
            table.push_record([bh.app_id.to_string(), name.to_string(), bh.branch.clone()]);
        }
        let rendered = table
            .build()
            .with(Style::blank())
            .with(tabled::settings::Padding::new(0, 2, 0, 0))
            .to_string();
        for line in rendered.lines() {
            sink.stdout_line(line);
        }
        sink.stdout_line("");
    }

    Ok(())
}
