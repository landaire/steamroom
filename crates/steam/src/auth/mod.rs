#[derive(Clone, Debug)]
pub struct AuthSession {
    pub client_id: Option<u64>,
    pub request_id: Option<Vec<u8>>,
    pub poll_interval: Option<f32>,
    pub allowed_confirmations: Vec<GuardType>,
    pub steam_id: Option<u64>,
}

#[derive(Clone, Debug)]
pub struct QrAuthSession {
    pub client_id: Option<u64>,
    pub request_id: Option<Vec<u8>>,
    pub challenge_url: Option<String>,
    pub poll_interval: Option<f32>,
    pub allowed_confirmations: Vec<GuardType>,
}

#[derive(Clone, Debug)]
pub struct AuthTokens {
    pub access_token: String,
    pub refresh_token: String,
    pub account_name: Option<String>,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum GuardType {
    None,
    EmailCode,
    DeviceCode,
    DeviceConfirmation,
}
