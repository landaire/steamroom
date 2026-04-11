pub mod msg;
pub mod multi;

use std::sync::Arc;
use std::sync::atomic::{AtomicI32, AtomicU64, Ordering};
use bytes::Bytes;
use prost::Message;
use futures_util::lock::Mutex;
use tracing::{debug, trace};
use crate::apps::{AccessToken, AppInfo, BetaBranch};
use crate::auth::{AuthSession, AuthTokens, GuardType, QrAuthSession};
use crate::cdn::CdnServer;
use crate::content::CdnAuthToken;
use crate::depot::{AppId, CellId, DepotId, DepotKey, ManifestId};
use crate::enums::EResultError;
use crate::error::{ConnectionError, Error};
use crate::generated;
use crate::messages::{EMsg, RawEMsg};
use crate::messages::header::{self, PacketHeader};
use crate::transport::Transport;
use self::msg::ClientMsg;

pub const PROTOCOL_VERSION: u32 = 65581;

struct ClientInner {
    transport: Box<dyn Transport>,
    cipher: Mutex<Option<crate::connection::encryption::SessionCipher>>,
    steam_id: AtomicU64,
    session_id: AtomicI32,
    source_job_id: AtomicU64,
}

pub struct SteamClient<S> {
    inner: Arc<ClientInner>,
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
    pub header: generated::CMsgProtoBufHeader,
    pub body: Bytes,
}

pub struct ServiceResponse {
    pub body: Bytes,
}

impl ServiceResponse {
    pub fn decode<M: Message + Default>(&self) -> Result<M, prost::DecodeError> {
        M::decode(&*self.body)
    }
}

impl SteamClient<Disconnected> {
    pub async fn connect<T: Transport>(
        transport: T,
    ) -> Result<(SteamClient<Connected>, async_channel::Receiver<IncomingMsg>), Error> {
        let (_tx, rx) = async_channel::unbounded();
        let inner = Arc::new(ClientInner {
            transport: Box::new(transport),
            cipher: Mutex::new(None),
            steam_id: AtomicU64::new(0),
            session_id: AtomicI32::new(0),
            source_job_id: AtomicU64::new(1),
        });

        Ok((
            SteamClient {
                inner,
                _state: Connected,
            },
            rx,
        ))
    }

    /// Connect via WebSocket — skips encryption handshake (TLS handles it).
    /// Messages are sent/received as plaintext over the WebSocket.
    pub async fn connect_ws<T: Transport>(
        transport: T,
    ) -> Result<(SteamClient<Encrypted>, async_channel::Receiver<IncomingMsg>), Error> {
        let (_tx, rx) = async_channel::unbounded();
        let inner = Arc::new(ClientInner {
            transport: Box::new(transport),
            cipher: Mutex::new(None),
            steam_id: AtomicU64::new(0),
            session_id: AtomicI32::new(0),
            source_job_id: AtomicU64::new(1),
        });

        Ok((
            SteamClient {
                inner,
                _state: Encrypted,
            },
            rx,
        ))
    }
}

