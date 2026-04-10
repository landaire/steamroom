pub mod msg;
pub mod multi;

use bytes::Bytes;
use tokio::sync::mpsc;
use crate::apps::{AccessToken, AppInfo, BetaBranch};
use crate::auth::{AuthSession, AuthTokens, GuardType, QrAuthSession};
use crate::cdn::CdnServer;
use crate::content::CdnAuthToken;
use crate::depot::{AppId, CellId, DepotId, DepotKey, ManifestId};
use crate::error::Error;
use crate::messages::EMsg;
use crate::transport::Transport;
use self::msg::ClientMsg;

pub const PROTOCOL_VERSION: u32 = 65580;

pub struct SteamClient<S> {
    _state: S,
}

pub struct Disconnected;
pub struct Connected;
pub struct Encrypted;
pub struct LoggedIn;

pub type DisconnectedClient = SteamClient<Disconnected>;

#[derive(Clone, Debug)]
pub struct IncomingMsg {
    pub emsg: EMsg,
    pub is_protobuf: bool,
    pub header: Bytes,
    pub body: Bytes,
}

pub struct ServiceResponse {
    pub body: Bytes,
}

impl ServiceResponse {
    pub fn decode<M: prost::Message + Default>(&self) -> Result<M, prost::DecodeError> {
        M::decode(&*self.body)
    }
}

impl SteamClient<Disconnected> {
    pub async fn connect<T: Transport>(
        transport: T,
    ) -> Result<(SteamClient<Connected>, mpsc::UnboundedReceiver<IncomingMsg>), Error> {
        todo!()
    }
}

impl SteamClient<Connected> {
    pub async fn encrypt(self) -> Result<SteamClient<Encrypted>, Error> {
        todo!()
    }
}

impl SteamClient<Encrypted> {
    pub async fn login(self, msg: ClientMsg<'_>) -> Result<(SteamClient<LoggedIn>, IncomingMsg), Error> {
        todo!()
    }

    pub async fn send_msg(&self, msg: &ClientMsg<'_>) -> Result<(), Error> {
        todo!()
    }

    pub async fn recv_msg(&self) -> Result<IncomingMsg, Error> {
        todo!()
    }

    pub async fn call_service_method_non_authed(
        &self,
        method_name: &str,
        body: &[u8],
    ) -> Result<ServiceResponse, Error> {
        todo!()
    }

    pub async fn get_password_rsa_public_key(
        &self,
        account_name: &str,
    ) -> Result<crate::generated::CAuthenticationGetPasswordRsaPublicKeyResponse, Error> {
        todo!()
    }

    pub async fn begin_auth_session_via_credentials(
        &self,
        request: crate::generated::CAuthenticationBeginAuthSessionViaCredentialsRequest,
    ) -> Result<AuthSession, Error> {
        todo!()
    }

    pub async fn begin_auth_session_via_qr(
        &self,
        request: crate::generated::CAuthenticationBeginAuthSessionViaQrRequest,
    ) -> Result<QrAuthSession, Error> {
        todo!()
    }

    pub async fn poll_auth_session(
        &self,
        client_id: u64,
        request_id: &[u8],
    ) -> Result<Option<AuthTokens>, Error> {
        todo!()
    }

    pub async fn submit_steam_guard_code(
        &self,
        client_id: u64,
        steam_id: u64,
        code: &str,
        code_type: GuardType,
    ) -> Result<(), Error> {
        todo!()
    }
}

impl SteamClient<LoggedIn> {
    pub async fn send_msg(&self, msg: &ClientMsg<'_>) -> Result<(), Error> {
        todo!()
    }

    pub async fn recv_msg(&self) -> Result<IncomingMsg, Error> {
        todo!()
    }

    pub async fn send_heartbeat(&self) -> Result<(), Error> {
        todo!()
    }

    pub async fn call_service_method(
        &self,
        method_name: &str,
        body: &[u8],
    ) -> Result<ServiceResponse, Error> {
        todo!()
    }

    pub async fn pics_get_access_tokens(
        &self,
        app_ids: &[AppId],
    ) -> Result<Vec<AccessToken>, Error> {
        todo!()
    }

    pub async fn pics_get_product_info(
        &self,
        apps: &[AccessToken],
    ) -> Result<Vec<AppInfo>, Error> {
        todo!()
    }

    pub async fn get_depot_decryption_key(
        &self,
        depot_id: DepotId,
        app_id: AppId,
    ) -> Result<DepotKey, Error> {
        todo!()
    }

    pub async fn check_beta_password(
        &self,
        app_id: AppId,
        password: &str,
    ) -> Result<Vec<BetaBranch>, Error> {
        todo!()
    }

    pub async fn get_cdn_servers(
        &self,
        cell_id: CellId,
        max_servers: Option<u32>,
    ) -> Result<Vec<CdnServer>, Error> {
        todo!()
    }

    pub async fn get_manifest_request_code(
        &self,
        app_id: AppId,
        depot_id: DepotId,
        manifest_id: ManifestId,
        branch: Option<&str>,
        branch_password_hash: Option<&str>,
    ) -> Result<Option<u64>, Error> {
        todo!()
    }

    pub async fn get_cdn_auth_token(
        &self,
        app_id: AppId,
        depot_id: DepotId,
        host_name: &str,
    ) -> Result<CdnAuthToken, Error> {
        todo!()
    }
}
