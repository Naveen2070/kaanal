//! The data generator: deterministic, realistic JSON from an `OpenAPI` 3.0 schema.
//!
//! Same seed → same output, always. Field-pick priority: `example` → `enum` →
//! `allOf`/`oneOf`/`anyOf` → type, and within a type: `format` → field-name
//! heuristic → constraints → fallback. Every field derives its own seed from
//! `(parent_seed, name, index)`, so schema edits never reshuffle sibling values
//! (rule R4.4).

use rand::seq::IndexedRandom;
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use serde_json::{Map, Value, json};

use crate::words;

/// **Private.** Recursion ceiling; deeper nesting yields `null` instead of overflowing.
const MAX_DEPTH: usize = 16;
/// **Private.** Cap on random array length so `maxItems: 1000000` can't build a huge array.
const MAX_RANDOM_ITEMS: u64 = 20;
/// **Private.** Timestamps are "the last 365 days before 2026-01-01", fixed so output never changes between runs.
const BASE_EPOCH: i64 = 1_767_225_600;

/// Deterministic JSON for an `OpenAPI` 3.0 schema whose `$ref`s are already inlined.
/// Priority: example → enum → allOf/oneOf/anyOf → type (format → field-name heuristic → constraints → fallback).
pub fn generate(schema: &Value, field: Option<&str>, seed: u64) -> Value {
    value_for(schema, field, seed, seed, 0)
}

/// **Private.** FNV-1a 64 with a separator byte between parts. Same algorithm as
/// `kaanal_core::stable_hash` (copied: this crate must not depend on core). Not a
/// std hasher, so it never changes between Rust versions.
fn stable_hash(parts: &[&[u8]]) -> u64 {
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for part in parts {
        for b in *part {
            h ^= u64::from(*b);
            h = h.wrapping_mul(PRIME);
        }
        h ^= 0xff;
        h = h.wrapping_mul(PRIME);
    }
    h
}

/// **Private.** Each child value gets its own seed from (parent seed, name, index),
/// so adding or reordering schema properties never changes the values of the others.
fn child_seed(parent: u64, name: &str, index: usize) -> u64 {
    stable_hash(&[
        &parent.to_le_bytes(),
        name.as_bytes(),
        &(index as u64).to_le_bytes(),
    ])
}

/// Copy path params into same-named top-level fields when the type fits (`/users/{id}` → body `id`).
pub fn apply_params(body: &mut Value, params: &[(String, String)]) {
    let Value::Object(obj) = body else { return };
    for (k, raw) in params {
        let Some(existing) = obj.get_mut(k) else {
            continue;
        };
        match existing {
            Value::Number(_) => {
                if let Ok(n) = raw.parse::<i64>() {
                    *existing = json!(n);
                }
            }
            Value::String(_) => *existing = json!(raw),
            _ => {}
        }
    }
}

