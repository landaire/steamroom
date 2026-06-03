use clap::Parser;
use clap::Subcommand;
use clap::ValueEnum;

#[derive(Parser, Debug)]
#[command(
    name = "steamroom",
    about = "Steam depot downloader",
    after_help = "Set DD_COMPAT=1 for flat-argument compatibility with the original DepotDownloader."
)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Command,

    #[command(flatten)]
    pub auth: AuthOptions,

    /// Enable debug logging
    #[arg(long)]
    pub debug: bool,

    /// Show full error chains on failure
    #[arg(long)]
    pub raw_errors: bool,

    /// Steam CDN cell ID to prefer
    #[arg(long)]
    pub cell_id: Option<u32>,

    /// Capture network traffic to a file for replay
    #[arg(long)]
    pub capture: Option<std::path::PathBuf>,

    /// Disable progress bars
    #[arg(long)]
    pub no_progress: bool,

    /// Suppress all output except errors
    #[arg(short, long)]
    pub quiet: bool,

    /// Never prompt; fail loudly if interactive auth is required. Also
    /// implied automatically when stdin is not a TTY.
    #[arg(long, env = "STEAMROOM_NON_INTERACTIVE")]
    pub non_interactive: bool,

    /// Send this command to the running daemon instead of executing
    /// directly. Pair with any subcommand. Start a daemon with
    /// `steamroom daemon start`.
    #[arg(long = "use-daemon")]
    pub use_daemon: bool,

    /// Resume daemon execution after fork+exec. Set by the parent
    /// during `daemon start`; not intended to be passed by the user.
    #[arg(long, hide = true)]
    pub daemon_resume: Option<String>,

    /// Push this request to the front of the daemon queue.
    /// Only valid with --use-daemon.
    #[arg(long)]
    pub priority: bool,

    /// Return immediately after the daemon accepts the job; do not
    /// stream progress. Only valid with --use-daemon.
    #[arg(long)]
    pub detach: bool,
}

/// Legacy flat-argument CLI compatible with the original DepotDownloader.
/// Activated with DD_COMPAT=1 environment variable.
///
/// DepotDownloader uses single-dash flags (`-app`, `-depot`, etc.).
/// The arg preprocessor in main() converts these to double-dash before parsing.
#[derive(Parser, Debug)]
#[command(name = "steamroom", about = "Steam depot downloader (DD_COMPAT mode)")]
pub struct CompatCli {
    #[arg(long = "app")]
    pub app_id: Option<u32>,
    #[arg(long = "depot")]
    pub depot_id: Option<u32>,
    #[arg(long = "manifest")]
    pub manifest_id: Option<u64>,
    #[arg(long = "username")]
    pub username: Option<String>,
    #[arg(long = "password")]
    pub password: Option<String>,
    #[arg(long = "dir")]
    pub output: Option<std::path::PathBuf>,
    #[arg(long = "branch")]
    pub branch: Option<String>,
    #[arg(long = "betapassword")]
    pub beta_password: Option<String>,
    #[arg(long)]
    pub qr: bool,
    #[arg(long = "remember-password")]
    pub remember_password: bool,
    #[arg(long = "filelist")]
    pub filelist: Option<std::path::PathBuf>,
    #[arg(long = "regex")]
    pub file_regex: Option<String>,
    #[arg(long = "validate")]
    pub verify: bool,
    #[arg(long)]
    pub os: Option<String>,
    #[arg(long)]
    pub arch: Option<String>,
    #[arg(long)]
    pub language: Option<String>,
    #[arg(long = "max-downloads")]
    pub max_downloads: Option<usize>,
    #[arg(long = "cellid")]
    pub cell_id: Option<u32>,
    #[arg(long)]
    pub debug: bool,
    #[arg(long = "device-name", env = "DD_DEVICE_NAME")]
    pub device_name: Option<String>,
}

impl CompatCli {
    pub fn into_cli(self) -> Cli {
        let app = self.app_id.unwrap_or(0);
        Cli {
            command: Command::Download(DownloadArgs {
                app,
                depot: self.depot_id,
                manifest: self.manifest_id,
                filelist: self.filelist,
                file_regex: self.file_regex,
                output: self.output,
                verify: self.verify,
                os: self.os,
                arch: self.arch,
                language: self.language,
                login_id: None,
                all_platforms: false,
                all_architectures: false,
                all_languages: false,
                lancache: false,
                max_downloads: self.max_downloads,
                branch: self.branch,
                branch_password: self.beta_password,
                local_keys: false,
                non_atomic: false,
                save_manifests: false,
                capture: None,
                bytes: false,
            }),
            auth: AuthOptions {
                username: self.username,
                password: self.password,
                qr: self.qr,
                use_steam_token: false,
                remember_password: self.remember_password,
                device_name: self.device_name,
            },
            debug: self.debug,
            raw_errors: false,
            cell_id: self.cell_id,
            capture: None,
            no_progress: false,
            quiet: false,
            non_interactive: false,
            use_daemon: false,
            daemon_resume: None,
            priority: false,
            detach: false,
        }
    }
}

