//! Locale packs: the names, places, phone shape, currency and country the generator
//! draws from (design D1.5, D1.6). Built-in packs are YAML compiled in and load
//! through the same [`Pack::parse`] as a project's `locales/*.yaml`.

use std::fmt;
use std::marker::PhantomData;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use schemars::JsonSchema;
use serde::de::{self, Deserializer, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, de::value::MapAccessDeserializer};

/// A pack or data-file error, positioned when the parser knows where.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DataError {
    /// 1-based line, when known.
    pub line: Option<usize>,
    /// 1-based column, when known.
    pub col: Option<usize>,
    /// What is wrong.
    pub message: String,
    /// The file it is in, set by [`crate::DataSet::load`]. `None` from a bare `parse`, and
    /// from `load` for a problem with the config's own `data:` value (then `at` is `["data"]`).
    pub file: Option<PathBuf>,
    /// Without `line`: the YAML key path to place it at, a list index written as its number
    /// (`["places", "3"]`). Empty when unknown.
    pub at: Vec<String>,
    /// With `at`: the value there is wrong, so point at it rather than its key (R1.30).
    pub value: bool,
}

impl fmt::Display for DataError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (self.line, self.col) {
            (Some(l), Some(c)) => write!(f, "{l}:{c}: {}", self.message),
            _ => write!(f, "{}", self.message),
        }
    }
}

impl std::error::Error for DataError {}

impl DataError {
    /// An error serde cannot place, at the key path `at` (see [`DataError::at`]).
    pub(crate) fn unplaced(at: &[&str], message: String) -> Self {
        DataError {
            line: None,
            col: None,
            message,
            file: None,
            at: at.iter().map(|s| (*s).to_string()).collect(),
            value: false,
        }
    }

    /// Like [`DataError::unplaced`], for a bad value at `at`.
    pub(crate) fn at_value(at: &[&str], message: String) -> Self {
        DataError {
            value: true,
            ..DataError::unplaced(at, message)
        }
    }
}

/// A locale pack (`locales/<tag>.yaml`).
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Pack {
    /// Pack format version; only `1` exists.
    #[serde(default = "one")]
    pub version: u32,
    /// The locale tag, e.g. `en-IN`; a project pack's file is `locales/<tag>.yaml`.
    #[schemars(extend("pattern" = "^[A-Za-z]{2,3}(-[A-Za-z0-9]{2,8})*$"))]
    pub tag: String,
    /// Order of the parts of a full name.
    pub name_order: NameOrder,
    /// Country name, for `country` fields.
    pub country: String,
    /// Currency, for `currency` fields.
    pub currency: Currency,
    /// Phone number shape.
    pub phone: Phone,
    /// Address template over `{number}`, `{street}`, `{city}`, `{state}`, `{postcode}`.
    pub address: String,
    /// Given names.
    #[serde(deserialize_with = "non_empty")]
    #[schemars(with = "Vec<NameSchema>", length(min = 1))]
    pub first_names: Vec<Name>,
    /// Family names.
    #[serde(deserialize_with = "non_empty")]
    #[schemars(with = "Vec<NameSchema>", length(min = 1))]
    pub last_names: Vec<Name>,
    /// Street names for `{street}`.
    #[serde(deserialize_with = "non_empty")]
    #[schemars(length(min = 1))]
    pub streets: Vec<String>,
    /// Places; city, state and postcode of one object always come from one row.
    #[serde(deserialize_with = "non_empty")]
    #[schemars(length(min = 1))]
    pub places: Vec<Place>,
    /// Coordinate bounds for `latitude`/`longitude` fields; without it, the whole globe.
    #[serde(default)]
    pub geo: Option<Geo>,
}

/// Coordinate bounds of a pack's country or region, `[min, max]` in degrees.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Geo {
    /// Latitude `[min, max]`, within -90 to 90.
    pub lat: [f64; 2],
    /// Longitude `[min, max]`, within -180 to 180.
    pub lon: [f64; 2],
}

/// The JSON Schema (draft 2020-12) of a locale pack file, for `kaanal schema pack`.
pub fn pack_schema() -> serde_json::Value {
    schemars::generate::SchemaSettings::draft2020_12()
        .into_generator()
        .into_root_schema_for::<Pack>()
        .to_value()
}

/// **Private.** serde default for [`Pack::version`].
fn one() -> u32 {
    1
}

/// Whether a full name is `given family` or `family given`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum NameOrder {
    /// `Anna Schmidt`
    GivenFamily,
    /// `Schmidt Anna`
    FamilyGiven,
}

