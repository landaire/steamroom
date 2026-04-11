use serde::de;
use serde::de::DeserializeSeed;
use serde::de::MapAccess;
use serde::de::Visitor;

use serde::Deserializer;
use std::fmt;

use super::key_value::KeyValue;
use super::key_value::KvValue;

#[derive(Debug)]
pub enum KvDeError {
    Custom(String),
    TypeMismatch {
        expected: &'static str,
        got: &'static str,
    },
}

impl fmt::Display for KvDeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Custom(msg) => write!(f, "{msg}"),
            Self::TypeMismatch { expected, got } => {
                write!(f, "expected {expected}, got {got}")
            }
        }
    }
}

impl std::error::Error for KvDeError {}

impl de::Error for KvDeError {
    fn custom<T: fmt::Display>(msg: T) -> Self {
        Self::Custom(msg.to_string())
    }
}

pub fn from_kv<'de, T: serde::Deserialize<'de>>(kv: &'de KeyValue) -> Result<T, KvDeError> {
    T::deserialize(KvValueDeserializer(&kv.value))
}

pub fn from_value<'de, T: serde::Deserialize<'de>>(value: &'de KvValue) -> Result<T, KvDeError> {
    T::deserialize(KvValueDeserializer(value))
}

struct KvValueDeserializer<'a>(&'a KvValue);

impl<'de> Deserializer<'de> for KvValueDeserializer<'de> {
    type Error = KvDeError;

    fn deserialize_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        match self.0 {
            KvValue::String(s) => visitor.visit_borrowed_str(s),
            KvValue::Int32(v) => visitor.visit_i32(*v),
            KvValue::Float32(v) => visitor.visit_f32(*v),
            KvValue::UInt64(v) => visitor.visit_u64(*v),
            KvValue::Int64(v) => visitor.visit_i64(*v),
            KvValue::Color(v) => visitor.visit_i32(*v),
            KvValue::Pointer(v) => visitor.visit_i32(*v),
            KvValue::Children(_) => self.deserialize_map(visitor),
        }
    }

    fn deserialize_bool<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        match self.0 {
            KvValue::String(s) => match s.as_str() {
                "1" | "true" => visitor.visit_bool(true),
                "0" | "false" => visitor.visit_bool(false),
                _ => Err(KvDeError::Custom(format!("cannot parse '{s}' as bool"))),
            },
            KvValue::Int32(v) => visitor.visit_bool(*v != 0),
            _ => Err(KvDeError::TypeMismatch {
                expected: "bool",
                got: kv_type_name(self.0),
            }),
        }
    }

    fn deserialize_u32<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        match self.0 {
            KvValue::String(s) => visitor.visit_u32(
                s.parse()
                    .map_err(|_| KvDeError::Custom(format!("cannot parse '{s}' as u32")))?,
            ),
            KvValue::Int32(v) => visitor.visit_u32(*v as u32),
            KvValue::UInt64(v) => visitor.visit_u32(*v as u32),
            _ => Err(KvDeError::TypeMismatch {
                expected: "u32",
                got: kv_type_name(self.0),
            }),
        }
    }

    fn deserialize_u64<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        match self.0 {
            KvValue::String(s) => visitor.visit_u64(
                s.parse()
                    .map_err(|_| KvDeError::Custom(format!("cannot parse '{s}' as u64")))?,
            ),
            KvValue::UInt64(v) => visitor.visit_u64(*v),
            KvValue::Int64(v) => visitor.visit_u64(*v as u64),
            KvValue::Int32(v) => visitor.visit_u64(*v as u64),
            _ => Err(KvDeError::TypeMismatch {
                expected: "u64",
                got: kv_type_name(self.0),
            }),
        }
    }

    fn deserialize_i32<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        match self.0 {
            KvValue::String(s) => visitor.visit_i32(
                s.parse()
                    .map_err(|_| KvDeError::Custom(format!("cannot parse '{s}' as i32")))?,
            ),
            KvValue::Int32(v) => visitor.visit_i32(*v),
            _ => Err(KvDeError::TypeMismatch {
                expected: "i32",
                got: kv_type_name(self.0),
            }),
        }
    }

    fn deserialize_i64<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        match self.0 {
            KvValue::String(s) => visitor.visit_i64(
                s.parse()
                    .map_err(|_| KvDeError::Custom(format!("cannot parse '{s}' as i64")))?,
            ),
            KvValue::Int64(v) => visitor.visit_i64(*v),
            KvValue::Int32(v) => visitor.visit_i64(*v as i64),
            _ => Err(KvDeError::TypeMismatch {
                expected: "i64",
                got: kv_type_name(self.0),
            }),
        }
    }

    fn deserialize_f32<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        match self.0 {
            KvValue::Float32(v) => visitor.visit_f32(*v),
            KvValue::String(s) => visitor.visit_f32(
                s.parse()
                    .map_err(|_| KvDeError::Custom(format!("cannot parse '{s}' as f32")))?,
            ),
            _ => Err(KvDeError::TypeMismatch {
                expected: "f32",
                got: kv_type_name(self.0),
            }),
        }
    }

    fn deserialize_f64<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        match self.0 {
            KvValue::Float32(v) => visitor.visit_f64(*v as f64),
            KvValue::String(s) => visitor.visit_f64(
                s.parse()
                    .map_err(|_| KvDeError::Custom(format!("cannot parse '{s}' as f64")))?,
            ),
            _ => Err(KvDeError::TypeMismatch {
                expected: "f64",
                got: kv_type_name(self.0),
            }),
        }
    }

    fn deserialize_str<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        match self.0 {
            KvValue::String(s) => visitor.visit_borrowed_str(s),
            _ => Err(KvDeError::TypeMismatch {
                expected: "string",
                got: kv_type_name(self.0),
            }),
        }
    }

    fn deserialize_string<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        self.deserialize_str(visitor)
    }

    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        visitor.visit_some(self)
    }

    fn deserialize_map<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        match self.0 {
            KvValue::Children(map) => visitor.visit_map(KvMapAccess {
                iter: map.iter(),
                value: None,
            }),
            _ => Err(KvDeError::TypeMismatch {
                expected: "map/struct",
                got: kv_type_name(self.0),
            }),
        }
    }

    fn deserialize_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        _fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Self::Error> {
        self.deserialize_map(visitor)
    }

    fn deserialize_ignored_any<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Self::Error> {
        visitor.visit_unit()
    }

    serde::forward_to_deserialize_any! {
        i8 i16 u8 u16 i128 u128 char bytes byte_buf unit unit_struct
        newtype_struct seq tuple tuple_struct enum identifier
    }
}

