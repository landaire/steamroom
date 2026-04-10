#[derive(Copy, Clone, Debug, Eq, PartialEq, Hash)]
pub struct GameId(u64);

impl GameId {
    pub const fn new(raw: u64) -> Self {
        Self(raw)
    }

    pub const fn from_app_id(app_id: u32) -> Self {
        Self(app_id as u64)
    }

    pub const fn raw(self) -> u64 {
        self.0
    }

    pub const fn app_id(self) -> u32 {
        (self.0 & 0xFF_FFFF) as u32
    }

    pub const fn game_type(self) -> u8 {
        ((self.0 >> 24) & 0xFF) as u8
    }

    pub const fn mod_id(self) -> u32 {
        ((self.0 >> 32) & 0xFFFF_FFFF) as u32
    }
}