/// **Private.** The recursive dispatcher behind [`generate`].
///
/// `group` is the seed of the enclosing object: fields that must agree with each other
/// (city, state, pincode, address) derive their shared choice from it, not from their own seeds.
fn value_for(s: &Value, field: Option<&str>, seed: u64, group: u64, depth: usize) -> Value {
    if depth > MAX_DEPTH {
        return Value::Null;
    }
    let rng = &mut ChaCha8Rng::seed_from_u64(seed);
    if let Some(ex) = s.get("example") {
        return ex.clone();
    }
    if let Some(choices) = s
        .get("enum")
        .and_then(Value::as_array)
        .filter(|e| !e.is_empty())
    {
        return choices.choose(rng).cloned().unwrap_or(Value::Null);
    }
    if let Some(parts) = s.get("allOf").and_then(Value::as_array) {
        let mut merged = Map::new();
        for (i, part) in parts.iter().enumerate() {
            if let Value::Object(o) =
                value_for(part, field, child_seed(seed, "allOf", i), group, depth + 1)
            {
                merged.extend(o);
            }
        }
        return Value::Object(merged);
    }
    if let Some(first) = ["oneOf", "anyOf"]
        .iter()
        .find_map(|k| s.get(*k).and_then(Value::as_array)?.first())
    {
        return value_for(first, field, child_seed(seed, "oneOf", 0), group, depth + 1);
    }
    let default_type = if s.get("properties").is_some() {
        "object"
    } else {
        "string"
    };
    match s
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or(default_type)
    {
        "object" => {
            let mut out = Map::new();
            if let Some(props) = s.get("properties").and_then(Value::as_object) {
                for (name, ps) in props {
                    out.insert(
                        name.clone(),
                        value_for(ps, Some(name), child_seed(seed, name, 0), seed, depth + 1),
                    );
                }
            }
            Value::Object(out)
        }
        "array" => {
            let min = s.get("minItems").and_then(Value::as_u64).unwrap_or(1);
            // limitation: cap random length so `maxItems: 1000000` can't build a huge array; minItems is still honoured
            let max = s
                .get("maxItems")
                .and_then(Value::as_u64)
                .unwrap_or(5)
                .min(MAX_RANDOM_ITEMS)
                .max(min);
            let n = rng.random_range(min..=max);
            let items = s.get("items").cloned().unwrap_or(json!({}));
            let n = usize::try_from(n).expect("length is capped by MAX_RANDOM_ITEMS");
            Value::Array(
                (0..n)
                    .map(|i| value_for(&items, None, child_seed(seed, "[]", i), group, depth + 1))
                    .collect(),
            )
        }
        "integer" => json!(int_for(s, field, rng)),
        "number" => number_for(s, field, rng),
        "boolean" => json!(rng.random_bool(0.5)),
        _ => json!(string_for(s, field, rng, group)),
    }
}

/// **Private.** `id`, `userId`, `user_id`, `ID` are ids; `valid`, `paid`, `android` are not.
fn is_id(field: Option<&str>) -> bool {
    let f = field.unwrap_or("");
    f.eq_ignore_ascii_case("id")
        || f.ends_with("Id")
        || f.ends_with("ID")
        || f.ends_with("_id")
        || f.ends_with("-id")
}

/// **Private.** `firstName`, `first_name`, `first-name` → `firstname`
fn norm(field: Option<&str>) -> String {
    field
        .unwrap_or("")
        .chars()
        .filter(|c| *c != '_' && *c != '-')
        .flat_map(char::to_lowercase)
        .collect()
}

/// **Private.** Integer value: heuristic ranges for known names (age, quantity, ids),
/// bounded by `minimum`/`maximum` and their exclusive variants.
fn int_for(s: &Value, field: Option<&str>, rng: &mut ChaCha8Rng) -> i64 {
    let f = norm(field);
    let (mut lo, mut hi) = match f.as_str() {
        "age" => (18, 80),
        "quantity" | "qty" | "count" => (1, 10),
        _ if is_id(field) => (1, 99_999),
        _ => (0, 1000),
    };
    if let Some(m) = s.get("minimum").and_then(Value::as_i64) {
        lo = if exclusive(s, "exclusiveMinimum") {
            m.saturating_add(1)
        } else {
            m
        };
        if hi < lo {
            hi = lo.saturating_add(1000);
        }
    }
    if let Some(m) = s.get("maximum").and_then(Value::as_i64) {
        hi = if exclusive(s, "exclusiveMaximum") {
            m.saturating_sub(1)
        } else {
            m
        };
        lo = lo.min(hi);
    }
    rng.random_range(lo..=hi)
}

/// **Private.** `OpenAPI` 3.0: `exclusiveMinimum: true` turns `minimum` into a strict bound.
fn exclusive(s: &Value, key: &str) -> bool {
    s.get(key).and_then(Value::as_bool).unwrap_or(false)
}

/// **Private.** Float value: lat/lng ranges for geo fields, money ranges for
/// price-like names, bounds applied with one-cent slack for exclusive limits.
fn number_for(s: &Value, field: Option<&str>, rng: &mut ChaCha8Rng) -> Value {
    let f = norm(field);
    match f.as_str() {
        "lat" | "latitude" => return json!(round(rng.random_range(8.0..=35.0), 6)),
        "lng" | "lon" | "longitude" => return json!(round(rng.random_range(68.0..=97.0), 6)),
        _ => {}
    }
    let money = ["price", "total", "amount", "cost", "subtotal"]
        .iter()
        .any(|k| f.ends_with(k));
    let (dlo, dhi) = if money { (99.0, 4999.0) } else { (0.0, 1000.0) };
    // values are rounded to 2 places, so a strict bound is one cent inside
    let lo = match s.get("minimum").and_then(Value::as_f64) {
        Some(m) if exclusive(s, "exclusiveMinimum") => m + 0.01,
        Some(m) => m,
        None => dlo,
    };
    let hi = match s.get("maximum").and_then(Value::as_f64) {
        Some(m) if exclusive(s, "exclusiveMaximum") => (m - 0.01).max(lo),
        Some(m) => m.max(lo),
        None if dhi > lo => dhi,
        None => lo + 1000.0,
    };
    let v = if hi > lo {
        rng.random_range(lo..=hi)
    } else {
        lo
    };
    json!(round(v, 2).clamp(lo, hi))
}