#[derive(Parser, Debug)]
pub struct AuthOptions {
    /// Steam username (or set STEAM_USER)
    #[arg(short, long, env = "STEAM_USER")]
    pub username: Option<String>,

    /// Steam password (or set STEAM_PASS)
    #[arg(short, long, env = "STEAM_PASS")]
    pub password: Option<String>,

    /// Login via QR code (scan with Steam mobile app)
    #[arg(long)]
    pub qr: bool,

    /// Use cached token from local Steam installation
    #[arg(long)]
    pub use_steam_token: bool,

    /// Save login token for future use
    #[arg(long)]
    pub remember_password: bool,

    /// Device name for Steam Guard (or set DD_DEVICE_NAME)
    #[arg(long, env = "DD_DEVICE_NAME")]
    pub device_name: Option<String>,
}

#[derive(Subcommand, Debug)]
pub enum Command {
    /// Daemon control: start a background daemon, then stop, observe,
    /// or attach to it.
    Daemon(DaemonArgs),
    /// Compare two manifests and show added, removed, and changed files
    Diff(DiffArgs),
    /// Download depot content to a local directory
    Download(DownloadArgs),
    /// List files in a depot manifest
    Files(FilesArgs),
    /// Show app metadata: name, type, depots, branches
    Info(InfoArgs),
    /// Show locally cached depot keys and beta branches from Steam's config.vdf
    LocalInfo(LocalInfoArgs),
    /// List depot manifest IDs for a branch
    Manifests(ManifestsArgs),
    /// Query Steam package (sub) details by ID
    Packages(PackagesArgs),
    /// Download and save a depot manifest without downloading content
    SaveManifest(SaveManifestArgs),
    /// Download a Steam Workshop item
    Workshop(WorkshopArgs),
}

#[derive(Parser, Debug)]
pub struct DaemonArgs {
    #[command(subcommand)]
    pub command: DaemonSub,
}

#[derive(Subcommand, Debug)]
pub enum DaemonSub {
    /// Authenticate (interactively if needed), fork into the background,
    /// and serve RPC. Auth flags (`--username`, `--qr`, etc.) go on the
    /// top-level command, e.g. `steamroom --username foo daemon start`.
    Start,
    /// Stop the running daemon.
    Stop {
        /// Cancel the active job immediately instead of waiting for it.
        #[arg(long)]
        force: bool,
    },
    /// Print queue, active job, and recent history. Default: TUI dashboard.
    Status {
        /// Print a one-shot text snapshot to stdout instead of opening
        /// the TUI.
        #[arg(long)]
        text: bool,
        /// Output format. Anything other than the default implies
        /// `--text` (JSON in a TUI is not meaningful).
        #[arg(long, value_enum)]
        format: Option<OutputFormat>,
    },
    /// Print the daemon's PID, socket path, and stop command. Does not
    /// contact the daemon, so it works even when the daemon is wedged.
    Info,
    /// Reconnect to an in-flight or recently-finished job by its ID and
    /// stream its events to this terminal. Use after `--use-daemon
    /// --detach <job>` to come back and watch progress, or to resume an
    /// attach session you exited with Ctrl-C. If the job has finished,
    /// `attach` writes its exit code from the daemon's recent-jobs ring
    /// and exits. If the ID is unknown, errors with `JobNotFound`.
    Attach {
        /// The job ID printed by `--use-daemon --detach` or shown in
        /// `daemon status`.
        job_id: u64,
    },
}