impl SteamClient<Connected> {
    pub async fn encrypt(self) -> Result<SteamClient<Encrypted>, Error> {
        debug!("waiting for ChannelEncryptRequest...");
        // Wait for ChannelEncryptRequest
        let data = self.inner.transport.recv().await?;
        debug!("received {} bytes", data.len());
        let parsed = header::PacketHeader::parse(&data)?;
        let (emsg, body) = match parsed {
            PacketHeader::Simple { header, body } => (header.emsg, body),
            _ => return Err(ConnectionError::EncryptionFailed.into()),
        };

        if emsg != EMsg::CHANNEL_ENCRYPT_REQUEST {
            return Err(ConnectionError::UnexpectedEMsg {
                expected: EMsg::CHANNEL_ENCRYPT_REQUEST,
                got: emsg,
            }
            .into());
        }

        // Generate session key
        let mut session_key = [0u8; 32];
        getrandom::getrandom(&mut session_key).expect("RNG failed");

        // The body contains: protocol version (u32) + universe (u32) + optional nonce (16 bytes)
        // We need to encrypt (session_key + nonce) with Steam's RSA public key
        let nonce = if body.len() > 8 { &body[8..] } else { &[] as &[u8] };

        let mut plaintext = Vec::with_capacity(32 + nonce.len());
        plaintext.extend_from_slice(&session_key);
        plaintext.extend_from_slice(nonce);
        let encrypted_key = crate::crypto::rsa::encrypt_with_steam_public_key(&plaintext)?;

        // Build ChannelEncryptResponse
        // Layout: protocol_version(u32) + key_size(u32) + encrypted_key + crc32 + trailing_zeros(u32)
        let mut response_body = Vec::new();
        response_body.extend_from_slice(&1u32.to_le_bytes()); // protocol version
        response_body.extend_from_slice(&(encrypted_key.len() as u32).to_le_bytes());
        response_body.extend_from_slice(&encrypted_key);
        let crc = crc32fast::hash(&encrypted_key);
        response_body.extend_from_slice(&crc.to_le_bytes());
        response_body.extend_from_slice(&0u32.to_le_bytes());

        // Send as simple (non-protobuf) message
        let mut packet = Vec::new();
        let raw = RawEMsg::without_proto(EMsg::CHANNEL_ENCRYPT_RESPONSE);
        packet.extend_from_slice(&raw.0.to_le_bytes());
        packet.extend_from_slice(&u64::MAX.to_le_bytes()); // target_job_id
        packet.extend_from_slice(&u64::MAX.to_le_bytes()); // source_job_id
        packet.extend_from_slice(&response_body);

        debug!("encrypt response packet ({} bytes): {:02x?}", packet.len(), &packet[..std::cmp::min(64, packet.len())]);
        self.inner.transport.send(&packet).await?;

        // Wait for ChannelEncryptResult
        let data = self.inner.transport.recv().await?;
        let parsed = header::PacketHeader::parse(&data)?;
        let (emsg, body) = match parsed {
            PacketHeader::Simple { header, body } => (header.emsg, body),
            _ => return Err(ConnectionError::EncryptionFailed.into()),
        };

        if emsg != EMsg::CHANNEL_ENCRYPT_RESULT {
            return Err(ConnectionError::UnexpectedEMsg {
                expected: EMsg::CHANNEL_ENCRYPT_RESULT,
                got: emsg,
            }
            .into());
        }

        if body.len() >= 4 {
            let code = u32::from_le_bytes(body[..4].try_into().unwrap()) as i32;
            debug!("ChannelEncryptResult code={code}");
            crate::enums::eresult(code)
                .map_err(|_| ConnectionError::EncryptionFailed)?;
        }

        // Store the session cipher
        let cipher = crate::connection::encryption::SessionCipher::new(session_key);
        *self.inner.cipher.lock().await = Some(cipher);

        debug!("encryption handshake complete");
        Ok(SteamClient {
            inner: self.inner,
            _state: Encrypted,
        })
    }
}

impl SteamClient<Encrypted> {
    async fn send_raw(&self, msg: &ClientMsg<'_>) -> Result<(), Error> {
        let data = msg.to_bytes();
        let cipher_guard = self.inner.cipher.lock().await;
        if let Some(cipher) = cipher_guard.as_ref() {
            let encrypted = cipher.encrypt(&data);
            self.inner.transport.send(&encrypted).await
        } else {
            // WebSocket mode: no cipher, send plaintext
            self.inner.transport.send(&data).await
        }
    }

    async fn recv_raw(&self) -> Result<IncomingMsg, Error> {
        let raw = self.inner.transport.recv().await?;

        let cipher_guard = self.inner.cipher.lock().await;
        let data = if let Some(cipher) = cipher_guard.as_ref() {
            cipher
                .decrypt(&raw)
                .map_err(|_| ConnectionError::EncryptionFailed)?
        } else {
            raw.to_vec()
        };
        parse_incoming(&data)
    }

    async fn send_hello(&self) -> Result<(), Error> {
        let hello = generated::CMsgClientHello {
            protocol_version: Some(PROTOCOL_VERSION),
        };
        let body = hello.encode_to_vec();
        let msg = ClientMsg::with_body(EMsg::CLIENT_HELLO, &body);
        self.send_raw(&msg).await
    }

