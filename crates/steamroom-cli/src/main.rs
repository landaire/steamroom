mod cli;
mod commands;
mod daemon;
mod download;
mod errors;
mod sink;

use clap::Parser;
use cli::*;
use errors::CliError;
use sink::StdoutSink;
use tokio_util::sync::CancellationToken;

fn main() {
    let cli = if std::env::var("DD_COMPAT").as_deref() == Ok("1") {
        // DepotDownloader uses single-dash flags (-app, -depot, etc.).
        // Clap expects double-dash, so convert before parsing.
        let args: Vec<String> = std::env::args()
            .map(|a| {
                if let Some(rest) = a.strip_prefix('-') {
                    if !rest.starts_with('-')
                        && rest.contains(|c: char| c.is_ascii_alphabetic())
                        && rest.len() > 1
                    {
                        format!("--{rest}")
                    } else {
                        a
                    }
                } else {
                    a
                }
            })
            .collect();
        cli::CompatCli::parse_from(args).into_cli()
    } else {
        Cli::parse()
    };
    let default_filter = if cli.quiet {
        "off"
    } else if cli.debug {
        "debug"
    } else if cfg!(debug_assertions) {
        "warn,steamroom=debug,steamroom_client=debug,steamroom_ffi=debug,steamroom_cli=debug"
    } else {
        "warn"
    };
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| default_filter.into()),
        )
        .init();

    let cpus = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .max_blocking_threads(cpus)
        .build()
        .expect("failed to build tokio runtime");

    let raw_errors = cli.raw_errors;
    commands::shared::init_interactive(
        !cli.non_interactive && std::io::IsTerminal::is_terminal(&std::io::stdin()),
    );
    if let Err(err) = rt.block_on(async_main(cli)) {
        if raw_errors {
            // Wrap in rootcause Report for full context chain
            let report: rootcause::Report<CliError> = rootcause::report!(err);
            eprintln!("Error: {report:?}");
        } else {
            eprintln!("Error: {err}");
        }
        std::process::exit(1);
    }
}

async fn async_main(cli: Cli) -> Result<(), CliError> {
    let show_progress = !cli.no_progress;
    let sink = StdoutSink::new(show_progress);
    let sink_ref: &dyn sink::JobSink = &sink;
    let cancel = CancellationToken::new();

    match cli.command {
        Command::LocalInfo(args) => {
            // No Steam connection required.
            commands::local_info::run_local_info(args, sink_ref, cancel).await
        }
        Command::Files(args) => {
            // --manifest-file path needs no client; only fetch when we have to.
            let client = if args.manifest_file.is_none() {
                Some(commands::shared::connect_and_login(&cli.auth).await?)
            } else {
                None
            };
            commands::files::run_files(args, client, sink_ref, cancel).await
        }
        Command::Info(args) => {
            let client = commands::shared::connect_and_login(&cli.auth).await?;
            commands::info::run_info(args, client, sink_ref, cancel).await
        }
        Command::Manifests(args) => {
            let client = commands::shared::connect_and_login(&cli.auth).await?;
            commands::manifests::run_manifests(args, client, sink_ref, cancel).await
        }
        Command::Diff(args) => {
            let client = commands::shared::connect_and_login(&cli.auth).await?;
            commands::diff::run_diff(args, client, sink_ref, cancel).await
        }
        Command::Packages(args) => {
            let client = commands::shared::connect_and_login(&cli.auth).await?;
            commands::packages::run_packages(args, client, sink_ref, cancel).await
        }
        Command::SaveManifest(args) => {
            let client = commands::shared::connect_and_login(&cli.auth).await?;
            commands::save_manifest::run_save_manifest(args, client, sink_ref, cancel).await
        }
        Command::Download(args) => {
            let client = commands::shared::connect_and_login(&cli.auth).await?;
            commands::download::run_download(args, client, sink_ref, cancel, show_progress).await
        }
        Command::Workshop(args) => {
            let client = commands::shared::connect_and_login(&cli.auth).await?;
            commands::workshop::run_workshop(args, client, sink_ref, cancel, show_progress).await
        }
    }
}
