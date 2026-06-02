use crate::login::credentials::poll_until_tokens;
use crate::login::error::LoginError;
use crate::login::terminal::ApprovedAuth;
use crate::login::{BuilderConfig, TransportConfig, establish_ready_client};

use steamroom::auth::GuardType;
use steamroom::client::{Ready, SteamClient};
use steamroom::generated::CAuthenticationBeginAuthSessionViaQrRequest;

/// Configured QR login. Call [`begin()`] to start the flow.
///
/// [`begin()`]: QrLogin::begin
pub struct QrLogin {
    pub(crate) config: BuilderConfig,
    pub(crate) transport: TransportConfig,
}

/// QR auth session in progress: the caller renders `challenge_url()` as a QR
/// code (or prints it), then calls `wait_for_scan()` to block until the user
/// approves on their Steam mobile app.
pub struct QrLoginFlow {
    client: SteamClient<Ready>,
    config: BuilderConfig,
    challenge_url: String,
    client_id: u64,
    request_id: Vec<u8>,
    poll_interval: f32,
    allowed_kinds: Vec<GuardType>,
}

impl QrLogin {
    /// Connect (or accept BYO client) and call `BeginAuthSessionViaQR`.
    pub async fn begin(self) -> Result<QrLoginFlow, LoginError> {
        let client = establish_ready_client(self.transport).await?;

        let device_name = self
            .config
            .device_name
            .clone()
            .unwrap_or_else(|| "steamroom".to_string());
        let req = CAuthenticationBeginAuthSessionViaQrRequest {
            device_friendly_name: Some(device_name),
            ..Default::default()
        };
        let session = client.begin_auth_session_via_qr(req).await?;

        Ok(QrLoginFlow {
            client,
            config: self.config,
            challenge_url: session
                .challenge_url
                .ok_or(LoginError::MissingField("challenge_url"))?,
            client_id: session
                .client_id
                .ok_or(LoginError::MissingField("client_id"))?,
            request_id: session
                .request_id
                .ok_or(LoginError::MissingField("request_id"))?,
            poll_interval: session.poll_interval.unwrap_or(5.0),
            allowed_kinds: session.allowed_confirmations,
        })
    }
}

impl QrLoginFlow {
    /// URL to encode as a QR code or print for the user. The caller picks
    /// the renderer (the steamroom CLI uses the `qrcode` crate).
    pub fn challenge_url(&self) -> &str {
        &self.challenge_url
    }

    /// Confirmation kinds Steam reported as acceptable (informational —
    /// always mobile confirmation for QR sessions).
    pub fn allowed_kinds(&self) -> &[GuardType] {
        &self.allowed_kinds
    }

    /// Poll `PollAuthSessionStatus` until the user scans + approves and
    /// tokens are returned.
    pub async fn wait_for_scan(self) -> Result<ApprovedAuth, LoginError> {
        let tokens = poll_until_tokens(
            &self.client,
            self.client_id,
            &self.request_id,
            self.poll_interval,
        )
        .await?;
        Ok(ApprovedAuth {
            client: self.client,
            config: self.config,
            tokens,
        })
    }
}
