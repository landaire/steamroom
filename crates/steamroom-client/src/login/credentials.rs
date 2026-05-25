use crate::login::error::LoginError;
use crate::login::terminal::ApprovedAuth;
use crate::login::{BuilderConfig, TransportConfig, establish_encrypted_client};

use base64::Engine;
use steamroom::auth::GuardType;
use steamroom::client::{Encrypted, SteamClient};
use steamroom::generated::CAuthenticationBeginAuthSessionViaCredentialsRequest;

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
    /// Steam Guard code required. Call `GuardChallenge::submit_code()`.
    NeedsGuardCode(GuardChallenge),
    /// Mobile-app confirmation pending. Call
    /// `MobileChallenge::wait_for_confirmation()`.
    NeedsMobileConfirm(MobileChallenge),
}

pub struct GuardChallenge {
    pub(crate) client: SteamClient<Encrypted>,
    pub(crate) config: BuilderConfig,
    pub(crate) client_id: u64,
    pub(crate) steam_id: u64,
    pub(crate) request_id: Vec<u8>,
    pub(crate) poll_interval: f32,
    pub(crate) allowed_kinds: Vec<GuardType>,
}

pub struct MobileChallenge {
    pub(crate) client: SteamClient<Encrypted>,
    pub(crate) config: BuilderConfig,
    pub(crate) client_id: u64,
    pub(crate) request_id: Vec<u8>,
    pub(crate) poll_interval: f32,
}

impl CredentialsLogin {
    /// Connect (or accept BYO client), fetch the RSA pubkey, RSA-encrypt the
    /// password, and call `BeginAuthSessionViaCredentials`. The returned flow
    /// indicates whether 2FA is needed and which kind.
    pub async fn begin(self) -> Result<CredentialsLoginFlow, LoginError> {
        let client = establish_encrypted_client(self.transport).await?;

        // RSA exchange
        let rsa = client
            .get_password_rsa_public_key(&self.account_name)
            .await?;
        let modulus = rsa.publickey_mod.ok_or(LoginError::MissingField("publickey_mod"))?;
        let exponent = rsa.publickey_exp.ok_or(LoginError::MissingField("publickey_exp"))?;
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

        let client_id = session.client_id.ok_or(LoginError::MissingField("client_id"))?;
        let request_id = session.request_id.ok_or(LoginError::MissingField("request_id"))?;
        let steam_id = session.steam_id.ok_or(LoginError::MissingField("steamid"))?;
        let poll_interval = session.poll_interval.unwrap_or(5.0);

        // Classify the next step.
        if session.allowed_confirmations.is_empty()
            || session
                .allowed_confirmations
                .iter()
                .any(|g| *g == GuardType::None)
        {
            // No 2FA needed — poll once for tokens (Steam may have them ready)
            // and produce ApprovedAuth directly.
            let tokens = poll_until_tokens(
                &client,
                client_id,
                &request_id,
                poll_interval,
            )
            .await?;
            return Ok(CredentialsLoginFlow::Approved(ApprovedAuth {
                client,
                config: self.config,
                tokens,
            }));
        }

        let needs_code = session
            .allowed_confirmations
            .iter()
            .any(|g| matches!(g, GuardType::DeviceCode | GuardType::EmailCode));

        if needs_code {
            return Ok(CredentialsLoginFlow::NeedsGuardCode(GuardChallenge {
                client,
                config: self.config,
                client_id,
                steam_id,
                request_id,
                poll_interval,
                allowed_kinds: session.allowed_confirmations,
            }));
        }

        // Mobile confirmation (DeviceConfirmation only)
        Ok(CredentialsLoginFlow::NeedsMobileConfirm(MobileChallenge {
            client,
            config: self.config,
            client_id,
            request_id,
            poll_interval,
        }))
    }
}

/// Poll `PollAuthSessionStatus` until tokens are returned. Used by the
/// guard-code and mobile-confirmation completion paths, and by the no-2FA
/// path in `begin()`.
pub(crate) async fn poll_until_tokens(
    client: &SteamClient<Encrypted>,
    client_id: u64,
    request_id: &[u8],
    interval_secs: f32,
) -> Result<steamroom::auth::AuthTokens, LoginError> {
    loop {
        tokio::time::sleep(std::time::Duration::from_secs_f32(interval_secs)).await;
        if let Some(tokens) = client.poll_auth_session(client_id, request_id).await? {
            return Ok(tokens);
        }
    }
}
