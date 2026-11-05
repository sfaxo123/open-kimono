//! JSON metadata parser that rejects decoded duplicate keys before insertion.
use super::{ArtifactError, RESERVED_METADATA_KEY};
use serde::de::{DeserializeSeed, Error, MapAccess, SeqAccess, Visitor};
use serde_json::{Map, Number, Value};
use std::cell::RefCell;

#[derive(Clone, Copy)]
enum Level {
    Root,
    Descriptor,
    Metadata,
}

#[derive(Clone, Copy)]
struct Seed<'a> {
    level: Level,
    duplicate: &'a RefCell<Option<ArtifactError>>,
}

impl<'de> DeserializeSeed<'de> for Seed<'_> {
    type Value = Value;
    fn deserialize<D: serde::Deserializer<'de>>(self, de: D) -> Result<Value, D::Error> {
        de.deserialize_any(self)
    }
}

impl<'de> Visitor<'de> for Seed<'_> {
    type Value = Value;
    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("JSON metadata without duplicate keys")
    }
    fn visit_bool<E: Error>(self, value: bool) -> Result<Value, E> {
        Ok(Value::Bool(value))
    }
    fn visit_i64<E: Error>(self, value: i64) -> Result<Value, E> {
        Ok(value.into())
    }
    fn visit_u64<E: Error>(self, value: u64) -> Result<Value, E> {
        Ok(value.into())
    }
    fn visit_f64<E: Error>(self, value: f64) -> Result<Value, E> {
        Number::from_f64(value)
            .map(Value::Number)
            .ok_or_else(|| E::custom("nonfinite JSON number"))
    }
    fn visit_str<E: Error>(self, value: &str) -> Result<Value, E> {
        Ok(value.into())
    }
    fn visit_string<E: Error>(self, value: String) -> Result<Value, E> {
        Ok(value.into())
    }
    fn visit_unit<E: Error>(self) -> Result<Value, E> {
        Ok(Value::Null)
    }
    fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Value, A::Error> {
        let mut values = Vec::new();
        while let Some(value) = seq.next_element_seed(self)? {
            values.push(value);
        }
        Ok(Value::Array(values))
    }
    fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Value, A::Error> {
        let mut values = Map::new();
        while let Some(key) = map.next_key::<String>()? {
            if values.contains_key(&key) {
                *self.duplicate.borrow_mut() = Some(match self.level {
                    Level::Root => ArtifactError::DuplicateTensorName(key),
                    Level::Descriptor => ArtifactError::DuplicateDescriptorField(key),
                    Level::Metadata => ArtifactError::DuplicateMetadataKey(key),
                });
                return Err(A::Error::custom("duplicate decoded JSON key"));
            }
            let level = if matches!(self.level, Level::Root) && key == RESERVED_METADATA_KEY {
                Level::Metadata
            } else {
                Level::Descriptor
            };
            let value = map.next_value_seed(Seed { level, ..self })?;
            values.insert(key, value);
        }
        Ok(Value::Object(values))
    }
}

pub(super) fn parse_unique_object(header: &str) -> Result<Map<String, Value>, ArtifactError> {
    let duplicate = RefCell::new(None);
    let mut de = serde_json::Deserializer::from_str(header);
    let value = Seed {
        level: Level::Root,
        duplicate: &duplicate,
    }
    .deserialize(&mut de)
    .map_err(|error| {
        duplicate
            .take()
            .unwrap_or_else(|| ArtifactError::HeaderJson(error.to_string()))
    })?;
    de.end()
        .map_err(|error| ArtifactError::HeaderJson(error.to_string()))?;
    match value {
        Value::Object(map) => Ok(map),
        _ => Err(ArtifactError::HeaderNotObject),
    }
}