    pub async fn login(
        self,
        msg: ClientMsg<'_>,
    ) -> Result<(SteamClient<LoggedIn>, IncomingMsg), Error> {
        // Send ClientHello then logon immediately
        self.send_hello().await?;
        self.send_raw(&msg).await?;

        // Process messages until we get LogOnResponse
        loop {
            let incoming = self.recv_raw().await?;
            debug!("login: received {:?}", incoming.emsg);

            debug!("login: received emsg={:?}", incoming.emsg);
            match incoming.emsg {
                EMsg::CLIENT_LOG_ON_RESPONSE => {
                    let resp = generated::CMsgClientLogonResponse::decode(&*incoming.body)?;
                    crate::enums::eresult(resp.eresult.ok_or(ConnectionError::MissingField("eresult"))?)
                        .map_err(ConnectionError::LogonFailed)?;

                    if let Some(sid) = incoming.header.steamid {
                        self.inner.steam_id.store(sid, Ordering::Relaxed);
                    }
                    if let Some(session_id) = incoming.header.client_sessionid {
                        self.inner.session_id.store(session_id, Ordering::Relaxed);
                    }

                    debug!("logged in, steamid={}", self.inner.steam_id.load(Ordering::Relaxed));

                    return Ok((
                        SteamClient {
                            inner: self.inner,
                            _state: LoggedIn,
                        },
                        incoming,
                    ));
                }
                EMsg::MULTI => {
                    // Unpack multi and check for logon response inside
                    let msgs = multi::unpack_multi(&incoming.body)?;
                    for sub in msgs {
                        let sub_msg = parse_incoming(&sub)?;
                        if sub_msg.emsg == EMsg::CLIENT_LOG_ON_RESPONSE {
                            let resp = generated::CMsgClientLogonResponse::decode(&*sub_msg.body)?;
                            crate::enums::eresult(resp.eresult.ok_or(ConnectionError::MissingField("eresult"))?)
                                .map_err(ConnectionError::LogonFailed)?;

                            if let Some(sid) = sub_msg.header.steamid {
                                self.inner.steam_id.store(sid, Ordering::Relaxed);
                            }
                            if let Some(session_id) = sub_msg.header.client_sessionid {
                                self.inner.session_id.store(session_id, Ordering::Relaxed);
                            }

                            return Ok((
                                SteamClient {
                                    inner: self.inner,
                                    _state: LoggedIn,
                                },
                                sub_msg,
                            ));
                        }
                    }
                }
                _ => {
                    trace!("login: ignoring {:?}", incoming.emsg);
                }
            }
        }
    }

    pub async fn send_msg(&self, msg: &ClientMsg<'_>) -> Result<(), Error> {
        self.send_raw(msg).await
    }

    pub async fn recv_msg(&self) -> Result<IncomingMsg, Error> {
        self.recv_raw().await
    }

    pub async fn call_service_method_non_authed(
        &self,
        method_name: &str,
        body: &[u8],
    ) -> Result<ServiceResponse, Error> {
        let job_id = self.inner.source_job_id.fetch_add(1, Ordering::Relaxed);
        let mut msg = ClientMsg::with_body(EMsg::SERVICE_METHOD_CALL_FROM_CLIENT, body);
        msg.header.target_job_name = Some(method_name.to_string());
        msg.header.jobid_source = Some(job_id);
        self.send_raw(&msg).await?;

        loop {
            let incoming = self.recv_raw().await?;
            if incoming.emsg == EMsg::SERVICE_METHOD_RESPONSE
                && incoming.header.jobid_target == Some(job_id)
            {
                check_service_eresult(&incoming)?;
                return Ok(ServiceResponse { body: incoming.body });
            }
            if incoming.emsg == EMsg::MULTI {
                let msgs = multi::unpack_multi(&incoming.body)?;
                for sub in msgs {
                    let sub_msg = parse_incoming(&sub)?;
                    if sub_msg.emsg == EMsg::SERVICE_METHOD_RESPONSE
                        && sub_msg.header.jobid_target == Some(job_id)
                    {
                        check_service_eresult(&sub_msg)?;
                        return Ok(ServiceResponse { body: sub_msg.body });
                    }
                }
            }
        }
    }

    pub async fn get_password_rsa_public_key(
        &self,
        account_name: &str,
    ) -> Result<generated::CAuthenticationGetPasswordRsaPublicKeyResponse, Error> {
        let req = generated::CAuthenticationGetPasswordRsaPublicKeyRequest {
            account_name: Some(account_name.to_string()),
        };
        let resp = self
            .call_service_method_non_authed(
                "Authentication.GetPasswordRSAPublicKey#1",
                &req.encode_to_vec(),
            )
            .await?;
        Ok(resp.decode()?)
    }

