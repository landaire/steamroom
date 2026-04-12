use std::collections::BTreeMap;
use winnow::binary::le_f32;
use winnow::binary::le_i32;
use winnow::binary::le_i64;
use winnow::binary::le_u8;
use winnow::binary::le_u64;
use winnow::error::ContextError;
use winnow::prelude::*;
use winnow::token::take_till;

use super::key_value::KeyValue;
use super::key_value::KvTag;
use super::key_value::KvValue;

#[derive(Debug, thiserror::Error)]
pub enum BinaryKvError {
    #[error("parse error: {0}")]
    Parse(String),
}

pub fn parse(data: &[u8]) -> Result<KeyValue, BinaryKvError> {
    kv_node
        .parse(data)
        .map_err(|e| BinaryKvError::Parse(e.to_string()))
}

fn cut() -> winnow::error::ErrMode<ContextError> {
    winnow::error::ErrMode::Cut(ContextError::new())
}

fn kv_node(input: &mut &[u8]) -> ModalResult<KeyValue> {
    let tag_byte = le_u8.parse_next(input)?;
    let tag = KvTag::from_u8(tag_byte).ok_or_else(cut)?;
    let key = cstring(input)?;

    if tag == KvTag::None {
        let children = kv_children(input)?;
        Ok(KeyValue {
            key,
            value: KvValue::Children(children),
        })
    } else {
        let value = kv_value(tag, input)?;
        Ok(KeyValue { key, value })
    }
}

fn kv_children(input: &mut &[u8]) -> ModalResult<BTreeMap<String, KeyValue>> {
    let mut children = BTreeMap::new();
    loop {
        let tag_byte = le_u8.parse_next(input)?;
        if let Some(tag) = KvTag::from_u8(tag_byte) {
            if tag.is_end() {
                break;
            }
            let key = cstring(input)?;
            let value = if tag == KvTag::None {
                KvValue::Children(kv_children(input)?)
            } else {
                kv_value(tag, input)?
            };
            children.insert(key.clone(), KeyValue { key, value });
        } else {
            return Err(cut());
        }
    }
    Ok(children)
}

fn kv_value(tag: KvTag, input: &mut &[u8]) -> ModalResult<KvValue> {
    match tag {
        KvTag::String | KvTag::WideString => Ok(KvValue::String(cstring(input)?)),
        KvTag::Int32 => Ok(KvValue::Int32(le_i32.parse_next(input)?)),
        KvTag::Float32 => Ok(KvValue::Float32(le_f32.parse_next(input)?)),
        KvTag::Color => Ok(KvValue::Color(le_i32.parse_next(input)?)),
        KvTag::Pointer => Ok(KvValue::Pointer(le_i32.parse_next(input)?)),
        KvTag::UInt64 => Ok(KvValue::UInt64(le_u64.parse_next(input)?)),
        KvTag::Int64 => Ok(KvValue::Int64(le_i64.parse_next(input)?)),
        _ => Err(cut()),
    }
}

fn cstring(input: &mut &[u8]) -> ModalResult<String> {
    let bytes = take_till(0.., b'\0').parse_next(input)?;
    let _ = le_u8.parse_next(input)?; // null terminator
    String::from_utf8(bytes.to_vec()).map_err(|_| cut())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple() {
        let mut data = Vec::new();
        data.push(0u8);
        data.extend_from_slice(b"root\0");
        data.push(1u8);
        data.extend_from_slice(b"key\0");
        data.extend_from_slice(b"value\0");
        data.push(8u8);

        let kv = parse(&data).unwrap();
        assert_eq!(kv.key, "root");
        assert_eq!(kv.get("key").unwrap().as_str(), Some("value"));
    }

    #[test]
    fn parse_numeric_types() {
        let mut data = Vec::new();
        data.push(0u8);
        data.extend_from_slice(b"root\0");
        data.push(2u8);
        data.extend_from_slice(b"num\0");
        data.extend_from_slice(&42i32.to_le_bytes());
        data.push(7u8);
        data.extend_from_slice(b"big\0");
        data.extend_from_slice(&999999u64.to_le_bytes());
        data.push(8u8);

        let kv = parse(&data).unwrap();
        assert_eq!(kv.get("num").unwrap().as_i32(), Some(42));
        assert_eq!(kv.get("big").unwrap().as_u64(), Some(999999));
    }

    #[test]
    fn parse_nested() {
        let mut data = Vec::new();
        data.push(0u8);
        data.extend_from_slice(b"root\0");
        data.push(0u8); // nested subsection
        data.extend_from_slice(b"child\0");
        data.push(1u8);
        data.extend_from_slice(b"val\0");
        data.extend_from_slice(b"inner\0");
        data.push(8u8); // end child
        data.push(8u8); // end root

        let kv = parse(&data).unwrap();
        let child = kv.get("child").unwrap();
        assert_eq!(child.get("val").unwrap().as_str(), Some("inner"));
    }

    #[test]
    fn unknown_tag_errors() {
        let data = [0xFF, b'x', 0];
        assert!(parse(&data).is_err());
    }
}
