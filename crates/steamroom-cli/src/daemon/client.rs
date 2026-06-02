//! Client side of `--use-daemon`. Real implementation in T19.

use crate::cli::Cli;
use crate::errors::CliError;

pub async fn dispatch_use_daemon(_cli: Cli) -> Result<(), CliError> {
    Err(CliError::Io(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "--use-daemon dispatcher will be implemented in task 19",
    )))
}
