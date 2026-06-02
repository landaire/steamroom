use crate::login::error::LoginError;
use crate::login::{BuilderConfig, TransportConfig, establish_ready_client};

use steamroom::auth::AuthTokens;

use prost::Message;
use steamroom::client::msg::ClientMsg;
use steamroom::client::{LoggedIn, PROTOCOL_VERSION, Ready, SteamClient};
use steamroom::generated::CMsgClientLogon;
use steamroom::messages::EMsg;
use steamroom::types::SteamId;

/// Anonymous login. No credentials, no tokens. Produced by
/// `LoginBuilder::anonymous()` or `PreparedLoginBuilder::anonymous()`.
pub struct AnonymousLogin {
    pub(crate) config: BuilderConfig,
    pub(crate) transport: TransportConfig,
}

impl AnonymousLogin {
    /// Connect (if needed), send `CMsgClientLogon` with no credentials, wait
    /// for `CLIENT_LOG_ON_RESPONSE`.
    pub async fn login(self) -> Result<SteamClient<LoggedIn>, LoginError> {
        let client = establish_ready_client(self.transport).await?;
        let logon = CMsgClientLogon {
            protocol_version: Some(PROTOCOL_VERSION),
            cell_id: Some(self.config.cell_id),
            client_os_type: Some(self.config.client_os.proto_value()),
            ..Default::default()
        };
        let steam_id = SteamId::from_parts(1, 10, 0, 0).raw();
        finish_logon(client, logon, steam_id).await
    }
}

/// Login with a previously-obtained refresh token. Produced by
/// `LoginBuilder::with_refresh_token()` or
/// `PreparedLoginBuilder::with_refresh_token()`.
pub struct TokenLogin {
    pub(crate) config: BuilderConfig,
    pub(crate) transport: TransportConfig,
    pub(crate) account_name: String,
    pub(crate) refresh_token: String,
}

impl TokenLogin {
    /// Connect (if needed), send `CMsgClientLogon` with the refresh token
    /// in the `access_token` field (Steam's wire name for this slot — yes,
    /// it's confusing), wait for `CLIENT_LOG_ON_RESPONSE`.
    pub async fn login(self) -> Result<SteamClient<LoggedIn>, LoginError> {
        let client = establish_ready_client(self.transport).await?;
        let logon = CMsgClientLogon {
            protocol_version: Some(PROTOCOL_VERSION),
            cell_id: Some(self.config.cell_id),
            client_os_type: Some(self.config.client_os.proto_value()),
            account_name: Some(self.account_name),
            access_token: Some(self.refresh_token),
            ..Default::default()
        };
        let steam_id = SteamId::from_parts(1, 1, 1, 0).raw();
        finish_logon(client, logon, steam_id).await
    }
}

/// Auth flow has completed; tokens are available. Inspect via [`tokens()`]
/// (e.g. to persist the refresh token) and then call [`finish()`] to send the
/// `CMsgClientLogon` and reach the `LoggedIn` state.
///
/// [`tokens()`]: ApprovedAuth::tokens
/// [`finish()`]: ApprovedAuth::finish
pub struct ApprovedAuth {
    pub(crate) client: SteamClient<Ready>,
    pub(crate) config: BuilderConfig,
    pub(crate) tokens: AuthTokens,
}

impl ApprovedAuth {
    pub fn tokens(&self) -> &AuthTokens {
        &self.tokens
    }

    pub async fn finish(self) -> Result<SteamClient<LoggedIn>, LoginError> {
        let account_name = self
            .tokens
            .account_name
            .clone()
            .ok_or(LoginError::MissingField("account_name"))?;
        let logon = CMsgClientLogon {
            protocol_version: Some(PROTOCOL_VERSION),
            cell_id: Some(self.config.cell_id),
            client_os_type: Some(self.config.client_os.proto_value()),
            account_name: Some(account_name),
            access_token: Some(self.tokens.access_token),
            ..Default::default()
        };
        let steam_id = SteamId::from_parts(1, 1, 1, 0).raw();
        finish_logon(self.client, logon, steam_id).await
    }
}

/// Shared logon: send `CMsgClientLogon`, await `CLIENT_LOG_ON_RESPONSE`.
/// Translates `ConnectionError::LogonFailed` to `LoginError::LogonFailed`;
/// other errors become `LoginError::Transport`.
pub(crate) async fn finish_logon(
    client: SteamClient<Ready>,
    logon: CMsgClientLogon,
    steam_id: u64,
) -> Result<SteamClient<LoggedIn>, LoginError> {
    let body = logon.encode_to_vec();
    let mut msg = ClientMsg::with_body(EMsg::CLIENT_LOGON, &body);
    msg.header.steamid = Some(steam_id);
    msg.header.client_sessionid = Some(0);

    match client.login(msg).await {
        Ok((client, _resp)) => Ok(client),
        Err(steamroom::Error::Connection(
            steamroom::error::ConnectionError::LogonFailed(eresult),
        )) => Err(LoginError::LogonFailed(eresult)),
        Err(e) => Err(LoginError::Transport(e)),
    }
}
