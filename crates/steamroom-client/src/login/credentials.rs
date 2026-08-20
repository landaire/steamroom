use crate::login::BuilderConfig;
use crate::login::STEAM_CLIENT_PLATFORM_TYPE;
use crate::login::TransportConfig;
use crate::login::error::LoginError;
use crate::login::establish_ready_client;
use crate::login::terminal::ApprovedAuth;

use base64::Engine;
use steamroom::auth::AuthClientId;
use steamroom::auth::AuthTokens;
use steamroom::auth::GuardType;
use steamroom::auth::PollInterval;
use steamroom::client::Ready;
use steamroom::client::SteamClient;
use steamroom::generated::CAuthenticationBeginAuthSessionViaCredentialsRequest;
use steamroom::types::SteamId;

/// Configured credentials login. Call [`begin()`] to start the auth flow.
///
/// [`begin()`]: CredentialsLogin::begin
pub struct CredentialsLogin {
    pub(crate) config: BuilderConfig,
    pub(crate) transport: TransportConfig,
    pub(crate) account_name: String,
    pub(crate) password: String,
}

/// State of a credentials login after `begin()`.
#[non_exhaustive]
pub enum CredentialsLoginFlow {
    /// No 2FA required. Call `ApprovedAuth::finish()` to complete the logon.
    Approved(ApprovedAuth),
    /// 2FA required. The [`ConfirmationChallenge`] reports every method Steam
    /// will accept; when both a code and an out-of-band confirmation are
    /// offered the caller may drive them concurrently.
    NeedsConfirmation(ConfirmationChallenge),
}

/// A pending 2FA challenge. Steam may accept a Steam Guard code, an out-of-band
/// confirmation (approve in the mobile app or via an email link), or both at
/// once. `submit_code` and `wait_for_tokens` borrow `&self` so a caller can
/// race code entry against a confirmation poll and let whichever the user
/// completes first win.
pub struct ConfirmationChallenge {
    client: SteamClient<Ready>,
    config: BuilderConfig,
    client_id: AuthClientId,
    steam_id: SteamId,
    request_id: Vec<u8>,
    poll_interval: PollInterval,
    code_kinds: Vec<GuardType>,
    confirmation_kinds: Vec<GuardType>,
}

impl CredentialsLogin {
    /// Connect (or accept BYO client), fetch the RSA pubkey, RSA-encrypt the
    /// password, and call `BeginAuthSessionViaCredentials`. The returned flow
    /// indicates whether 2FA is needed and which kind.
    pub async fn begin(self) -> Result<CredentialsLoginFlow, LoginError> {
        let client = establish_ready_client(self.transport).await?;

        // RSA exchange
        let rsa = client
            .get_password_rsa_public_key(&self.account_name)
            .await?;
        let modulus = rsa
            .publickey_mod
            .ok_or(LoginError::MissingField("publickey_mod"))?;
        let exponent = rsa
            .publickey_exp
            .ok_or(LoginError::MissingField("publickey_exp"))?;
        let timestamp = rsa.timestamp.unwrap_or(0);

        let encrypted_password = steamroom::crypto::rsa::encrypt_with_rsa_public_key(
            self.password.as_bytes(),
            &modulus,
            &exponent,
        )
        .map_err(|e| LoginError::Transport(e.into()))?;
        let encoded_password =
            base64::engine::general_purpose::STANDARD.encode(&encrypted_password);

        // Begin auth session
        let req = CAuthenticationBeginAuthSessionViaCredentialsRequest {
            account_name: Some(self.account_name.clone()),
            encrypted_password: Some(encoded_password),
            encryption_timestamp: Some(timestamp),
            remember_login: Some(true),
            persistence: Some(1),
            platform_type: Some(STEAM_CLIENT_PLATFORM_TYPE),
            device_friendly_name: self.config.device_name.clone(),
            ..Default::default()
        };
        let session = match client.begin_auth_session_via_credentials(req).await {
            Ok(s) => s,
            Err(steamroom::Error::Connection(
                steamroom::error::ConnectionError::ServiceMethodFailed(
                    steamroom::enums::EResultError::InvalidPassword,
                ),
            )) => return Err(LoginError::InvalidPassword),
            Err(e) => return Err(LoginError::Transport(e)),
        };

        let client_id = session
            .client_id
            .ok_or(LoginError::MissingField("client_id"))?;
        let request_id = session
            .request_id
            .ok_or(LoginError::MissingField("request_id"))?;
        let steam_id = session
            .steam_id
            .ok_or(LoginError::MissingField("steamid"))?;
        let poll_interval = session.poll_interval.unwrap_or(PollInterval::DEFAULT);

        // Classify the next step.
        if session.allowed_confirmations.is_empty()
            || session.allowed_confirmations.contains(&GuardType::None)
        {
            // No 2FA needed — poll once for tokens (Steam may have them ready)
            // and produce ApprovedAuth directly.
            let tokens = poll_until_tokens(&client, client_id, &request_id, poll_interval).await?;
            return Ok(CredentialsLoginFlow::Approved(ApprovedAuth {
                client,
                config: self.config,
                tokens,
            }));
        }

        // Partition the offered methods. Steam commonly offers a code AND
        // mobile confirmation together; neither wins over the other here, both
        // are surfaced so the caller can drive whichever the user picks.
        let code_kinds: Vec<GuardType> = session
            .allowed_confirmations
            .iter()
            .copied()
            .filter(|g| g.is_code())
            .collect();
        let confirmation_kinds: Vec<GuardType> = session
            .allowed_confirmations
            .iter()
            .copied()
            .filter(|g| g.is_confirmation())
            .collect();

        if code_kinds.is_empty() && confirmation_kinds.is_empty() {
            // Only methods this client cannot drive (e.g. legacy machine auth).
            return Err(LoginError::NoSupportedConfirmation);
        }

        Ok(CredentialsLoginFlow::NeedsConfirmation(
            ConfirmationChallenge {
                client,
                config: self.config,
                client_id,
                steam_id,
                request_id,
                poll_interval,
                code_kinds,
                confirmation_kinds,
            },
        ))
    }
}

