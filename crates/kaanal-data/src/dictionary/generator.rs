//! The closed set of generators a dictionary entry can name, and the rules for whether one fits a field.

use std::collections::BTreeMap;
use std::fmt;

use schemars::JsonSchema;
use serde::Deserialize;
use serde::de::{self, Deserializer, MapAccess, Visitor};
use serde_json::Value;

use crate::generate::count;
use crate::pack::{digit_count, fill, non_empty};

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
    Locale {},
    Language {},
    Url {},
    Uuid {},
    Integer {
        min: Option<i64>,
        max: Option<i64>,
        /// `uniform` (default) or `bands`: four equal bands weighted 25/30/25/20.
        #[serde(default)]
        dist: IntDist,
    },
    Money {
        min: Option<f64>,
        max: Option<f64>,
        /// `decade` (default): many cheap values, a few expensive; or `uniform`.
        #[serde(default)]
        dist: MoneyDist,
        /// `plain` (default) or `retail`: prices end in .99, .00, .95, .49.
        #[serde(default)]
        style: MoneyStyle,
    },
    Latitude {
        min: Option<f64>,
        max: Option<f64>,
    },
    Longitude {
        min: Option<f64>,
        max: Option<f64>,
    },
    Timestamp {
        /// Window in days before the base date (default 365); most values fall in its last third.
        days: Option<u32>,
    },
    Date {
        /// Window in days before the base date (default 365); most values fall in its last third.
        days: Option<u32>,
        /// A birth date instead: the person's age in whole years on the base date is drawn from
        /// these bounds (`[18, 80]`) by the four weighted bands, then a day inside that year.
        years: Option<[u32; 2]>,
    },
    Text {
        #[serde(default)]
        kind: TextKind,
    },
    Pick {
        #[serde(deserialize_with = "non_empty")]
        #[schemars(length(min = 1))]
        from: Vec<Value>,
        /// Relative weights by value (`paid: 8`); unlisted values weigh 1. Also weights the
        /// `enum` of a spec field with this name.
        #[serde(default)]
        #[schemars(with = "Option<BTreeMap<String, u32>>")]
        weights: Option<Weights>,
    },
    Template {
        value: String,
    },
    Static {
        value: Value,
    },
}

/// **Private.** How an `integer` entry spreads its values.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum IntDist {
    #[default]
    Uniform,
    Bands,
}

/// **Private.** How a `money` entry spreads its values.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MoneyDist {
    #[default]
    Decade,
    Uniform,
}

/// **Private.** How a `money` entry ends its values.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum MoneyStyle {
    #[default]
    Plain,
    Retail,
}

/// **Private.** `weights`: value (as JSON text) to weight. YAML keys such as `true` and `4` are
/// read as their text, so `{ true: 8, false: 2 }` works unquoted.
#[derive(Debug, Clone, Default)]
pub(crate) struct Weights(pub(crate) BTreeMap<String, u32>);

/// **Private.** A map key that is any scalar, read as text.
struct ScalarKey(String);

impl<'de> Deserialize<'de> for ScalarKey {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl Visitor<'_> for V {
            type Value = ScalarKey;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a value to weight")
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<ScalarKey, E> {
                Ok(ScalarKey(v.to_string()))
            }
            fn visit_bool<E: de::Error>(self, v: bool) -> Result<ScalarKey, E> {
                Ok(ScalarKey(v.to_string()))
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> Result<ScalarKey, E> {
                Ok(ScalarKey(v.to_string()))
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> Result<ScalarKey, E> {
                Ok(ScalarKey(v.to_string()))
            }
            fn visit_f64<E: de::Error>(self, v: f64) -> Result<ScalarKey, E> {
                // `1.0` weighs the value `1`, as a whole-number enum value does
                Ok(ScalarKey(if v.fract() == 0.0 && v.abs() < 1e15 {
                    format!("{v:.0}")
                } else {
                    v.to_string()
                }))
            }
        }
        d.deserialize_any(V)
    }
}

impl<'de> Deserialize<'de> for Weights {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Weights;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a map of value to weight, e.g. { paid: 8, failed: 1 }")
            }
            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Weights, A::Error> {
                let mut out = BTreeMap::new();
                while let Some((ScalarKey(k), w)) = map.next_entry::<ScalarKey, u32>()? {
                    out.insert(k, w);
                }
                Ok(Weights(out))
            }
        }
        d.deserialize_map(V)
    }
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
            Gen::Pick { from, .. } => from.iter().any(|v| value_fits(v, s, ty)),
            Gen::Static { value } => value_fits(value, s, ty),
            _ => ty == "string",
        }
    }

    /// **Private.** Whether every string a `pick`, `static` or `template` can emit for a
    /// field of type `ty` meets its `minLength`/`maxLength` in characters. Those values are
    /// emitted as written, never cut or padded; other generators are fitted or well-formed
    /// (R4.5), so they always pass.
    pub(crate) fn length_fits(&self, s: &Value, ty: &str) -> bool {
        let max = count(s, "maxLength", false);
        let min = count(s, "minLength", true);
        let ok = |n: usize| {
            let n = u64::try_from(n).unwrap_or(u64::MAX);
            max.is_none_or(|m| n <= m) && min.is_none_or(|m| n >= m)
        };
        match self {
            Gen::Pick { from, .. } => from
                .iter()
                .filter(|v| value_fits(v, s, ty))
                .filter_map(Value::as_str)
                .all(|t| ok(t.chars().count())),
            Gen::Static { value } => value.as_str().is_none_or(|t| ok(t.chars().count())),
            // placeholders have fixed widths, so a template has exactly one output length
            Gen::Template { value } => fill(value, |k| template_key(k).map(|(_, n)| "0".repeat(n)))
                .is_none_or(|t| ok(t.chars().count())),
            _ => true,
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