struct KvMapAccess<'a> {
    iter: std::collections::btree_map::Iter<'a, String, KeyValue>,
    value: Option<&'a KvValue>,
}

impl<'de> MapAccess<'de> for KvMapAccess<'de> {
    type Error = KvDeError;

    fn next_key_seed<K: DeserializeSeed<'de>>(
        &mut self,
        seed: K,
    ) -> Result<Option<K::Value>, Self::Error> {
        match self.iter.next() {
            Some((key, kv)) => {
                self.value = Some(&kv.value);
                seed.deserialize(de::value::BorrowedStrDeserializer::new(key))
                    .map(Some)
            }
            None => Ok(None),
        }
    }

    fn next_value_seed<V: DeserializeSeed<'de>>(
        &mut self,
        seed: V,
    ) -> Result<V::Value, Self::Error> {
        let value = self
            .value
            .take()
            .ok_or_else(|| KvDeError::Custom("value without key".into()))?;
        seed.deserialize(KvValueDeserializer(value))
    }
}

fn kv_type_name(v: &KvValue) -> &'static str {
    match v {
        KvValue::String(_) => "String",
        KvValue::Int32(_) => "Int32",
        KvValue::Float32(_) => "Float32",
        KvValue::UInt64(_) => "UInt64",
        KvValue::Int64(_) => "Int64",
        KvValue::Color(_) => "Color",
        KvValue::Pointer(_) => "Pointer",
        KvValue::Children(_) => "Children",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::key_value::parse_text_kv;

    #[test]
    fn deserialize_flat_struct() {
        #[derive(serde::Deserialize, Debug, PartialEq)]
        struct AppState {
            appid: String,
            name: String,
            #[serde(rename = "Universe")]
            universe: String,
        }

        let input = r#""AppState"
{
    "appid"     "480"
    "name"      "Spacewar"
    "Universe"  "1"
}"#;
        let kv = parse_text_kv(input).unwrap();
        let app: AppState = from_kv(&kv).unwrap();
        assert_eq!(app.appid, "480");
        assert_eq!(app.name, "Spacewar");
        assert_eq!(app.universe, "1");
    }

    #[test]
    fn deserialize_numeric_from_string() {
        #[derive(serde::Deserialize, Debug)]
        struct Info {
            appid: u32,
            buildid: u64,
        }

        let input = r#""info"
{
    "appid"     "480"
    "buildid"   "3538192"
}"#;
        let kv = parse_text_kv(input).unwrap();
        let info: Info = from_kv(&kv).unwrap();
        assert_eq!(info.appid, 480);
        assert_eq!(info.buildid, 3538192);
    }

    #[test]
    fn deserialize_nested_struct() {
        #[derive(serde::Deserialize, Debug)]
        struct Root {
            depots: Depots,
        }

        #[derive(serde::Deserialize, Debug)]
        struct Depots {
            #[serde(rename = "481")]
            depot_481: Depot,
        }

        #[derive(serde::Deserialize, Debug)]
        struct Depot {
            manifests: Manifests,
        }

        #[derive(serde::Deserialize, Debug)]
        struct Manifests {
            public: Branch,
        }

        #[derive(serde::Deserialize, Debug)]
        struct Branch {
            gid: String,
        }

        let input = r#""root"
{
    "depots"
    {
        "481"
        {
            "manifests"
            {
                "public"
                {
                    "gid"   "3183503801510301321"
                }
            }
        }
    }
}"#;
        let kv = parse_text_kv(input).unwrap();
        let root: Root = from_kv(&kv).unwrap();
        assert_eq!(
            root.depots.depot_481.manifests.public.gid,
            "3183503801510301321"
        );
    }

    #[test]
    fn deserialize_optional_fields() {
        #[derive(serde::Deserialize, Debug)]
        #[allow(dead_code)]
        struct Branch {
            buildid: String,
            #[serde(default)]
            description: Option<String>,
            #[serde(default)]
            pwdrequired: Option<String>,
        }

        let input = r#""branch"
{
    "buildid"   "3538192"
}"#;
        let kv = parse_text_kv(input).unwrap();
        let branch: Branch = from_kv(&kv).unwrap();
        assert_eq!(branch.buildid, "3538192");
        assert!(branch.description.is_none());
    }

    #[test]
    fn deserialize_bool_from_string() {
        #[derive(serde::Deserialize, Debug)]
        struct Config {
            pwdrequired: bool,
        }

        let input = r#""config" { "pwdrequired" "1" }"#;
        let kv = parse_text_kv(input).unwrap();
        let config: Config = from_kv(&kv).unwrap();
        assert!(config.pwdrequired);
    }
}