    pub async fn begin_auth_session_via_credentials(
        &self,
        request: generated::CAuthenticationBeginAuthSessionViaCredentialsRequest,
    ) -> Result<AuthSession, Error> {
        let resp = self
            .call_service_method_non_authed(
                "Authentication.BeginAuthSessionViaCredentials#1",
                &request.encode_to_vec(),
            )
            .await?;
        let r: generated::CAuthenticationBeginAuthSessionViaCredentialsResponse = resp.decode()?;
        Ok(AuthSession {
            client_id: r.client_id,
            request_id: r.request_id,
            poll_interval: r.interval,
            allowed_confirmations: r
                .allowed_confirmations
                .iter()
                .filter_map(|c| guard_type_from_proto(c.confirmation_type))
                .collect(),
            steam_id: r.steamid,
        })
    }

    pub async fn begin_auth_session_via_qr(
        &self,
        request: generated::CAuthenticationBeginAuthSessionViaQrRequest,
    ) -> Result<QrAuthSession, Error> {
        let resp = self
            .call_service_method_non_authed(
                "Authentication.BeginAuthSessionViaQR#1",
                &request.encode_to_vec(),
            )
            .await?;
        let r: generated::CAuthenticationBeginAuthSessionViaQrResponse = resp.decode()?;
        Ok(QrAuthSession {
            client_id: r.client_id,
            request_id: r.request_id,
            challenge_url: r.challenge_url,
            poll_interval: r.interval,
            allowed_confirmations: r
                .allowed_confirmations
                .iter()
                .filter_map(|c| guard_type_from_proto(c.confirmation_type))
                .collect(),
        })
    }

    pub async fn poll_auth_session(
        &self,
        client_id: u64,
        request_id: &[u8],
    ) -> Result<Option<AuthTokens>, Error> {
        let req = generated::CAuthenticationPollAuthSessionStatusRequest {
            client_id: Some(client_id),
            request_id: Some(request_id.to_vec()),
            ..Default::default()
        };
        let resp = self
            .call_service_method_non_authed(
                "Authentication.PollAuthSessionStatus#1",
                &req.encode_to_vec(),
            )
            .await?;
        let r: generated::CAuthenticationPollAuthSessionStatusResponse = resp.decode()?;
        if let (Some(access), Some(refresh)) =
            (r.access_token.as_ref(), r.refresh_token.as_ref())
        {
            if !access.is_empty() {
                return Ok(Some(AuthTokens {
                    access_token: access.clone(),
                    refresh_token: refresh.clone(),
                    account_name: r.account_name,
                }));
            }
        }
        Ok(None)
    }

    pub async fn submit_steam_guard_code(
        &self,
        client_id: u64,
        steam_id: u64,
        code: &str,
        code_type: GuardType,
    ) -> Result<(), Error> {
        let req = generated::CAuthenticationUpdateAuthSessionWithSteamGuardCodeRequest {
            client_id: Some(client_id),
            steamid: Some(steam_id),
            code: Some(code.to_string()),
            code_type: Some(code_type.to_proto()),
        };
        self.call_service_method_non_authed(
            "Authentication.UpdateAuthSessionWithSteamGuardCode#1",
            &req.encode_to_vec(),
        )
        .await?;
        Ok(())
    }
}

impl SteamClient<LoggedIn> {
    async fn send_raw(&self, msg: &ClientMsg<'_>) -> Result<(), Error> {
        let data = msg.to_bytes();
        let cipher_guard = self.inner.cipher.lock().await;
        if let Some(cipher) = cipher_guard.as_ref() {
            let encrypted = cipher.encrypt(&data);
            self.inner.transport.send(&encrypted).await
        } else {
            self.inner.transport.send(&data).await
        }
    }

    async fn recv_raw(&self) -> Result<IncomingMsg, Error> {
        let raw = self.inner.transport.recv().await?;
        let cipher_guard = self.inner.cipher.lock().await;
        let data = if let Some(cipher) = cipher_guard.as_ref() {
            cipher
                .decrypt(&raw)
                .map_err(|_| ConnectionError::EncryptionFailed)?
        } else {
            raw.to_vec()
        };
        parse_incoming(&data)
    }

