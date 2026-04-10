use std::collections::BTreeMap;

#[derive(Clone, Debug)]
pub struct KeyValue {
    pub key: String,
    pub value: KvValue,
}

#[derive(Clone, Debug)]
pub enum KvValue {
    String(String),
    Int32(i32),
    Float32(f32),
    UInt64(u64),
    Int64(i64),
    Color(i32),
    Pointer(i32),
    Children(BTreeMap<String, KeyValue>),
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum KvTag {
    None = 0,
    String = 1,
    Int32 = 2,
    Float32 = 3,
    Pointer = 4,
    WideString = 5,
    Color = 6,
    UInt64 = 7,
    End = 8,
    Int64 = 10,
    AltEnd = 11,
}

#[derive(Debug, thiserror::Error)]
pub enum TextKvError {
    #[error("unexpected EOF")]
    UnexpectedEof,
    #[error("expected quote")]
    ExpectedQuote,
    #[error("expected open brace")]
    ExpectedOpenBrace,
}

impl KvTag {
    pub fn from_u8(b: u8) -> Option<Self> {
        match b {
            0 => Some(Self::None),
            1 => Some(Self::String),
            2 => Some(Self::Int32),
            3 => Some(Self::Float32),
            4 => Some(Self::Pointer),
            5 => Some(Self::WideString),
            6 => Some(Self::Color),
            7 => Some(Self::UInt64),
            8 => Some(Self::End),
            10 => Some(Self::Int64),
            11 => Some(Self::AltEnd),
            _ => None,
        }
    }

    pub fn is_end(self) -> bool {
        matches!(self, Self::End | Self::AltEnd)
    }
}

impl KeyValue {
    pub fn get(&self, key: &str) -> Option<&KeyValue> {
        match &self.value {
            KvValue::Children(map) => map.get(key),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match &self.value {
            KvValue::String(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_i32(&self) -> Option<i32> {
        match &self.value {
            KvValue::Int32(v) => Some(*v),
            _ => None,
        }
    }

    pub fn as_u64(&self) -> Option<u64> {
        match &self.value {
            KvValue::UInt64(v) => Some(*v),
            _ => None,
        }
    }
}

pub fn parse_binary_kv(data: &[u8]) -> Result<KeyValue, std::io::Error> {
    todo!()
}

pub fn parse_text_kv(input: &str) -> Result<KeyValue, TextKvError> {
    todo!()
}
