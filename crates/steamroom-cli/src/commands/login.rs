use std::io::Write;

use steamroom_client::credentials::StartupAuth;
use steamroom_client::login::LoginBuilder;

use crate::cli::AuthOptions;
use crate::cli::LoginArgs;
use crate::commands::shared;
use crate::errors::CliError;

pub async fn run_login(_args: LoginArgs, mut auth: AuthOptions) -> Result<(), CliError> {
    let prompt_for_options = !auth.has_explicit_method();
    if prompt_for_options {
        require_interactive()?;
        auth.use_steam_token =
            prompt_yes_no("Use the login token from the local Steam installation? [y/N]")?;
    }

    if auth.use_steam_token {
        let builder = LoginBuilder::new()
            .device_name(auth.device_name.as_deref().unwrap_or("steamroom"));
        let (client, account_name) =
            shared::login_with_steam_token(builder, auth.username.as_deref()).await?;
        drop(client);

        shared::set_startup_auth(Some(StartupAuth::SteamToken {
            account_name: Some(account_name),
        }))?;
        println!("Login successful. Steam login token will be used on startup.");
        return Ok(());
    }

    let username = match auth.username.take() {
        Some(username) => username,
        None => prompt_username()?,
    };
    auth.username = Some(username.clone());

    if prompt_for_options && !auth.remember_password {
        auth.remember_password = prompt_yes_no(
            "Save a Steam refresh token for future logins? Your password is not stored. [y/N]",
        )?;
    }

    let builder = LoginBuilder::new()
        .device_name(auth.device_name.as_deref().unwrap_or("steamroom"));
    let client = if auth.qr {
        require_interactive()?;
        shared::drive_qr_flow(builder, &username, auth.remember_password).await?
    } else {
        shared::drive_credentials_flow(builder, &username, &auth).await?
    };
    drop(client);

    shared::set_startup_auth(None)?;
    println!("Login successful.");
    Ok(())
}

fn require_interactive() -> Result<(), CliError> {
    if !shared::is_interactive() {
        return Err(CliError::InteractiveAuthRequired);
    }

    Ok(())
}

fn prompt_username() -> Result<String, CliError> {
    require_interactive()?;
    let username = prompt_line("Steam username:")?;
    if username.is_empty() {
        return Err(CliError::UsernameRequired);
    }

    Ok(username)
}

fn prompt_yes_no(prompt: &str) -> Result<bool, CliError> {
    loop {
        let answer = prompt_line(prompt)?;
        if let Some(answer) = parse_yes_no(&answer) {
            return Ok(answer);
        }
        eprintln!("Enter yes or no.");
    }
}

fn prompt_line(prompt: &str) -> Result<String, CliError> {
    eprintln!("{prompt}");
    eprint!("> ");
    std::io::stderr().flush()?;
    let mut input = String::new();
    let bytes_read = std::io::stdin().read_line(&mut input)?;
    if bytes_read == 0 {
        return Err(CliError::InteractiveAuthRequired);
    }

    Ok(input.trim().to_string())
}

fn parse_yes_no(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "" | "n" | "no" => Some(false),
        "y" | "yes" => Some(true),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_confirmation_answers() {
        assert_eq!(parse_yes_no(""), Some(false));
        assert_eq!(parse_yes_no("YES"), Some(true));
        assert_eq!(parse_yes_no("n"), Some(false));
        assert_eq!(parse_yes_no("maybe"), None);
    }
}
