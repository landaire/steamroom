// Protobuf-generated types will be included here once proto extraction is complete.
// For now, stub the types needed by the client module.

#[derive(Clone, prost::Message)]
pub struct CAuthenticationGetPasswordRsaPublicKeyResponse {
    #[prost(string, optional, tag = "1")]
    pub publickey_mod: Option<String>,
    #[prost(string, optional, tag = "2")]
    pub publickey_exp: Option<String>,
    #[prost(uint64, optional, tag = "3")]
    pub timestamp: Option<u64>,
}

#[derive(Clone, prost::Message)]
pub struct CAuthenticationBeginAuthSessionViaCredentialsRequest {
    #[prost(string, optional, tag = "1")]
    pub device_friendly_name: Option<String>,
    #[prost(string, optional, tag = "2")]
    pub account_name: Option<String>,
    #[prost(string, optional, tag = "3")]
    pub encrypted_password: Option<String>,
    #[prost(uint64, optional, tag = "4")]
    pub encryption_timestamp: Option<u64>,
    #[prost(bool, optional, tag = "5")]
    pub remember_login: Option<bool>,
    #[prost(enumeration = "i32", optional, tag = "6")]
    pub platform_type: Option<i32>,
    #[prost(enumeration = "i32", optional, tag = "7")]
    pub persistence: Option<i32>,
    #[prost(string, optional, tag = "8")]
    pub website_id: Option<String>,
}

#[derive(Clone, prost::Message)]
pub struct CAuthenticationBeginAuthSessionViaQrRequest {
    #[prost(string, optional, tag = "1")]
    pub device_friendly_name: Option<String>,
    #[prost(enumeration = "i32", optional, tag = "2")]
    pub platform_type: Option<i32>,
}

#[derive(Clone, prost::Message)]
pub struct CMsgProtoBufHeader {
    #[prost(fixed64, optional, tag = "1")]
    pub steamid: Option<u64>,
    #[prost(int32, optional, tag = "2")]
    pub client_sessionid: Option<i32>,
    #[prost(uint32, optional, tag = "3")]
    pub routing_appid: Option<u32>,
    #[prost(fixed64, optional, tag = "10")]
    pub jobid_source: Option<u64>,
    #[prost(fixed64, optional, tag = "11")]
    pub jobid_target: Option<u64>,
    #[prost(string, optional, tag = "12")]
    pub target_job_name: Option<String>,
    #[prost(int32, optional, tag = "13")]
    pub eresult: Option<i32>,
    #[prost(string, optional, tag = "14")]
    pub error_message: Option<String>,
}

#[derive(Clone, prost::Message)]
pub struct CMsgMulti {
    #[prost(uint32, optional, tag = "1")]
    pub size_unzipped: Option<u32>,
    #[prost(bytes = "vec", optional, tag = "2")]
    pub message_body: Option<Vec<u8>>,
}

#[derive(Clone, prost::Message)]
pub struct CMsgClientLogon {
    #[prost(uint32, optional, tag = "1")]
    pub protocol_version: Option<u32>,
    #[prost(uint32, optional, tag = "2")]
    pub deprecated_obfuscated_private_ip: Option<u32>,
    #[prost(uint32, optional, tag = "3")]
    pub cell_id: Option<u32>,
    #[prost(uint32, optional, tag = "4")]
    pub last_session_id: Option<u32>,
    #[prost(uint32, optional, tag = "5")]
    pub client_package_version: Option<u32>,
    #[prost(string, optional, tag = "50")]
    pub account_name: Option<String>,
    #[prost(string, optional, tag = "51")]
    pub password: Option<String>,
    #[prost(string, optional, tag = "60")]
    pub access_token: Option<String>,
}

#[derive(Clone, prost::Message)]
pub struct CMsgClientLogonResponse {
    #[prost(int32, optional, tag = "1")]
    pub eresult: Option<i32>,
    #[prost(int32, optional, tag = "2")]
    pub out_of_game_heartbeat_seconds: Option<i32>,
    #[prost(int32, optional, tag = "3")]
    pub in_game_heartbeat_seconds: Option<i32>,
    #[prost(fixed64, optional, tag = "7")]
    pub client_supplied_steamid: Option<u64>,
    #[prost(uint32, optional, tag = "8")]
    pub ip_public: Option<u32>,
    #[prost(uint32, optional, tag = "9")]
    pub server_time: Option<u32>,
    #[prost(uint32, optional, tag = "14")]
    pub cell_id: Option<u32>,
}

#[derive(Clone, prost::Message)]
pub struct ContentManifestPayload {
    #[prost(message, repeated, tag = "1")]
    pub mappings: Vec<content_manifest_payload::FileMapping>,
}

pub mod content_manifest_payload {
    #[derive(Clone, prost::Message)]
    pub struct FileMapping {
        #[prost(string, optional, tag = "1")]
        pub filename: Option<String>,
        #[prost(uint64, optional, tag = "2")]
        pub size: Option<u64>,
        #[prost(uint32, optional, tag = "3")]
        pub flags: Option<u32>,
        #[prost(bytes = "vec", optional, tag = "4")]
        pub sha_filename: Option<Vec<u8>>,
        #[prost(bytes = "vec", optional, tag = "5")]
        pub sha_content: Option<Vec<u8>>,
        #[prost(message, repeated, tag = "6")]
        pub chunks: Vec<file_mapping::ChunkData>,
        #[prost(string, optional, tag = "7")]
        pub linktarget: Option<String>,
    }

    pub mod file_mapping {
        #[derive(Clone, prost::Message)]
        pub struct ChunkData {
            #[prost(bytes = "vec", optional, tag = "1")]
            pub sha: Option<Vec<u8>>,
            #[prost(fixed32, optional, tag = "2")]
            pub crc: Option<u32>,
            #[prost(uint64, optional, tag = "3")]
            pub offset: Option<u64>,
            #[prost(uint32, optional, tag = "4")]
            pub cb_original: Option<u32>,
            #[prost(uint32, optional, tag = "5")]
            pub cb_compressed: Option<u32>,
        }
    }
}

#[derive(Clone, prost::Message)]
pub struct ContentManifestMetadata {
    #[prost(uint32, optional, tag = "1")]
    pub depot_id: Option<u32>,
    #[prost(uint64, optional, tag = "2")]
    pub gid_manifest: Option<u64>,
    #[prost(uint32, optional, tag = "3")]
    pub creation_time: Option<u32>,
    #[prost(bool, optional, tag = "4")]
    pub filenames_encrypted: Option<bool>,
    #[prost(uint64, optional, tag = "5")]
    pub cb_disk_original: Option<u64>,
    #[prost(uint64, optional, tag = "6")]
    pub cb_disk_compressed: Option<u64>,
    #[prost(uint32, optional, tag = "7")]
    pub unique_chunks: Option<u32>,
    #[prost(uint32, optional, tag = "8")]
    pub crc_encrypted: Option<u32>,
    #[prost(uint32, optional, tag = "9")]
    pub crc_clear: Option<u32>,
}

#[derive(Clone, prost::Message)]
pub struct ContentManifestSignature {
    #[prost(bytes = "vec", optional, tag = "1")]
    pub signature: Option<Vec<u8>>,
}
