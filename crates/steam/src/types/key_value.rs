use std::collections::BTreeMap;
use std::io::{Cursor, Read};
use byteorder::{LittleEndian, ReadBytesExt};

#[derive(Clone, Debug, serde::Serialize)]
pub struct KeyValue {
    pub key: String,
    pub value: KvValue,
}

#[derive(Clone, Debug, serde::Serialize)]
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
    pub fn from_binary(data: &[u8]) -> Result<Self, std::io::Error> {
        parse_binary_kv(data)
    }

    pub fn from_text(input: &str) -> Result<Self, TextKvError> {
        parse_text_kv(input)
    }

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
    let mut cursor = Cursor::new(data);
    read_binary_kv_node(&mut cursor)
}

fn read_binary_kv_node(cursor: &mut Cursor<&[u8]>) -> Result<KeyValue, std::io::Error> {
    let tag = cursor.read_u8()?;
    let tag = KvTag::from_u8(tag).ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidData, format!("unknown KV tag: {tag}"))
    })?;

    if tag == KvTag::None {
        // Subsection: key followed by children
        let key = read_cstring(cursor)?;
        let mut children = BTreeMap::new();
        loop {
            let peek = cursor.read_u8()?;
            let peek_tag = KvTag::from_u8(peek);
            if peek_tag.map_or(false, |t| t.is_end()) {
                break;
            }
            // Put the byte back by seeking
            cursor.set_position(cursor.position() - 1);
            let child = read_binary_kv_node(cursor)?;
            children.insert(child.key.clone(), child);
        }
        Ok(KeyValue {
            key,
            value: KvValue::Children(children),
        })
    } else {
        let key = read_cstring(cursor)?;
        let value = match tag {
            KvTag::String | KvTag::WideString => KvValue::String(read_cstring(cursor)?),
            KvTag::Int32 => KvValue::Int32(cursor.read_i32::<LittleEndian>()?),
            KvTag::Float32 => KvValue::Float32(cursor.read_f32::<LittleEndian>()?),
            KvTag::Color => KvValue::Color(cursor.read_i32::<LittleEndian>()?),
            KvTag::Pointer => KvValue::Pointer(cursor.read_i32::<LittleEndian>()?),
            KvTag::UInt64 => KvValue::UInt64(cursor.read_u64::<LittleEndian>()?),
            KvTag::Int64 => KvValue::Int64(cursor.read_i64::<LittleEndian>()?),
            _ => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("unexpected tag in value position: {tag:?}"),
                ))
            }
        };
        Ok(KeyValue { key, value })
    }
}

fn read_cstring(cursor: &mut Cursor<&[u8]>) -> Result<String, std::io::Error> {
    let mut bytes = Vec::new();
    loop {
        let b = cursor.read_u8()?;
        if b == 0 {
            break;
        }
        bytes.push(b);
    }
    String::from_utf8(bytes)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))
}

pub fn parse_text_kv(input: &str) -> Result<KeyValue, TextKvError> {
    let mut chars = input.chars().peekable();
    skip_whitespace(&mut chars);
    parse_text_kv_pair(&mut chars)
}

fn parse_text_kv_pair(
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
) -> Result<KeyValue, TextKvError> {
    let key = parse_text_string(chars)?;
    skip_whitespace(chars);

    match chars.peek() {
        Some('{') => {
            chars.next(); // consume '{'
            let mut children = BTreeMap::new();
            loop {
                skip_whitespace(chars);
                match chars.peek() {
                    Some('}') => {
                        chars.next();
                        break;
                    }
                    None => break,
                    _ => {
                        let child = parse_text_kv_pair(chars)?;
                        children.insert(child.key.clone(), child);
                    }
                }
            }
            Ok(KeyValue {
                key,
                value: KvValue::Children(children),
            })
        }
        _ => {
            let value = parse_text_string(chars)?;
            Ok(KeyValue {
                key,
                value: KvValue::String(value),
            })
        }
    }
}

fn parse_text_string(
    chars: &mut std::iter::Peekable<std::str::Chars<'_>>,
) -> Result<String, TextKvError> {
    skip_whitespace(chars);
    match chars.peek() {
        Some('"') => {
            chars.next(); // consume opening quote
            let mut s = String::new();
            loop {
                match chars.next() {
                    Some('"') => return Ok(s),
                    Some('\\') => match chars.next() {
                        Some('n') => s.push('\n'),
                        Some('t') => s.push('\t'),
                        Some('\\') => s.push('\\'),
                        Some('"') => s.push('"'),
                        Some(other) => {
                            s.push('\\');
                            s.push(other);
                        }
                        None => return Err(TextKvError::UnexpectedEof),
                    },
                    Some(c) => s.push(c),
                    None => return Err(TextKvError::UnexpectedEof),
                }
            }
        }
        Some(_) => {
            // Unquoted token
            let mut s = String::new();
            while let Some(&c) = chars.peek() {
                if c.is_whitespace() || c == '{' || c == '}' {
                    break;
                }
                s.push(c);
                chars.next();
            }
            if s.is_empty() {
                Err(TextKvError::UnexpectedEof)
            } else {
                Ok(s)
            }
        }
        None => Err(TextKvError::UnexpectedEof),
    }
}

fn skip_whitespace(chars: &mut std::iter::Peekable<std::str::Chars<'_>>) {
    while let Some(&c) = chars.peek() {
        if c.is_whitespace() {
            chars.next();
        } else if c == '/' {
            // Skip // comments
            let mut clone = chars.clone();
            clone.next();
            if clone.peek() == Some(&'/') {
                // Consume until newline
                for c in chars.by_ref() {
                    if c == '\n' {
                        break;
                    }
                }
            } else {
                break;
            }
        } else {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_text_kv_simple() {
        let input = r#""AppState"
{
    "appid"		"480"
    "name"		"Spacewar"
    "Universe"		"1"
}"#;
        let kv = parse_text_kv(input).unwrap();
        assert_eq!(kv.key, "AppState");
        let appid = kv.get("appid").unwrap();
        assert_eq!(appid.as_str(), Some("480"));
        let name = kv.get("name").unwrap();
        assert_eq!(name.as_str(), Some("Spacewar"));
    }

    #[test]
    fn parse_binary_kv_simple() {
        // Build a minimal binary KV: tag=0 "root" { tag=1 "key" "value" } end
        let mut data = Vec::new();
        data.push(0); // tag=None (subsection)
        data.extend_from_slice(b"root\0");
        data.push(1); // tag=String
        data.extend_from_slice(b"key\0");
        data.extend_from_slice(b"value\0");
        data.push(8); // End

        let kv = parse_binary_kv(&data).unwrap();
        assert_eq!(kv.key, "root");
        let child = kv.get("key").unwrap();
        assert_eq!(child.as_str(), Some("value"));
    }
}