impl ConfirmationChallenge {
    /// Steam Guard code kinds Steam will accept via [`submit_code`]. Empty when
    /// the only path forward is an out-of-band confirmation.
    ///
    /// [`submit_code`]: ConfirmationChallenge::submit_code
    pub fn code_kinds(&self) -> &[GuardType] {
        &self.code_kinds
    }

    /// Out-of-band confirmation kinds pending (mobile-app approval, email
    /// link). Empty when the only path forward is entering a code.
    pub fn confirmation_kinds(&self) -> &[GuardType] {
        &self.confirmation_kinds
    }

    /// Whether a Steam Guard code may be submitted.
    pub fn accepts_code(&self) -> bool {
        !self.code_kinds.is_empty()
    }

    /// Whether the user can approve out of band (mobile app / email) instead
    /// of, or concurrently with, entering a code.
    pub fn accepts_confirmation(&self) -> bool {
        !self.confirmation_kinds.is_empty()
    }

    /// Submit a Steam Guard code. `Ok(())` means Steam accepted it; retrieve
    /// tokens with [`wait_for_tokens`]. `Err(LoginError::InvalidGuardCode)` is
    /// recoverable: the challenge is untouched, so the caller can prompt for a
    /// new code without restarting the RSA exchange.
    ///
    /// Borrows `&self` so it can run concurrently with [`wait_for_tokens`],
    /// letting the caller race code entry against an out-of-band confirmation.
    ///
    /// [`wait_for_tokens`]: ConfirmationChallenge::wait_for_tokens
    pub async fn submit_code(&self, code: &str, kind: GuardType) -> Result<(), LoginError> {
        match self
            .client
            .submit_steam_guard_code(self.client_id, self.steam_id, code, kind)
            .await
        {
            Ok(()) => Ok(()),
            Err(steamroom::Error::Connection(
                steamroom::error::ConnectionError::ServiceMethodFailed(
                    steamroom::enums::EResultError::TwoFactorCodeMismatch,
                ),
            )) => Err(LoginError::InvalidGuardCode),
            Err(e) => Err(LoginError::Transport(e)),
        }
    }

    /// Poll `PollAuthSessionStatus` until the session is confirmed by any
    /// accepted method (out-of-band approval, or a code accepted via
    /// [`submit_code`]) and tokens are issued.
    ///
    /// Borrows `&self` so it can race [`submit_code`]; pair it with
    /// [`into_approved`] to finish.
    ///
    /// [`submit_code`]: ConfirmationChallenge::submit_code
    /// [`into_approved`]: ConfirmationChallenge::into_approved
    pub async fn wait_for_tokens(&self) -> Result<AuthTokens, LoginError> {
        poll_until_tokens(
            &self.client,
            self.client_id,
            &self.request_id,
            self.poll_interval,
        )
        .await
    }

    /// Consume the challenge, wrapping issued tokens for the final logon.
    pub fn into_approved(self, tokens: AuthTokens) -> ApprovedAuth {
        ApprovedAuth {
            client: self.client,
            config: self.config,
            tokens,
        }
    }

    /// Convenience for the confirmation-only path (no code entry): poll until
    /// the user approves out of band, then return the approved handle.
    /// Consumes `self`, so it cannot be combined with [`submit_code`]; for the
    /// mixed case race [`wait_for_tokens`] against [`submit_code`] and finish
    /// with [`into_approved`].
    ///
    /// [`submit_code`]: ConfirmationChallenge::submit_code
    /// [`wait_for_tokens`]: ConfirmationChallenge::wait_for_tokens
    /// [`into_approved`]: ConfirmationChallenge::into_approved
    pub async fn wait_for_confirmation(self) -> Result<ApprovedAuth, LoginError> {
        let tokens = self.wait_for_tokens().await?;
        Ok(self.into_approved(tokens))
    }
}

/// Poll `PollAuthSessionStatus` until tokens are returned. Used by the
/// guard-code and mobile-confirmation completion paths, and by the no-2FA
/// path in `begin()`.
pub(crate) async fn poll_until_tokens(
    client: &SteamClient<Ready>,
    client_id: AuthClientId,
    request_id: &[u8],
    interval: PollInterval,
) -> Result<AuthTokens, LoginError> {
    loop {
        tokio::time::sleep(interval.as_duration()).await;
        if let Some(tokens) = client.poll_auth_session(client_id, request_id).await? {
            return Ok(tokens);
        }
    }
}