    fn make_msg<'a>(&self, emsg: EMsg, body: &'a [u8]) -> ClientMsg<'a> {
        let mut msg = ClientMsg::with_body(emsg, body);
        msg.header.steamid = Some(self.inner.steam_id.load(Ordering::Relaxed));
        msg.header.client_sessionid = Some(self.inner.session_id.load(Ordering::Relaxed));
        msg
    }

    pub async fn send_msg(&self, msg: &ClientMsg<'_>) -> Result<(), Error> {
        self.send_raw(msg).await
    }

    pub async fn recv_msg(&self) -> Result<IncomingMsg, Error> {
        self.recv_raw().await
    }

    pub async fn send_heartbeat(&self) -> Result<(), Error> {
        let msg = self.make_msg(EMsg::CLIENT_HEART_BEAT, &[]);
        self.send_raw(&msg).await
    }

    pub async fn call_service_method(
        &self,
        method_name: &str,
        body: &[u8],
    ) -> Result<ServiceResponse, Error> {
        let job_id = self.inner.source_job_id.fetch_add(1, Ordering::Relaxed);
        let mut msg = self.make_msg(EMsg::SERVICE_METHOD_CALL_FROM_CLIENT, body);
        msg.header.target_job_name = Some(method_name.to_string());
        msg.header.jobid_source = Some(job_id);
        self.send_raw(&msg).await?;

        loop {
            let incoming = self.recv_raw().await?;
            if incoming.emsg == EMsg::SERVICE_METHOD_RESPONSE
                && incoming.header.jobid_target == Some(job_id)
            {
                check_service_eresult(&incoming)?;
                return Ok(ServiceResponse { body: incoming.body });
            }
            if incoming.emsg == EMsg::MULTI {
                let msgs = multi::unpack_multi(&incoming.body)?;
                for sub in msgs {
                    let sub_msg = parse_incoming(&sub)?;
                    if sub_msg.emsg == EMsg::SERVICE_METHOD_RESPONSE
                        && sub_msg.header.jobid_target == Some(job_id)
                    {
                        check_service_eresult(&sub_msg)?;
                        return Ok(ServiceResponse { body: sub_msg.body });
                    }
                }
            }
        }
    }

    pub async fn pics_get_access_tokens(
        &self,
        app_ids: &[AppId],
    ) -> Result<Vec<AccessToken>, Error> {
        let req = generated::CMsgClientPicsAccessTokenRequest {
            appids: app_ids.iter().map(|a| a.0).collect(),
            ..Default::default()
        };
        let body = req.encode_to_vec();
        let msg = self.make_msg(EMsg::CLIENT_PICS_ACCESS_TOKEN_REQUEST, &body); // k_EMsgClientPICSAccessTokenRequest
        self.send_raw(&msg).await?;

        loop {
            let incoming = self.recv_raw().await?;
            if incoming.emsg == EMsg::CLIENT_PICS_ACCESS_TOKEN_RESPONSE {
                // k_EMsgClientPICSAccessTokenResponse
                let resp = generated::CMsgClientPicsAccessTokenResponse::decode(&*incoming.body)?;
                return Ok(resp
                    .app_access_tokens
                    .iter()
                    .map(|t| AccessToken {
                        app_id: AppId(t.appid.unwrap_or(0)), // appid echoed back from our request
                        token: t.access_token.unwrap_or(0), // 0 = no token needed (free app)
                    })
                    .collect());
            }
            if incoming.emsg == EMsg::MULTI {
                let msgs = multi::unpack_multi(&incoming.body)?;
                for sub in msgs {
                    let sub_msg = parse_incoming(&sub)?;
                    if sub_msg.emsg == EMsg::CLIENT_PICS_ACCESS_TOKEN_RESPONSE {
                        let resp =
                            generated::CMsgClientPicsAccessTokenResponse::decode(&*sub_msg.body)?;
                        return Ok(resp
                            .app_access_tokens
                            .iter()
                            .map(|t| AccessToken {
                                app_id: AppId(t.appid.unwrap_or(0)), // appid echoed back from our request
                                token: t.access_token.unwrap_or(0), // 0 = no token needed (free app)
                            })
                            .collect());
                    }
                }
            }
        }
    }

    pub async fn pics_get_product_info(
        &self,
        apps: &[AccessToken],
    ) -> Result<Vec<AppInfo>, Error> {
        let req = generated::CMsgClientPicsProductInfoRequest {
            apps: apps
                .iter()
                .map(|a| generated::c_msg_client_pics_product_info_request::AppInfo {
                    appid: Some(a.app_id.0),
                    access_token: Some(a.token),
                    ..Default::default()
                })
                .collect(),
            meta_data_only: Some(false),
            ..Default::default()
        };
        let body = req.encode_to_vec();
        let msg = self.make_msg(EMsg::CLIENT_PICS_PRODUCT_INFO_REQUEST, &body); // k_EMsgClientPICSProductInfoRequest
        self.send_raw(&msg).await?;

        loop {
            let incoming = self.recv_raw().await?;
            if incoming.emsg == EMsg::CLIENT_PICS_PRODUCT_INFO_RESPONSE {
                // k_EMsgClientPICSProductInfoResponse
                let resp = generated::CMsgClientPicsProductInfoResponse::decode(&*incoming.body)?;
                return Ok(resp
                    .apps
                    .iter()
                    .map(|a| AppInfo {
                        app_id: a.appid.map(AppId),
                        change_number: a.change_number,
                        kv_data: a.buffer.clone(),
                    })
                    .collect());
            }
            if incoming.emsg == EMsg::MULTI {
                let msgs = multi::unpack_multi(&incoming.body)?;
                for sub in msgs {
                    let sub_msg = parse_incoming(&sub)?;
                    if sub_msg.emsg == EMsg::CLIENT_PICS_PRODUCT_INFO_RESPONSE {
                        let resp =
                            generated::CMsgClientPicsProductInfoResponse::decode(&*sub_msg.body)?;
                        return Ok(resp
                            .apps
                            .iter()
                            .map(|a| AppInfo {
                                app_id: a.appid.map(AppId),
                                change_number: a.change_number,
                                kv_data: a.buffer.clone(),
                            })
                            .collect());
                    }
                }
            }
        }
    }

    pub async fn get_depot_decryption_key(
        &self,
        depot_id: DepotId,
        app_id: AppId,
    ) -> Result<DepotKey, Error> {
        let req = generated::CMsgClientGetDepotDecryptionKey {
            depot_id: Some(depot_id.0),
            app_id: Some(app_id.0),
        };
        let body = req.encode_to_vec();
        let msg = self.make_msg(EMsg::CLIENT_GET_DEPOT_DECRYPTION_KEY, &body); // k_EMsgClientGetDepotDecryptionKey
        self.send_raw(&msg).await?;

        loop {
            let incoming = self.recv_raw().await?;
            if incoming.emsg == EMsg::CLIENT_GET_DEPOT_DECRYPTION_KEY_RESPONSE {
                // k_EMsgClientGetDepotDecryptionKeyResponse
                let resp =
                    generated::CMsgClientGetDepotDecryptionKeyResponse::decode(&*incoming.body)?;
                crate::enums::eresult(resp.eresult.ok_or(ConnectionError::MissingField("eresult"))?)
                    .map_err(|_| ConnectionError::DepotAccessDenied(depot_id.0))?;
                let key_data = resp.depot_encryption_key.ok_or(ConnectionError::MissingField("depot_encryption_key"))?;
                if key_data.len() != 32 {
                    return Err(ConnectionError::EncryptionFailed.into());
                }
                let mut key = [0u8; 32];
                key.copy_from_slice(&key_data);
                return Ok(DepotKey(key));
            }
            if incoming.emsg == EMsg::MULTI {
                let msgs = multi::unpack_multi(&incoming.body)?;
                for sub in msgs {
                    let sub_msg = parse_incoming(&sub)?;
                    if sub_msg.emsg == EMsg::CLIENT_GET_DEPOT_DECRYPTION_KEY_RESPONSE {
                        let resp = generated::CMsgClientGetDepotDecryptionKeyResponse::decode(
                            &*sub_msg.body,
                        )?;
                        crate::enums::eresult(resp.eresult.ok_or(ConnectionError::MissingField("eresult"))?)
                            .map_err(|_| ConnectionError::DepotAccessDenied(depot_id.0))?;
                        let key_data = resp.depot_encryption_key.ok_or(ConnectionError::MissingField("depot_encryption_key"))?;
                        if key_data.len() != 32 {
                            return Err(ConnectionError::EncryptionFailed.into());
                        }
                        let mut key = [0u8; 32];
                        key.copy_from_slice(&key_data);
                        return Ok(DepotKey(key));
                    }
                }
            }
        }
    }

    pub async fn check_beta_password(
        &self,
        _app_id: AppId,
        _password: &str,
    ) -> Result<Vec<BetaBranch>, Error> {
        todo!()
    }

    pub async fn get_cdn_servers(
        &self,
        cell_id: CellId,
        max_servers: Option<u32>,
    ) -> Result<Vec<CdnServer>, Error> {
        let req = generated::CContentServerDirectoryGetServersForSteamPipeRequest {
            cell_id: Some(cell_id.0),
            max_servers: max_servers,
            ..Default::default()
        };
        let resp = self
            .call_service_method(
                "ContentServerDirectory.GetServersForSteamPipe#1",
                &req.encode_to_vec(),
            )
            .await?;
        let r: generated::CContentServerDirectoryGetServersForSteamPipeResponse = resp.decode()?;
        Ok(r.servers
            .iter()
            .filter_map(|s| {
                let host_str = s.host.as_deref()?;
                let https = s.https_support.as_deref() == Some("mandatory")
                    || s.https_support.as_deref() == Some("optional");
                let (host, port) = if let Some((h, p)) = host_str.rsplit_once(':') {
                    (h.to_string(), p.parse().unwrap_or(if https { 443 } else { 80 }))
                } else {
                    (host_str.to_string(), if https { 443 } else { 80 })
                };
                Some(CdnServer {
                    host,
                    port,
                    https,
                    vhost: s.vhost.clone().unwrap_or_default(),
                })
            })
            .collect())
    }

    pub async fn get_manifest_request_code(
        &self,
        app_id: AppId,
        depot_id: DepotId,
        manifest_id: ManifestId,
        branch: Option<&str>,
        branch_password_hash: Option<&str>,
    ) -> Result<Option<u64>, Error> {
        let req = generated::CContentServerDirectoryGetManifestRequestCodeRequest {
            app_id: Some(app_id.0),
            depot_id: Some(depot_id.0),
            manifest_id: Some(manifest_id.0),
            app_branch: branch.map(|s| s.to_string()),
            branch_password_hash: branch_password_hash.map(|s| s.to_string()),
        };
        let resp = self
            .call_service_method(
                "ContentServerDirectory.GetManifestRequestCode#1",
                &req.encode_to_vec(),
            )
            .await?;
        let r: generated::CContentServerDirectoryGetManifestRequestCodeResponse = resp.decode()?;
        Ok(r.manifest_request_code)
    }

    pub async fn get_cdn_auth_token(
        &self,
        app_id: AppId,
        depot_id: DepotId,
        host_name: &str,
    ) -> Result<CdnAuthToken, Error> {
        let req = generated::CContentServerDirectoryGetCdnAuthTokenRequest {
            depot_id: Some(depot_id.0),
            host_name: Some(host_name.to_string()),
            app_id: Some(app_id.0),
        };
        let resp = self
            .call_service_method(
                "ContentServerDirectory.GetCDNAuthToken#1",
                &req.encode_to_vec(),
            )
            .await?;
        let r: generated::CContentServerDirectoryGetCdnAuthTokenResponse = resp.decode()?;
        Ok(CdnAuthToken {
            token: r.token,
            expiration_time: r.expiration_time,
        })
    }
}