/// **Private.** Rounds to `places` decimal digits.
fn round(v: f64, places: i32) -> f64 {
    let p = 10f64.powi(places);
    (v * p).round() / p
}

/// **Private.** String value: `format` first, then the field-name heuristic, then a
/// lorem word. Formats beat lengths (rule R4.5): `fit_length` is skipped for values
/// that must stay well-formed.
fn string_for(s: &Value, field: Option<&str>, rng: &mut ChaCha8Rng, group: u64) -> String {
    let v = match s.get("format").and_then(Value::as_str).unwrap_or("") {
        "email" => email(rng, group),
        "uuid" => uuid(rng),
        "date-time" => date_time(rng),
        "date" => date_time(rng)[..10].to_string(),
        "uri" | "url" => format!("https://example.com/{}", words::pick(words::WORDS, rng)),
        "ipv4" => format!(
            "10.{}.{}.{}",
            rng.random_range(0..=255),
            rng.random_range(0..=255),
            rng.random_range(1..=254)
        ),
        _ => heuristic(&norm(field), is_id(field), group, rng)
            .unwrap_or_else(|| words::pick(words::WORDS, rng).to_string()),
    };
    let structured = s.get("format").is_some() || keeps_format(&norm(field), is_id(field));
    // truncating or padding an email, uuid or timestamp would make it invalid; the format wins over lengths
    if structured { v } else { fit_length(v, s) }
}

/// **Private.** Heuristic fields whose value must stay well-formed (see `heuristic`).
fn keeps_format(f: &str, id_like: bool) -> bool {
    id_like
        || f.ends_with("email")
        || f.ends_with("phone")
        || f.ends_with("url")
        || matches!(
            f,
            "mobile" | "createdat" | "updatedat" | "deletedat" | "timestamp"
        )
}

/// **Private.** The (city, state, pincode) of the object a field belongs to: same
/// object, same place.
fn place(group: u64) -> (&'static str, &'static str, &'static str) {
    let i = stable_hash(&[&group.to_le_bytes(), b"place"]) % words::CITIES.len() as u64;
    words::CITIES[usize::try_from(i).expect("i < CITIES.len()")]
}

/// **Private.** The (first, last) name of the person an object describes: same
/// object, same person, so `firstName`, `lastName`, `name` and `email` agree.
fn person(group: u64) -> (&'static str, &'static str) {
    let pick = |list: &[&'static str], tag: &[u8]| {
        let i = stable_hash(&[&group.to_le_bytes(), tag]) % list.len() as u64;
        list[usize::try_from(i).expect("i < list.len()")]
    };
    (pick(words::FIRST, b"first"), pick(words::LAST, b"last"))
}

/// **Private.** The field-name dictionary. Returns `None` when nothing matches, so
/// the caller falls back to a lorem word.
fn heuristic(f: &str, id_like: bool, group: u64, rng: &mut ChaCha8Rng) -> Option<String> {
    Some(match f {
        "firstname" | "givenname" => person(group).0.to_string(),
        "lastname" | "surname" | "familyname" => person(group).1.to_string(),
        "name" | "fullname" => format!("{} {}", person(group).0, person(group).1),
        f if f.ends_with("email") => email(rng, group),
        f if f.ends_with("phone") || f == "mobile" => format!("+91 9{}", digits(rng, 9)),
        "city" => place(group).0.to_string(),
        "state" => place(group).1.to_string(),
        "pincode" | "zip" | "zipcode" | "postalcode" => place(group).2.to_string(),
        "country" => "India".to_string(),
        "address" | "street" => {
            format!(
                "{} {}, {}",
                rng.random_range(1..=250),
                words::pick(words::STREETS, rng),
                place(group).0
            )
        }
        "currency" => "INR".to_string(),
        "createdat" | "updatedat" | "deletedat" | "timestamp" => date_time(rng),
        "description" | "bio" | "summary" => sentence(rng),
        "status" => "active".to_string(),
        f if f.ends_with("url") => {
            format!(
                "https://example.com/{}/{}",
                f.trim_end_matches("url"),
                rng.random_range(1..=9999)
            )
        }
        _ if id_like => uuid(rng),
        _ => return None,
    })
}