#[derive(Parser, Debug)]
pub struct DownloadArgs {
    /// Steam app ID
    #[arg(long)]
    pub app: u32,
    /// Depot ID (auto-detected if omitted)
    #[arg(long)]
    pub depot: Option<u32>,
    /// Manifest ID (uses latest for branch if omitted)
    #[arg(long)]
    pub manifest: Option<u64>,
    /// File containing paths to download (one per line, prefix with regex: for patterns)
    #[arg(long)]
    pub filelist: Option<std::path::PathBuf>,
    /// Regex pattern to filter files
    #[arg(long)]
    pub file_regex: Option<String>,
    /// Output directory
    #[arg(long, short)]
    pub output: Option<std::path::PathBuf>,
    /// Skip files that already match the manifest
    #[arg(long)]
    pub verify: bool,
    /// Filter depots by OS (e.g. windows, linux, macos)
    #[arg(long)]
    pub os: Option<String>,
    /// Filter depots by architecture (e.g. 32, 64)
    #[arg(long)]
    pub arch: Option<String>,
    /// Filter depots by language
    #[arg(long)]
    pub language: Option<String>,
    /// Login ID for concurrent sessions
    #[arg(long)]
    pub login_id: Option<u32>,
    /// Download all platform depots
    #[arg(long)]
    pub all_platforms: bool,
    /// Download all architecture depots
    #[arg(long)]
    pub all_architectures: bool,
    /// Download all language depots
    #[arg(long)]
    pub all_languages: bool,
    /// Use lancache-compatible CDN requests
    #[arg(long)]
    pub lancache: bool,
    /// Maximum concurrent chunk downloads
    #[arg(long)]
    pub max_downloads: Option<usize>,
    /// Branch to download (default: public)
    #[arg(long)]
    pub branch: Option<String>,
    /// Password for beta branch access
    #[arg(long)]
    pub branch_password: Option<String>,
    /// Use depot decryption keys from Steam's local config.vdf instead of requesting from server
    #[arg(long)]
    pub local_keys: bool,
    /// Write chunks directly to target files instead of staging + rename
    #[arg(long)]
    pub non_atomic: bool,
    /// Save raw and decompressed manifests alongside downloaded files
    #[arg(long)]
    pub save_manifests: bool,
    /// Capture network traffic to a file
    #[arg(long)]
    pub capture: Option<std::path::PathBuf>,
    /// Show file sizes in raw bytes
    #[arg(long)]
    pub bytes: bool,
}

#[derive(Parser, Debug)]
pub struct FilesArgs {
    /// Steam app ID (not needed with --manifest-file)
    #[arg(long)]
    pub app: Option<u32>,
    /// Depot ID (auto-detected if omitted)
    #[arg(long)]
    pub depot: Option<u32>,
    /// Manifest ID (uses latest for branch if omitted)
    #[arg(long)]
    pub manifest: Option<u64>,
    /// Read from a local manifest file instead of fetching from CDN
    #[arg(long, value_name = "PATH")]
    pub manifest_file: Option<std::path::PathBuf>,
    /// Depot key for filename decryption (hex). Auto-detected from depot.json if available
    #[arg(long, value_name = "HEX")]
    pub depot_key: Option<String>,
    /// Branch to list files for (default: public)
    #[arg(long)]
    pub branch: Option<String>,
    /// Password for beta branch access
    #[arg(long)]
    pub branch_password: Option<String>,
    /// Filter depots by OS
    #[arg(long)]
    pub os: Option<String>,
    /// Output format
    #[arg(long, value_enum)]
    pub format: Option<OutputFormat>,
    /// Show raw encrypted filenames
    #[arg(long)]
    pub raw: bool,
    /// Show file sizes in raw bytes
    #[arg(long)]
    pub bytes: bool,
}

#[derive(Parser, Debug)]
pub struct LocalInfoArgs {
    /// Output format
    #[arg(long, value_enum)]
    pub format: Option<OutputFormat>,
    /// Show info for a specific Steam user
    #[arg(long)]
    pub user: Option<String>,
    /// List all local Steam users
    #[arg(long)]
    pub users: bool,
}

#[derive(Parser, Debug)]
pub struct SaveManifestArgs {
    /// Steam app ID
    #[arg(long)]
    pub app: u32,
    /// Depot ID
    #[arg(long)]
    pub depot: u32,
    /// Manifest ID (uses latest for branch if omitted)
    #[arg(long)]
    pub manifest: Option<u64>,
    /// Branch (default: public)
    #[arg(long)]
    pub branch: Option<String>,
    /// Output directory for saved manifests
    #[arg(long, short)]
    pub output: std::path::PathBuf,
}

#[derive(Parser, Debug)]
pub struct InfoArgs {
    /// Steam app ID
    #[arg(long)]
    pub app: u32,
    /// Output format
    #[arg(long, value_enum)]
    pub format: Option<OutputFormat>,
    /// Filter depots by OS (e.g. windows, linux, macos)
    #[arg(long)]
    pub os: Option<String>,
    /// Show redistributable depots
    #[arg(long)]
    pub show_all: bool,
}

#[derive(Parser, Debug)]
pub struct ManifestsArgs {
    /// Steam app ID
    #[arg(long)]
    pub app: u32,
    /// Branch to list manifests for (default: public)
    #[arg(long)]
    pub branch: Option<String>,
    /// Password for beta branch access
    #[arg(long)]
    pub branch_password: Option<String>,
    /// Output format
    #[arg(long, value_enum)]
    pub format: Option<OutputFormat>,
}

