#[derive(Clone, Debug)]
pub struct CdnAuthToken {
    pub token: Option<String>,
    pub expiration_time: Option<u32>,
}
