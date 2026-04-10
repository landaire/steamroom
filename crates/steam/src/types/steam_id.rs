#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub struct SteamId(u64);

impl SteamId {
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    pub const fn raw(self) -> u64 {
        self.0
    }

    pub const fn account_id(self) -> u32 {
        self.0 as u32
    }

    pub const fn instance(self) -> u32 {
        ((self.0 >> 32) & 0xF_FFFF) as u32
    }

    pub const fn account_type(self) -> u8 {
        ((self.0 >> 52) & 0xF) as u8
    }

    pub const fn universe(self) -> u8 {
        ((self.0 >> 56) & 0xFF) as u8
    }

    pub const fn from_parts(
        universe: u8,
        account_type: u8,
        instance: u32,
        account_id: u32,
    ) -> Self {
        let raw = (account_id as u64)
            | ((instance as u64 & 0xF_FFFF) << 32)
            | ((account_type as u64 & 0xF) << 52)
            | ((universe as u64) << 56);
        Self(raw)
    }
}

impl std::fmt::Display for SteamId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}
