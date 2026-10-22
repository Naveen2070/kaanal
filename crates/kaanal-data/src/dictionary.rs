//! The field dictionary (design D1.2, D1.3): a normalised field name maps to one or more
//! generators from a closed Rust set. The built-in dictionary is YAML compiled in and
//! loads through the same [`Dictionary::parse`] as a project's `data.yaml`.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, OnceLock};

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::Deserialize;
use serde::de::value::{MapAccessDeserializer, SeqAccessDeserializer};
use serde::de::{self, Deserializer, MapAccess, SeqAccess, Visitor};
use serde_json::Value;

use crate::ApiModel;
use crate::dataset::Options;
use crate::generate::{MAX_DEPTH, merge_all_of, scalar_type};
use crate::pack::{DataError, digit_count, fill, non_empty, serde_error};

/// A field dictionary: exact keys and `*suffix` keys, both normalised.
#[derive(Debug, Clone, Default)]
pub struct Dictionary {
    exact: HashMap<String, Vec<Gen>>,
    suffix: HashMap<String, Vec<Gen>>,
    warnings: Vec<String>,
}

/// **Private.** One generator and its parameters. The set is closed: a dictionary
/// cannot run code (D1.2). Parameter-less generators are empty struct variants so an
/// unknown parameter is an error, not silently ignored.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(tag = "gen", rename_all = "snake_case", deny_unknown_fields)]
#[schemars(description = "One generator: `gen` names it, the other keys are its parameters.")]
pub(crate) enum Gen {
    FirstName {},
    LastName {},
    FullName {},
    Email {},
    Username {},
    Phone {},
    City {},
    State {},
    Postcode {},
    Country {},
    Street {},
    Address {},
    Currency {},
    Url {},
    Uuid {},
    Integer {
        min: Option<i64>,
        max: Option<i64>,
    },
    Money {
        min: Option<f64>,
        max: Option<f64>,
    },
    Latitude {
        min: Option<f64>,
        max: Option<f64>,
    },
    Longitude {
        min: Option<f64>,
        max: Option<f64>,
    },
    Timestamp {},
    Date {},
    Text {
        #[serde(default)]
        kind: TextKind,
    },
    Pick {
        #[serde(deserialize_with = "non_empty")]
        #[schemars(length(min = 1))]
        from: Vec<Value>,
    },
    Template {
        value: String,
    },
    Static {
        value: Value,
    },
}

/// **Private.** Length of a `text` value.
#[derive(Debug, Clone, Copy, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[schemars(description = "Length of a `text` value: `sentence` (default) or `paragraph`.")]
pub(crate) enum TextKind {
    #[default]
    Sentence,
    Paragraph,
}

impl Gen {
    /// **Private.** Whether this generator's output can satisfy a field of scalar type
    /// `ty` with schema `s` (design 11a rule 2). Numbers clamp to bounds, so only `pick`
    /// and `static` values are checked against `minimum`/`maximum`.
    pub(crate) fn fits(&self, s: &Value, ty: &str) -> bool {
        match self {
            Gen::Integer { .. }
            | Gen::Money { .. }
            | Gen::Latitude { .. }
            | Gen::Longitude { .. } => {
                matches!(ty, "integer" | "number")
            }
            Gen::Pick { from } => from.iter().any(|v| value_fits(v, s, ty)),
            Gen::Static { value } => value_fits(value, s, ty),
            _ => ty == "string",
        }
    }
}

/// **Private.** A user value fits when its JSON type matches and a number is in bounds.
pub(crate) fn value_fits(v: &Value, s: &Value, ty: &str) -> bool {
    let type_ok = match ty {
        "string" => v.is_string(),
        // whole and representable as i64: `2.0` and `u64::MAX` do not fit
        "integer" => v.is_i64(),
        "number" => v.is_number(),
        "boolean" => v.is_boolean(),
        _ => false,
    };
    let Some(x) = v.as_f64() else { return type_ok };
    let excl = |k| s.get(k).and_then(Value::as_bool).unwrap_or(false);
    let min_ok = s.get("minimum").and_then(Value::as_f64).is_none_or(|m| {
        if excl("exclusiveMinimum") {
            x > m
        } else {
            x >= m
        }
    });
    let max_ok = s.get("maximum").and_then(Value::as_f64).is_none_or(|m| {
        if excl("exclusiveMaximum") {
            x < m
        } else {
            x <= m
        }
    });
    type_ok && min_ok && max_ok
}