/// Currency of a pack.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Currency {
    /// ISO 4217 code, e.g. `INR`.
    pub code: String,
    /// Display symbol, e.g. `₹`.
    pub symbol: String,
}

/// Phone number shape: `prefix`, a space, then one of `formats`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Phone {
    /// Country calling code, e.g. `+91`.
    pub prefix: String,
    /// Number patterns; `{n}` is n seeded digits.
    #[serde(deserialize_with = "non_empty")]
    #[schemars(length(min = 1))]
    pub formats: Vec<String>,
}

/// One place row.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Place {
    /// City name.
    pub city: String,
    /// State or region, if the locale has one.
    #[serde(default)]
    pub state: Option<String>,
    /// Postcode pattern; `{n}` is n seeded digits.
    pub postcode: String,
}

/// A name in its native script, with an optional ASCII `latin` form for emails.
/// In YAML, a plain string or `{ name, latin }`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Name {
    /// The name as written.
    pub name: String,
    /// ASCII form for emails and usernames.
    pub latin: Option<String>,
}

/// **Private.** The YAML shape of a [`Name`], for the JSON Schema only.
#[derive(JsonSchema)]
#[serde(untagged)]
#[schemars(description = "A name, or `{ name, latin }` with an ASCII form for emails.")]
#[expect(dead_code, reason = "schema-only type, never constructed")]
enum NameSchema {
    Plain(String),
    Full { name: String, latin: Option<String> },
}

impl<'de> Deserialize<'de> for Name {
    // A visitor, not `#[serde(untagged)]`: untagged buffers the input and loses the position.
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Full {
            name: Text,
            latin: Option<Latin>,
        }
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = Name;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a name or { name, latin }")
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<Name, E> {
                Ok(Name {
                    name: non_empty_str(v, "name")?,
                    latin: None,
                })
            }
            fn visit_map<A: MapAccess<'de>>(self, map: A) -> Result<Name, A::Error> {
                let f = Full::deserialize(MapAccessDeserializer::new(map))?;
                Ok(Name {
                    name: f.name.0,
                    latin: f.latin.map(|l| l.0),
                })
            }
        }
        d.deserialize_any(V)
    }
}

/// **Private.** `v` owned, or an error naming `what` when it is empty.
fn non_empty_str<E: de::Error>(v: &str, what: &str) -> Result<String, E> {
    if v.is_empty() {
        return Err(E::custom(format!("{what} must not be empty")));
    }
    Ok(v.to_string())
}

/// **Private.** A non-empty `name:` value. Like [`Latin`], checked in the visitor so
/// the error carries the scalar's own position.
struct Text(String);

impl<'de> Deserialize<'de> for Text {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl Visitor<'_> for V {
            type Value = Text;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("a non-empty string")
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<Text, E> {
                non_empty_str(v, "name").map(Text)
            }
        }
        d.deserialize_str(V)
    }
}

/// **Private.** A `latin:` value; must be non-empty ASCII. Checked in the visitor so the
/// error carries the scalar's own position.
struct Latin(String);

impl<'de> Deserialize<'de> for Latin {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl Visitor<'_> for V {
            type Value = Latin;
            fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
                f.write_str("an ASCII string")
            }
            fn visit_str<E: de::Error>(self, v: &str) -> Result<Latin, E> {
                if v.is_ascii() {
                    non_empty_str(v, "latin").map(Latin)
                } else {
                    Err(E::custom(format!("`latin: {v}` must be ASCII")))
                }
            }
        }
        d.deserialize_str(V)
    }
}

/// **Private.** A list that must have at least one entry; the error is raised inside
/// the sequence so it carries the list's position.
pub(crate) fn non_empty<'de, D, T>(d: D) -> Result<Vec<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    struct V<T>(PhantomData<T>);
    impl<'de, T: Deserialize<'de>> Visitor<'de> for V<T> {
        type Value = Vec<T>;
        fn expecting(&self, f: &mut fmt::Formatter) -> fmt::Result {
            f.write_str("a non-empty list")
        }
        fn visit_seq<A: SeqAccess<'de>>(self, mut seq: A) -> Result<Vec<T>, A::Error> {
            let mut out = Vec::new();
            while let Some(x) = seq.next_element()? {
                out.push(x);
            }
            if out.is_empty() {
                return Err(de::Error::custom("list must not be empty"));
            }
            Ok(out)
        }
    }
    d.deserialize_seq(V(PhantomData))
}

