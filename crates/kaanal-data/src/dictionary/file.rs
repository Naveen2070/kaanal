//! The dictionary file format (`data.yaml`): serde types, their JSON Schema and positioned errors.

use std::fmt;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::Deserialize;
use serde::de::value::{MapAccessDeserializer, SeqAccessDeserializer};
use serde::de::{self, Deserializer, MapAccess, SeqAccess, Visitor};
use serde_json::Value;

use super::check::check;
use super::generator::Gen;
use super::norm;
use crate::pack::non_empty;

/// A field dictionary file (`data.yaml`): field names mapped to generators.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(title = "Dictionary")]
pub(super) struct File {
    /// File format version; only `1` exists.
    // checked inside deserialization so the error carries a position
    #[serde(default = "one", deserialize_with = "version_one")]
    #[schemars(range(min = 1, max = 1))]
    #[expect(dead_code, reason = "only checked while parsing")]
    version: u32,
    /// Field name (normalised: case, `_` and `-` ignored; `*suffix` matches the last words)
    /// to one generator `{ gen: ... }` or a list of them, the first that fits the field's type
    /// applying.
    #[serde(default, deserialize_with = "fields_in_order")]
    #[schemars(schema_with = "fields_schema")]
    pub(super) fields: Vec<(String, Vec<Gen>)>,
    /// Extra time chains, earliest first: each list names fields of one object whose values
    /// are kept in that order (`[created_at, shipped_at]`). Appended to the built-in chains.
    #[serde(default)]
    #[schemars(with = "Vec<Vec<String>>")]
    pub(super) chains: Vec<Vec<Key>>,
    /// Field names that remove every built-in chain containing them.
    #[serde(default)]
    #[schemars(with = "Vec<String>")]
    pub(super) chains_off: Vec<Key>,
}

/// **Private.** Schema of `fields`: each entry one generator or a non-empty list of them.
fn fields_schema(g: &mut SchemaGenerator) -> Schema {
    let one = g.subschema_for::<Gen>();
    json_schema!({
        "type": "object",
        "additionalProperties": {
            "anyOf": [one.clone(), { "type": "array", "minItems": 1, "items": one }]
        }
    })
}

/// The JSON Schema (draft 2020-12) of a dictionary file, for `kaanal schema dictionary`.
pub fn dictionary_schema() -> Value {
    schemars::generate::SchemaSettings::draft2020_12()
        .into_generator()
        .into_root_schema_for::<File>()
        .to_value()
}

/// **Private.** serde default for [`File::version`].
fn one() -> u32 {
    1
}

/// **Private.** `version`, which must be `1`.
/// Checked in the visitor so the error carries the scalar's own position.
fn version_one<'de, D: Deserializer<'de>>(d: D) -> Result<u32, D::Error> {
    struct V;
    impl Visitor<'_> for V {
        type Value = u32;
        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("1")
        }
        fn visit_u64<E: de::Error>(self, v: u64) -> Result<u32, E> {
            if v == 1 {
                Ok(1)
            } else {
                Err(E::custom(format!("{v} is not supported; use 1")))
            }
        }
        fn visit_i64<E: de::Error>(self, v: i64) -> Result<u32, E> {
            Err(E::custom(format!("{v} is not supported; use 1")))
        }
    }
    d.deserialize_u64(V)
}

/// **Private.** `fields` in file order (so the later of two keys that normalise alike wins,
/// every time), each key checked to be non-empty after normalising.
fn fields_in_order<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<(String, Vec<Gen>)>, D::Error> {
    struct V;
    impl<'de> Visitor<'de> for V {
        type Value = Vec<(String, Vec<Gen>)>;
        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("a map of field names to generators")
        }
        fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
            let mut out = Vec::new();
            while let Some(Key(key)) = map.next_key()? {
                let Alternatives(gens) = map.next_value()?;
                out.push((key, gens));
            }
            Ok(out)
        }
    }
    d.deserialize_map(V)
}

/// **Private.** A `fields` key, non-empty after normalising. Checked in the visitor so the
/// error carries the key's own position.
pub(super) struct Key(pub(super) String);

impl<'de> Deserialize<'de> for Key {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl Visitor<'_> for V {
            type Value = Key;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a field name")
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<Key, E> {
                if norm(v.strip_prefix('*').unwrap_or(v)).is_empty() {
                    return Err(E::custom(format!("key `{v}` is empty after normalising")));
                }
                Ok(Key(v.to_string()))
            }
        }
        d.deserialize_str(V)
    }
}

/// **Private.** An entry: one generator, or a list tried in order; the first that fits
/// the field's type applies (`*id`: a uuid for strings, a number for integers).
struct Alternatives(Vec<Gen>);

impl<'de> Deserialize<'de> for Alternatives {
    // A visitor, not `#[serde(untagged)]`: untagged hides which generator or parameter is wrong.
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Alternatives;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("{ gen: ... } or a list of them")
            }
            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Alternatives, A::Error> {
                checked(vec![Gen::deserialize(MapAccessDeserializer::new(map))?])
            }
            fn visit_seq<A: SeqAccess<'de>>(self, seq: A) -> Result<Alternatives, A::Error> {
                checked(non_empty(SeqAccessDeserializer::new(seq))?)
            }
        }
        /// Parameter checks raised here, inside the entry, so the error carries its position.
        fn checked<E: de::Error>(gens: Vec<Gen>) -> Result<Alternatives, E> {
            for g in &gens {
                check(g).map_err(E::custom)?;
            }
            Ok(Alternatives(gens))
        }
        d.deserialize_any(V)
    }
}
