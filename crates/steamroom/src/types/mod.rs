/// 64-bit GameID with app, type, and mod fields.
pub mod game_id;
/// KeyValue text and binary format parsing.
pub mod key_value;
/// Binary KeyValue parser (winnow-based).
pub mod kv_binary;
/// Serde `Deserialize` implementation for KeyValue trees.
pub mod kv_de;
/// 64-bit SteamID with account, instance, type, and universe fields.
pub mod steam_id;

pub use game_id::GameId;
pub use key_value::KeyValue;
pub use key_value::KvTag;
pub use key_value::KvValue;
pub use kv_binary::BinaryKvError;
pub use kv_de::from_kv;
pub use kv_de::from_value;
pub use steam_id::SteamId;