/// **Private.** `first.lastNN@example.com` for the object's person — always a reserved domain (rule R5.7).
fn email(rng: &mut ChaCha8Rng, group: u64) -> String {
    let (first, last) = person(group);
    let (first, last) = (first.to_lowercase(), last.to_lowercase());
    format!("{first}.{last}{}@example.com", rng.random_range(1..=99))
}

/// **Private.** A random UUID with the v4 version and variant bits set.
fn uuid(rng: &mut ChaCha8Rng) -> String {
    use std::fmt::Write as _;
    let mut b: [u8; 16] = rng.random();
    b[6] = (b[6] & 0x0f) | 0x40;
    b[8] = (b[8] & 0x3f) | 0x80;
    let h: String = b.iter().fold(String::with_capacity(32), |mut s, x| {
        write!(s, "{x:02x}").expect("writing to a String cannot fail");
        s
    });
    format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    )
}

/// **Private.** `n` random decimal digits (phone numbers).
fn digits(rng: &mut ChaCha8Rng, n: usize) -> String {
    (0..n)
        .map(|_| char::from(b'0' + rng.random_range(0..10u8)))
        .collect()
}

/// **Private.** A 6–12 word lorem sentence, capitalised, ending with a full stop.
fn sentence(rng: &mut ChaCha8Rng) -> String {
    let n = rng.random_range(6..=12);
    let mut s: Vec<&str> = (0..n).map(|_| words::pick(words::WORDS, rng)).collect();
    let first = s[0][..1].to_uppercase() + &s[0][1..];
    s[0] = "";
    format!("{first}{}.", s.join(" "))
}

/// **Private.** An RFC 3339 UTC timestamp within the 365 days before [`BASE_EPOCH`].
fn date_time(rng: &mut ChaCha8Rng) -> String {
    let t = BASE_EPOCH - rng.random_range(0..365 * 86_400i64);
    let (days, secs) = (t.div_euclid(86_400), t.rem_euclid(86_400));
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        secs / 3600,
        secs % 3600 / 60,
        secs % 60
    )
}

/// **Private.** Days since 1970-01-01 → (year, month, day). Howard Hinnant's algorithm.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = u32::try_from(doy - (153 * mp + 2) / 5 + 1).expect("day in 1..=31");
    let m = u32::try_from(if mp < 10 { mp + 3 } else { mp - 9 }).expect("month in 1..=12");
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

/// **Private.** Applies `maxLength` (truncate) and `minLength` (pad with `x`) to a
/// plain string. Never called on formatted or well-formed values.
fn fit_length(mut v: String, s: &Value) -> String {
    if let Some(max) = s.get("maxLength").and_then(Value::as_u64) {
        v = v
            .chars()
            .take(usize::try_from(max).unwrap_or(usize::MAX))
            .collect();
    }
    if let Some(min) = s.get("minLength").and_then(Value::as_u64) {
        while (v.chars().count() as u64) < min {
            v.push('x');
        }
    }
    v
}

#[cfg(test)]
mod tests {
    use super::civil_from_days;

    #[test]
    fn stable_hash_matches_core_algorithm() {
        // same vectors as kaanal_core::stable_hash tests
        assert_eq!(super::stable_hash(&[]), 0xcbf2_9ce4_8422_2325);
        assert_ne!(
            super::stable_hash(&[b"ab", b"c"]),
            super::stable_hash(&[b"a", b"bc"])
        );
    }

    #[test]
    fn civil_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(super::BASE_EPOCH / 86_400), (2026, 1, 1));
        assert_eq!(civil_from_days(19_782), (2024, 2, 29));
    }
}