/// Built-in pack sources, by tag; the one list to extend when a pack is added.
const BUILTIN: &[(&str, &str)] = &[
    ("en-IN", include_str!("../data/locales/en-IN.yaml")),
    ("en-US", include_str!("../data/locales/en-US.yaml")),
];

/// Tags of the packs compiled into Kaanal.
pub fn builtin_tags() -> &'static [&'static str] {
    static TAGS: OnceLock<Vec<&'static str>> = OnceLock::new();
    TAGS.get_or_init(|| BUILTIN.iter().map(|(t, _)| *t).collect())
}

impl Pack {
    /// Parses and checks a pack (design 11a rule 4): every block present, lists
    /// non-empty, `latin:` ASCII, templates use known placeholders, and an `address`
    /// with `{state}` only when every place has a state.
    ///
    /// # Errors
    /// A [`DataError`], with line and column when serde knows them; checks across
    /// blocks name the block or place instead.
    pub fn parse(src: &str) -> Result<Pack, DataError> {
        let src = src.strip_prefix('\u{feff}').unwrap_or(src);
        let p: Pack = serde_yaml_ng::from_str(src).map_err(|e| serde_error(&e))?;
        if p.version != 1 {
            return Err(DataError::at_value(
                &["version"],
                format!("version: {} is not supported; use 1", p.version),
            ));
        }
        let address_key =
            |k: &str| matches!(k, "number" | "street" | "city" | "state" | "postcode");
        if fill(&p.address, |k| address_key(k).then(String::new)).is_none() {
            return Err(DataError::at_value(
                &["address"],
                format!(
                    "address: `{}` has an unclosed brace or an unknown placeholder; use {{number}}, {{street}}, {{city}}, {{state}}, {{postcode}}",
                    p.address
                ),
            ));
        }
        for (i, pat) in p.phone.formats.iter().enumerate() {
            check_digits(pat, &["phone", "formats", &i.to_string()], "phone.formats")?;
        }
        for (i, pl) in p.places.iter().enumerate() {
            check_digits(
                &pl.postcode,
                &["places", &i.to_string(), "postcode"],
                "places.postcode",
            )?;
        }
        if let Some(g) = &p.geo {
            for (key, [lo, hi], limit) in [("lat", g.lat, 90.0), ("lon", g.lon, 180.0)] {
                let inside = |x: f64| (-limit..=limit).contains(&x);
                let message = if !inside(lo) || !inside(hi) {
                    format!("geo.{key}: [{lo}, {hi}] must be within -{limit} to {limit}")
                } else if lo > hi {
                    format!("geo.{key}: min {lo} is above max {hi}")
                } else {
                    continue;
                };
                return Err(DataError::at_value(&["geo", key], message));
            }
        }
        if p.address.contains("{state}")
            && let Some((i, pl)) = p
                .places
                .iter()
                .enumerate()
                .find(|(_, pl)| pl.state.is_none())
        {
            return Err(DataError::unplaced(
                &["places", &i.to_string()],
                format!(
                    "places: `{}` has no state, but address uses {{state}}; add a state or drop {{state}}",
                    pl.city
                ),
            ));
        }
        Ok(p)
    }

    /// A built-in pack by exact tag, parsed once and shared.
    pub fn builtin(tag: &str) -> Option<Arc<Pack>> {
        static CACHE: OnceLock<Vec<Arc<Pack>>> = OnceLock::new();
        let packs = CACHE.get_or_init(|| {
            BUILTIN
                .iter()
                .map(|(t, src)| {
                    Arc::new(Pack::parse(src).unwrap_or_else(|e| panic!("built-in {t}: {e}")))
                })
                .collect()
        });
        let i = BUILTIN.iter().position(|(t, _)| *t == tag)?;
        Some(packs[i].clone())
    }
}

/// **Private.** A phone or postcode pattern uses only `{n}` digit placeholders.
fn check_digits(pat: &str, at: &[&str], block: &str) -> Result<(), DataError> {
    match fill(pat, |k| digit_count(k).map(|_| String::new())) {
        Some(_) => Ok(()),
        None => Err(DataError::at_value(
            at,
            format!("{block}: `{pat}` may only use {{n}} for n digits, n from 1 to {MAX_DIGITS}"),
        )),
    }
}

/// **Private.** Largest `{n}` in a digit pattern.
const MAX_DIGITS: usize = 20;

