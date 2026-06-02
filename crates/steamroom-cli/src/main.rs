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
    // The daemon-resume child installs its own subscriber inside serve_resumed
    // (which adds the JobScopedLogLayer). Installing a global fmt subscriber
    // here first would cause serve_resumed's try_init() to silently no-op,
    // leaving JobScopedLogLayer uninstalled.
    let is_daemon_resume = cli.daemon_resume.is_some();
    if !is_daemon_resume {
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
    }

    if let Err(err) = cli.validate() {
        eprintln!("Error: {err}");
        std::process::exit(2);
    }

    // Daemon resume path: this process IS the long-lived daemon worker.
    // It must build its own runtime and NEVER fork.
    if let Some(user) = cli.daemon_resume.clone() {
        let rt = build_runtime();
        // Daemon is non-interactive; no TTY in the resumed child.
        commands::shared::init_interactive(false);
        let result = rt.block_on(async { daemon::lifecycle::serve_resumed(user, cli).await });
        report_and_exit(result, false);
    }

    let interactive =
        !cli.non_interactive && std::io::IsTerminal::is_terminal(&std::io::stdin());

    // --daemon: authenticate in the foreground (tokio is allowed here),
    // then drop the runtime fully before forking.
    if cli.daemon {
        commands::shared::init_interactive(interactive);
        let rt = build_runtime();
        let auth_result =
            rt.block_on(async { daemon::lifecycle::launch_daemon_authenticate(&cli).await });
        // Critically: drop the runtime BEFORE forking. Tokio worker
        // threads holding glibc locks at fork() would deadlock the
        // post-fork process.
        drop(rt);
        let username = match auth_result {
            Ok(u) => u,
            Err(e) => {
                eprintln!("Error: {e}");
                std::process::exit(1);
            }
        };
        match daemon::lifecycle::detach_and_exec_resume(&username, &daemon::lifecycle::log_path())
        {
            Ok(()) => std::process::exit(0),
            Err(e) => {
                eprintln!("Error: {e}");
                std::process::exit(1);
            }
        }
    }

    // --use-daemon: connect to daemon, ship the Request, attach.
    if cli.use_daemon {
        commands::shared::init_interactive(interactive);
        let rt = build_runtime();
        let result = rt.block_on(async { daemon::client::dispatch_use_daemon(cli).await });
        report_and_exit(result, false);
    }

    // Direct mode (existing path).
    commands::shared::init_interactive(interactive);
    let rt = build_runtime();
    let raw_errors = cli.raw_errors;
    let result = rt.block_on(async_main(cli));
    report_and_exit(result, raw_errors);
}

fn build_runtime() -> tokio::runtime::Runtime {
    let cpus = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4);
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .max_blocking_threads(cpus)
        .build()
        .expect("failed to build tokio runtime")
}

fn report_and_exit(result: Result<(), CliError>, raw_errors: bool) -> ! {
    if let Err(err) = result {
        if raw_errors {
            let report: rootcause::Report<CliError> = rootcause::report!(err);
            eprintln!("Error: {report:?}");
        } else {
            eprintln!("Error: {err}");
        }
        std::process::exit(1);
    }
    std::process::exit(0);
}

async fn async_main(cli: Cli) -> Result<(), CliError> {
    use std::sync::Arc;
    let show_progress = !cli.no_progress;
    let sink: Arc<dyn sink::JobSink> = Arc::new(StdoutSink::new());
    let cancel = CancellationToken::new();

    match cli.command {
        Command::LocalInfo(args) => {
            // No Steam connection required.
            commands::local_info::run_local_info(args, sink, cancel).await
        }
        Command::Files(args) => {
            // --manifest-file path needs no client; only fetch when we have to.
            let client = if args.manifest_file.is_none() {
                Some(commands::shared::connect_and_login(&cli.auth).await?)
            } else {
                None
            };
            commands::files::run_files(args, client, sink, cancel).await
        }
        Command::Info(args) => {
            let client = commands::shared::connect_and_login(&cli.auth).await?;
            commands::info::run_info(args, client, sink, cancel).await
        }
        Command::Manifests(args) => {
            let client = commands::shared::connect_and_login(&cli.auth).await?;
            commands::manifests::run_manifests(args, client, sink, cancel).await
        }
        Command::Diff(args) => {
            let client = commands::shared::connect_and_login(&cli.auth).await?;
            commands::diff::run_diff(args, client, sink, cancel).await
        }
        Command::Packages(args) => {
            let client = commands::shared::connect_and_login(&cli.auth).await?;
            commands::packages::run_packages(args, client, sink, cancel).await
        }
        Command::SaveManifest(args) => {
            let client = commands::shared::connect_and_login(&cli.auth).await?;
            commands::save_manifest::run_save_manifest(args, client, sink, cancel).await
        }
        Command::Download(args) => {
            let client = commands::shared::connect_and_login(&cli.auth).await?;
            commands::download::run_download(args, client, sink, cancel, show_progress).await
        }
        Command::Workshop(args) => {
            let client = commands::shared::connect_and_login(&cli.auth).await?;
            commands::workshop::run_workshop(args, client, sink, cancel, show_progress).await
        }
        Command::Daemon(args) => daemon::client::run_daemon_subcommand(args.command).await,
    }
}
