pub mod header;

pub const EMSG_MASK: u32 = 0x7FFF_FFFF;
pub const PROTO_MASK: u32 = 0x8000_0000;

#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub struct EMsg(pub u32);

impl EMsg {
    pub const INVALID: Self = Self(0);
    pub const MULTI: Self = Self(1);
    pub const SERVICE_METHOD: Self = Self(146);
    pub const SERVICE_METHOD_RESPONSE: Self = Self(147);
    pub const SERVICE_METHOD_CALL_FROM_CLIENT: Self = Self(151);
    pub const SERVICE_METHOD_SEND_TO_CLIENT: Self = Self(152);
    pub const CLIENT_HEART_BEAT: Self = Self(703);
    pub const CLIENT_LOGOFF: Self = Self(706);
    pub const CLIENT_GAMES_PLAYED: Self = Self(716);
    pub const CLIENT_LOG_ON_RESPONSE: Self = Self(751);
    pub const CLIENT_SET_HEARTBEAT_RATE: Self = Self(755);
    pub const CLIENT_LOGGED_OFF: Self = Self(757);
    pub const CLIENT_PERSONA_STATE: Self = Self(766);
    pub const CLIENT_FRIENDS_LIST: Self = Self(767);
    pub const CLIENT_ACCOUNT_INFO: Self = Self(768);
    pub const CLIENT_LICENSE_LIST: Self = Self(780);
    pub const CLIENT_PING: Self = Self(781);
    pub const CLIENT_GET_APP_OWNERSHIP_TICKET: Self = Self(813);
    pub const CLIENT_GET_APP_OWNERSHIP_TICKET_RESPONSE: Self = Self(814);
    pub const CHANNEL_ENCRYPT_REQUEST: Self = Self(1303);
    pub const CHANNEL_ENCRYPT_RESPONSE: Self = Self(1304);
    pub const CHANNEL_ENCRYPT_RESULT: Self = Self(1305);
    pub const CLIENT_LOGON: Self = Self(5514);
    pub const CLIENT_GET_DEPOT_DECRYPTION_KEY: Self = Self(5438);
    pub const CLIENT_GET_DEPOT_DECRYPTION_KEY_RESPONSE: Self = Self(5439);
    pub const CLIENT_PICS_ACCESS_TOKEN_REQUEST: Self = Self(8905);
    pub const CLIENT_PICS_ACCESS_TOKEN_RESPONSE: Self = Self(8906);
    pub const CLIENT_PICS_PRODUCT_INFO_REQUEST: Self = Self(8903);
    pub const CLIENT_PICS_PRODUCT_INFO_RESPONSE: Self = Self(8904);
    pub const CLIENT_HELLO: Self = Self(9805);
}

impl std::fmt::Display for EMsg {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "EMsg({})", self.0)
    }
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct RawEMsg(pub u32);

impl RawEMsg {
    pub const fn is_protobuf(self) -> bool {
        self.0 & PROTO_MASK != 0
    }

    pub fn emsg(self) -> EMsg {
        EMsg(self.0 & EMSG_MASK)
    }

    pub const fn with_proto(emsg: EMsg) -> Self {
        Self(emsg.0 | PROTO_MASK)
    }

    pub const fn without_proto(emsg: EMsg) -> Self {
        Self(emsg.0)
    }
}