/// **Private.** `"3"` → `Some(3)` for a digit placeholder in `1..=MAX_DIGITS`.
pub(crate) fn digit_count(key: &str) -> Option<usize> {
    // `parse` alone would take `+3`
    if !key.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    key.parse().ok().filter(|n| (1..=MAX_DIGITS).contains(n))
}

/// **Private.** Expands `{key}` placeholders left to right with `f`; `None` when a
/// brace is unclosed or `f` does not know a key.
pub(crate) fn fill(template: &str, mut f: impl FnMut(&str) -> Option<String>) -> Option<String> {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(i) = rest.find('{') {
        out.push_str(&rest[..i]);
        let end = i + rest[i..].find('}')?;
        out.push_str(&f(&rest[i + 1..end])?);
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    Some(out)
}

/// **Private.** A `serde_yaml_ng` error with its own position, without the position
/// repeated in the message (same as core's `serde_error`).
pub(crate) fn serde_error(e: &serde_yaml_ng::Error) -> DataError {
    let loc = e.location();
    let mut message = e.to_string();
    if let Some(l) = &loc {
        let at = format!(" at line {} column {}", l.line(), l.column());
        if let Some(m) = message.strip_suffix(&at) {
            message = m.to_string();
        }
    }
    DataError {
        line: loc.as_ref().map(serde_yaml_ng::Location::line),
        col: loc.as_ref().map(serde_yaml_ng::Location::column),
        message,
        file: None,
        at: Vec::new(),
        value: false,
    }
}

impl Pack {
    /// ASCII lower-case form of a name for emails and usernames: the `latin:` form if the
    /// name has one, else the name with diacritics folded (`José` → `jose`, `Straße` →
    /// `strasse`) and everything else dropped, else `user<n>` when nothing is left (`田中`).
    /// Never panics, whatever the Unicode input.
    ///
    /// Output is lower-case ASCII letters and digits only: spaces, apostrophes and hyphens
    /// are dropped from `latin:` forms and folded names alike.
    pub fn ascii_local(&self, name: &Name, n: u64) -> String {
        let src = name.latin.as_deref().unwrap_or(&name.name);
        let mut out = String::with_capacity(src.len());
        for c in src.chars() {
            if c.is_ascii_alphanumeric() {
                out.push(c.to_ascii_lowercase());
            } else if let Some(f) = fold(c) {
                out.push_str(f);
            }
        }
        if out.is_empty() {
            return format!("user{n}");
        }
        out
    }
}

/// **Private.** Hand-written fold of Latin-1 Supplement and Latin Extended-A letters to
/// lower-case ASCII. Anything else (CJK, emoji, combining marks, RTL) is `None`: dropped.
fn fold(c: char) -> Option<&'static str> {
    Some(match c {
        'À'..='Å' | 'à'..='å' | '\u{100}'..='\u{105}' => "a",
        'Æ' | 'æ' => "ae",
        'Ç' | 'ç' | '\u{106}'..='\u{10d}' => "c",
        'Ð' | 'ð' | '\u{10e}'..='\u{111}' => "d",
        'È'..='Ë' | 'è'..='ë' | '\u{112}'..='\u{11b}' => "e",
        '\u{11c}'..='\u{123}' => "g",
        '\u{124}'..='\u{127}' => "h",
        'Ì'..='Ï' | 'ì'..='ï' | '\u{128}'..='\u{131}' => "i",
        '\u{132}' | '\u{133}' => "ij",
        '\u{134}' | '\u{135}' => "j",
        '\u{136}'..='\u{138}' => "k",
        '\u{139}'..='\u{142}' => "l",
        'Ñ' | 'ñ' | '\u{143}'..='\u{149}' => "n",
        '\u{14a}' | '\u{14b}' => "ng",
        'Ò'..='Ö' | 'Ø' | 'ò'..='ö' | 'ø' | '\u{14c}'..='\u{151}' => "o",
        '\u{152}' | '\u{153}' => "oe",
        '\u{154}'..='\u{159}' => "r",
        '\u{15a}'..='\u{161}' | '\u{17f}' => "s",
        'ß' => "ss",
        'Þ' | 'þ' => "th",
        '\u{162}'..='\u{167}' => "t",
        'Ù'..='Ü' | 'ù'..='ü' | '\u{168}'..='\u{173}' => "u",
        '\u{174}' | '\u{175}' => "w",
        'Ý' | 'ý' | 'ÿ' | '\u{176}'..='\u{178}' => "y",
        '\u{179}'..='\u{17e}' => "z",
        _ => return None,
    })
}