#[derive(Parser, Debug)]
pub struct WorkshopArgs {
    /// Steam app ID
    #[arg(long)]
    pub app: u32,
    /// Workshop item ID
    #[arg(long)]
    pub item: u64,
    /// Output directory
    #[arg(long, short)]
    pub output: Option<std::path::PathBuf>,
}

#[derive(Parser, Debug)]
pub struct DiffArgs {
    /// Steam app ID
    #[arg(long)]
    pub app: u32,
    /// Depot ID
    #[arg(long)]
    pub depot: u32,
    /// Old manifest ID
    #[arg(long)]
    pub from: u64,
    /// New manifest ID
    #[arg(long)]
    pub to: u64,
    /// Branch (used for manifest request codes)
    #[arg(long)]
    pub branch: Option<String>,
    /// Output format
    #[arg(long, value_enum)]
    pub format: Option<OutputFormat>,
}

#[derive(Parser, Debug)]
pub struct PackagesArgs {
    /// Package (sub) IDs to query
    #[arg(long = "package", required = true, num_args = 1..)]
    pub packages: Vec<u32>,
    /// Output format
    #[arg(long, value_enum)]
    pub format: Option<OutputFormat>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
pub enum OutputFormat {
    Table,
    Json,
    Plain,
}

use crate::daemon::proto::Request;
use crate::errors::CliError;

impl Cli {
    /// Lower a parsed `Cli` into the wire-typed `Request`. Flags that
    /// the daemon cannot honor (auth flags, `--capture`) are warned
    /// about on stderr and ignored; only structural mistakes (a daemon
    /// control subcommand) are returned as errors.
    pub fn into_rpc_request(self) -> Result<Request, CliError> {
        let auth = &self.auth;
        let has_auth = auth.username.is_some()
            || auth.password.is_some()
            || auth.qr
            || auth.use_steam_token
            || auth.remember_password
            || auth.device_name.is_some();
        if has_auth {
            eprintln!(
                "warning: auth flags are ignored under --use-daemon; \
                 the daemon serves the account it was started with"
            );
        }
        if self.capture.is_some() {
            eprintln!("warning: --capture is ignored under --use-daemon");
        }

        let priority = self.priority;
        match self.command {
            Command::Download(a) => {
                if a.capture.is_some() {
                    eprintln!("warning: --capture is ignored under --use-daemon");
                }
                Ok(Request::Download {
                    args: crate::daemon::proto::DownloadParams::from(a),
                    priority,
                })
            }
            Command::Info(a) => Ok(Request::Info {
                args: crate::daemon::proto::InfoParams::from(a),
                priority,
            }),
            Command::Files(a) => Ok(Request::Files {
                args: crate::daemon::proto::FilesParams::from(a),
                priority,
            }),
            Command::Manifests(a) => Ok(Request::Manifests {
                args: crate::daemon::proto::ManifestsParams::from(a),
                priority,
            }),
            Command::Diff(a) => Ok(Request::Diff {
                args: crate::daemon::proto::DiffParams::from(a),
                priority,
            }),
            Command::Packages(a) => Ok(Request::Packages {
                args: crate::daemon::proto::PackagesParams::from(a),
                priority,
            }),
            Command::SaveManifest(a) => Ok(Request::SaveManifest {
                args: crate::daemon::proto::SaveManifestParams::from(a),
                priority,
            }),
            Command::Workshop(a) => Ok(Request::Workshop {
                args: crate::daemon::proto::WorkshopParams::from(a),
                priority,
            }),
            Command::LocalInfo(a) => Ok(Request::LocalInfo {
                args: crate::daemon::proto::LocalInfoParams::from(a),
                priority,
            }),
            Command::Daemon(_) => Err(CliError::DaemonRejectedFlag("daemon subcommand")),
        }
    }

    /// Belt-and-suspenders flag validation that clap can't express.
    pub fn validate(&self) -> Result<(), CliError> {
        if self.priority && !self.use_daemon {
            return Err(CliError::PriorityWithoutDaemon);
        }
        if self.detach && !self.use_daemon {
            return Err(CliError::DetachWithoutDaemon);
        }
        // `daemon start` and `--use-daemon` together are nonsensical:
        // start launches a daemon, --use-daemon talks to one.
        if self.use_daemon
            && matches!(self.command, Command::Daemon(DaemonArgs { command: DaemonSub::Start }))
        {
            return Err(CliError::DaemonModeConflict);
        }
        Ok(())
    }
}
