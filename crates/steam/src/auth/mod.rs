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

impl GuardType {
    pub fn from_proto(value: i32) -> Option<Self> {
        match value {
            1 => Some(Self::None),
            2 => Some(Self::EmailCode),
            3 => Some(Self::DeviceCode),
            4 => Some(Self::DeviceConfirmation),
            _ => Option::None,
        }
    }

    pub fn to_proto(self) -> i32 {
        match self {
            Self::None => 1,
            Self::EmailCode => 2,
            Self::DeviceCode => 3,
            Self::DeviceConfirmation => 4,
        }
    }
}