fn parse_incoming(data: &[u8]) -> Result<IncomingMsg, Error> {
    let parsed = header::PacketHeader::parse(data)?;
    match parsed {
        PacketHeader::Protobuf { header: h, body } => {
            let proto_header = h.decode_header().unwrap_or_default();
            Ok(IncomingMsg {
                emsg: h.emsg,
                is_protobuf: true,
                header: proto_header,
                body,
            })
        }
        PacketHeader::Simple { header: h, body } => Ok(IncomingMsg {
            emsg: h.emsg,
            is_protobuf: false,
            header: generated::CMsgProtoBufHeader::default(),
            body,
        }),
        PacketHeader::Extended { header: h, body } => Ok(IncomingMsg {
            emsg: h.emsg,
            is_protobuf: false,
            header: generated::CMsgProtoBufHeader {
                steamid: Some(h.steam_id),
                client_sessionid: Some(h.session_id),
                ..Default::default()
            },
            body,
        }),
    }
}

fn check_service_eresult(msg: &IncomingMsg) -> Result<(), Error> {
    if let Some(code) = msg.header.eresult {
        crate::enums::eresult(code)
            .map_err(ConnectionError::ServiceMethodFailed)?;
    }
    Ok(())
}

fn guard_type_from_proto(confirmation_type: Option<i32>) -> Option<GuardType> {
    GuardType::from_proto(confirmation_type?)
}