/// **Private.** A `{digits:n}` / `{letters:n}` template placeholder → (letters?, n).
pub(crate) fn template_key(k: &str) -> Option<(bool, usize)> {
    let (kind, n) = k.split_once(':')?;
    let letters = match kind {
        "digits" => false,
        "letters" => true,
        _ => return None,
    };
    Some((letters, digit_count(n)?))
}

/// A field dictionary file (`data.yaml`): field names mapped to generators.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
#[schemars(title = "Dictionary")]
struct File {
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
    fields: Vec<(String, Vec<Gen>)>,
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
struct Key(String);

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

/// **Private.** `firstName`, `first_name`, `First-Name` → `firstname`.
pub(crate) fn norm(field: &str) -> String {
    field
        .chars()
        .filter(|c| *c != '_' && *c != '-')
        .flat_map(char::to_lowercase)
        .collect()
}

/// **Private.** Lower-case words of a field name, split at `_`, `-` and case changes:
/// `lastOrderID` → `last`, `order`, `id`; `IDCard` → `id`, `card`.
fn words(field: &str) -> Vec<String> {
    let cs: Vec<char> = field.chars().collect();
    let mut out = Vec::new();
    let mut cur = String::new();
    for (i, &c) in cs.iter().enumerate() {
        if c == '_' || c == '-' {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
            continue;
        }
        if c.is_uppercase() && !cur.is_empty() {
            let prev = cs[i - 1];
            let next_lower = cs.get(i + 1).is_some_and(|n| n.is_lowercase());
            if prev.is_lowercase() || prev.is_numeric() || (prev.is_uppercase() && next_lower) {
                out.push(std::mem::take(&mut cur));
            }
        }
        cur.extend(c.to_lowercase());
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// **Private.** The built-in dictionary source.
const BUILTIN: &str = include_str!("../data/dictionary.yaml");

impl Dictionary {
    /// Parses and checks a dictionary file (`version`, `fields`). An empty file is an
    /// empty dictionary. Keys are normalised; a key starting `*` matches a suffix of
    /// whole words (`*id` matches `userId`, not `paid`).
    ///
    /// # Errors
    /// A [`DataError`] with line and column: unknown generator or parameter (listing the
    /// valid ones), bad template, range or bound, empty key, unsupported version.
    pub fn parse(src: &str) -> Result<Dictionary, DataError> {
        let src = src.strip_prefix('\u{feff}').unwrap_or(src);
        let file: Option<File> = serde_yaml_ng::from_str(src).map_err(|e| serde_error(&e))?;
        let Some(file) = file else {
            return Ok(Dictionary::default());
        };
        let mut d = Dictionary::default();
        // (suffix?, normalised) → the original key that set it, to report collisions
        let mut seen: HashMap<(bool, String), String> = HashMap::new();
        for (key, gens) in file.fields {
            let (is_suffix, k) = match key.strip_prefix('*') {
                Some(rest) => (true, norm(rest)),
                None => (false, norm(&key)),
            };
            if let Some(prev) = seen.insert((is_suffix, k.clone()), key.clone()) {
                d.warnings.push(format!(
                    "fields: `{prev}` and `{key}` are the same key after normalising; the later `{key}` wins"
                ));
            }
            let map = if is_suffix {
                &mut d.suffix
            } else {
                &mut d.exact
            };
            map.insert(k, gens);
        }
        Ok(d)
    }

    /// The built-in dictionary, parsed once and shared.
    pub fn builtin() -> Arc<Dictionary> {
        static CACHE: OnceLock<Arc<Dictionary>> = OnceLock::new();
        CACHE
            .get_or_init(|| {
                Arc::new(
                    Dictionary::parse(BUILTIN)
                        .unwrap_or_else(|e| panic!("built-in dictionary: {e}")),
                )
            })
            .clone()
    }

    /// This dictionary with `project`'s keys added; a project key replaces the same key.
    /// Note the generator consults [`Options::dictionary_project`] *before* `format` and
    /// [`Options::dictionary`] after (D1.8), so keep the two apart when that matters.
    #[must_use]
    pub fn overlaid(&self, project: Dictionary) -> Dictionary {
        let mut d = self.clone();
        d.exact.extend(project.exact);
        d.suffix.extend(project.suffix);
        d
    }

    /// Load-time warnings, e.g. two keys that normalise alike (the later one wins).
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    /// Number of keys, exact and suffix.
    pub fn len(&self) -> usize {
        self.exact.len() + self.suffix.len()
    }

    /// Whether the dictionary has no keys.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Whether some key matches `field`.
    pub fn contains(&self, field: &str) -> bool {
        self.lookup(field).is_some()
    }

    /// **Private.** The entry for `field`: an exact key, else the longest whole-word suffix key.
    pub(crate) fn lookup(&self, field: &str) -> Option<&[Gen]> {
        if let Some(g) = self.exact.get(&norm(field)) {
            return Some(g);
        }
        let w = words(field);
        (1..=w.len())
            .rev()
            .find_map(|k| self.suffix.get(&w[w.len() - k..].concat()))
            .map(Vec::as_slice)
    }
}

/// **Private.** Checks a generator's parameters beyond their types.
fn check(g: &Gen) -> Result<(), String> {
    let ordered = |lo: Option<f64>, hi: Option<f64>| match (lo, hi) {
        _ if lo.into_iter().chain(hi).any(|x| !x.is_finite()) => {
            Err("min and max must be finite".to_string())
        }
        (Some(a), Some(b)) if a > b => Err(format!("min {a} is above max {b}")),
        _ => Ok(()),
    };
    match g {
        #[expect(
            clippy::cast_precision_loss,
            reason = "only compared, never used as a value"
        )]
        Gen::Integer { min, max } => ordered(min.map(|v| v as f64), max.map(|v| v as f64)),
        Gen::Money { min, max } | Gen::Latitude { min, max } | Gen::Longitude { min, max } => {
            ordered(*min, *max)
        }
        Gen::Template { value } => fill(value, |k| template_key(k).map(|_| String::new()))
            .map(|_| ())
            .ok_or_else(|| {
                format!(
                    "template `{value}` has an unclosed brace or an unknown placeholder; use {{digits:n}} or {{letters:n}}, n from 1 to 20"
                )
            }),
        _ => Ok(()),
    }
}

/// Startup warnings for project dictionary entries that match a response field but cannot
/// fit its type or bounds, so they are skipped (design 11a rule 2). One line per field,
/// e.g. `GET /users .age: ...`. Built-in entries that do not fit are skipped silently.
pub fn dictionary_warnings(model: &ApiModel, opts: &Options) -> Vec<String> {
    let Some(project) = opts.dictionary_project.as_deref() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for e in &model.endpoints {
        if let Some(s) = &e.schema {
            walk(
                s,
                None,
                &format!("{} {}", e.method, e.path),
                project,
                &mut out,
                0,
            );
        }
    }
    out
}

/// **Private.** Schema walk for [`dictionary_warnings`], following what the generator visits.
fn walk(
    s: &Value,
    field: Option<&str>,
    at: &str,
    d: &Dictionary,
    out: &mut Vec<String>,
    depth: usize,
) {
    if depth > MAX_DEPTH {
        return;
    }
    if s.get("allOf").is_some() {
        return walk(&merge_all_of(s).0, field, at, d, out, depth);
    }
    let Value::Object(m) = s else { return };
    // as the generator: examples and enums win, writeOnly properties are never returned
    if m.contains_key("example")
        || m.contains_key("enum")
        || m.contains_key("const")
        || m.get("writeOnly").and_then(Value::as_bool) == Some(true)
    {
        return;
    }
    let parts: Vec<&Value> = ["oneOf", "anyOf"]
        .iter()
        .filter_map(|k| m.get(*k).and_then(Value::as_array))
        .flatten()
        .collect();
    if !parts.is_empty() {
        // the generator takes a branch, never the schema around it
        for part in parts {
            walk(part, field, at, d, out, depth + 1);
        }
        return;
    }
    match scalar_type(s) {
        "object" => {
            for (name, p) in m
                .get("properties")
                .and_then(Value::as_object)
                .into_iter()
                .flatten()
            {
                walk(p, Some(name), &format!("{at} .{name}"), d, out, depth + 1);
            }
        }
        "array" => {
            if let Some(items) = m.get("items") {
                walk(items, None, at, d, out, depth + 1);
            }
        }
        ty => {
            if let Some(f) = field
                && let Some(gens) = d.lookup(f)
                && !gens.iter().any(|g| g.fits(s, ty))
            {
                let msg = format!(
                    "{at}: project dictionary entry for `{f}` does not fit type `{ty}` or its bounds; skipped"
                );
                if !out.contains(&msg) {
                    out.push(msg);
                }
            }
        }
    }
}
