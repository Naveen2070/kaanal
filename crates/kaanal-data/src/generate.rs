//! The data generator: deterministic, realistic JSON from an `OpenAPI` 3.0 schema.
//!
//! Same seed → same output, always. An `allOf` is first merged into one schema; then
//! field-pick priority: `example` → `enum` → `oneOf`/`anyOf` → type, and within a type: `format` → field-name
//! heuristic → constraints → fallback. Every field derives its own seed from
//! `(parent_seed, name, index)`, so schema edits never reshuffle sibling values
//! (rule R4.4).

use rand::seq::IndexedRandom;
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use serde_json::{Map, Value, json};

use crate::words;

/// **Private.** Recursion ceiling; deeper nesting yields `null` instead of overflowing.
pub(crate) const MAX_DEPTH: usize = 16;
/// **Private.** Cap on random array length so `maxItems: 1000000` can't build a huge array.
const MAX_RANDOM_ITEMS: u64 = 20;
/// **Private.** Most values one body may hold. Nested arrays multiply (`20^5` = 3.2M) and
/// `minItems` is honoured, so a per-array cap alone cannot bound a body; past this, values are `null`.
const MAX_VALUES: usize = 10_000;
/// **Private.** Longest string a `minLength` can ask for; a larger value is capped here and
/// warned at startup (R6.10d). ponytail: 10 000 is the default of an open question in RULES.md.
pub(crate) const MAX_MIN_LENGTH: u64 = 10_000;
/// **Private.** Timestamps are "the last 365 days before 2026-01-01", fixed so output never changes between runs.
const BASE_EPOCH: i64 = 1_767_225_600;

/// Deterministic JSON for an `OpenAPI` 3.0 schema whose `$ref`s are already inlined.
/// `allOf` is merged first; then example → enum → oneOf/anyOf → type (format → field-name heuristic → constraints → fallback).
pub fn generate(schema: &Value, field: Option<&str>, seed: u64) -> Value {
    let mut budget = MAX_VALUES;
    value_for(schema, field, seed, seed, 0, &mut budget)
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
/// `budget` counts the values still allowed in this body (see `MAX_VALUES`).
fn value_for(
    s: &Value,
    field: Option<&str>,
    seed: u64,
    group: u64,
    depth: usize,
    budget: &mut usize,
) -> Value {
    if s.get("allOf").is_some() {
        // Merge the schemas, then generate once: parts share one seed, so one person (R6.10a).
        return value_for(&merge_all_of(s).0, field, seed, group, depth, budget);
    }
    if depth > MAX_DEPTH || *budget == 0 {
        return Value::Null;
    }
    *budget -= 1;
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
    if let Some(first) = ["oneOf", "anyOf"]
        .iter()
        .find_map(|k| s.get(*k).and_then(Value::as_array)?.first())
    {
        return value_for(
            first,
            field,
            child_seed(seed, "oneOf", 0),
            group,
            depth + 1,
            budget,
        );
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
                    // OpenAPI: a writeOnly property is sent in requests, never returned (R6.10b).
                    if ps.get("writeOnly").and_then(Value::as_bool) == Some(true) {
                        continue;
                    }
                    out.insert(
                        name.clone(),
                        value_for(
                            ps,
                            Some(name),
                            child_seed(seed, name, 0),
                            seed,
                            depth + 1,
                            budget,
                        ),
                    );
                }
            }
            Value::Object(out)
        }
        "array" => array_for(s, rng, seed, group, depth, budget),
        "integer" => json!(int_for(s, field, rng)),
        "number" => number_for(s, field, rng),
        "boolean" => json!(rng.random_bool(0.5)),
        _ => json!(string_for(s, field, rng, group)),
    }
}

/// **Private.** Merges an `allOf` schema into one schema (R6.10a): `properties` and
/// `required` are unioned (a later property wins), every other keyword is first-wins with
/// the `allOf` siblings counting first, and nested `allOf` is expanded. A conflicting `type`
/// keeps the first and is reported in the second value, for the startup warning.
pub(crate) fn merge_all_of(s: &Value) -> (Value, Vec<String>) {
    let mut out = Map::new();
    let mut conflicts = Vec::new();
    merge_into(&mut out, s, &mut conflicts);
    (Value::Object(out), conflicts)
}

/// **Private.** One step of [`merge_all_of`]: `s`'s own keywords, then its `allOf` parts.
fn merge_into(out: &mut Map<String, Value>, s: &Value, conflicts: &mut Vec<String>) {
    let Value::Object(m) = s else { return };
    for (k, v) in m {
        match (k.as_str(), v) {
            ("allOf", _) => {}
            ("properties", Value::Object(p)) => {
                if let Value::Object(o) = out.entry(k).or_insert_with(|| json!({})) {
                    o.extend(p.clone());
                }
            }
            ("required", Value::Array(r)) => {
                if let Value::Array(o) = out.entry(k).or_insert_with(|| json!([])) {
                    o.extend(
                        r.iter()
                            .filter(|x| !o.contains(x))
                            .cloned()
                            .collect::<Vec<_>>(),
                    );
                }
            }
            ("type", _) if out.get(k).is_some_and(|t| t != v) => {
                conflicts.push(format!("type {} vs {v}; first wins", out[k]));
            }
            _ => {
                out.entry(k).or_insert_with(|| v.clone());
            }
        }
    }
    for part in m
        .get("allOf")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        merge_into(out, part, conflicts);
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
    let min = int_bound(s, "minimum", true);
    if let Some(m) = min {
        lo = m;
        if hi < lo {
            hi = lo.saturating_add(1000);
        }
    }
    if let Some(m) = int_bound(s, "maximum", false) {
        hi = m;
        if lo > hi {
            // A lone maximum below the default range moves the range (as `number_for` does);
            // an explicit minimum above the maximum is contradictory, so it collapses to it.
            lo = if min.is_some() {
                hi
            } else {
                hi.saturating_sub(1000)
            };
        }
    }
    rng.random_range(lo..=hi)
}

/// **Private.** An integer `minimum` (`lower`) or `maximum`, exclusive flag applied. A `u64`
/// past `i64::MAX` saturates; a float rounds inward, so `0.0` counts as `0` and `1.5` as `2`
/// for a lower bound (rule R6.10c).
#[expect(
    clippy::cast_possible_truncation,
    reason = "f64 -> i64 `as` saturates, which is the wanted clamp"
)]
fn int_bound(s: &Value, key: &str, lower: bool) -> Option<i64> {
    let v = s.get(key)?;
    let excl = exclusive(
        s,
        if lower {
            "exclusiveMinimum"
        } else {
            "exclusiveMaximum"
        },
    );
    if let Some(m) = v.as_i64().or_else(|| v.as_u64().map(|_| i64::MAX)) {
        return Some(match (excl, lower) {
            (false, _) => m,
            (true, true) => m.saturating_add(1),
            (true, false) => m.saturating_sub(1),
        });
    }
    let m = v.as_f64()?;
    Some(match (excl, lower) {
        (false, true) => m.ceil(),
        (false, false) => m.floor(),
        (true, true) => m.floor() + 1.0,
        (true, false) => m.ceil() - 1.0,
    } as i64)
}

/// **Private.** A length or item count (`minLength`, `maxItems`, …). Whole-number floats
/// count; a fractional one rounds inward (`lower` up, else down); negatives become 0.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    reason = "f64 -> u64 `as` saturates (negatives to 0), which is the wanted clamp"
)]
pub(crate) fn count(s: &Value, key: &str, lower: bool) -> Option<u64> {
    let v = s.get(key)?;
    v.as_u64().or_else(|| {
        v.as_f64()
            .map(|m| (if lower { m.ceil() } else { m.floor() }) as u64)
    })
}

/// **Private.** `OpenAPI` 3.0: `exclusiveMinimum: true` turns `minimum` into a strict bound.
fn exclusive(s: &Value, key: &str) -> bool {
    s.get(key).and_then(Value::as_bool).unwrap_or(false)
}

/// **Private.** Float value: default ranges by field name (lat/lng 6 places, money and
/// the rest 2 places), then `minimum`/`maximum` with one-unit slack for exclusive limits.
fn number_for(s: &Value, field: Option<&str>, rng: &mut ChaCha8Rng) -> Value {
    let f = norm(field);
    let money = ["price", "total", "amount", "cost", "subtotal"]
        .iter()
        .any(|k| f.ends_with(k));
    let ((dlo, dhi), places) = match f.as_str() {
        "lat" | "latitude" => ((8.0, 35.0), 6),
        "lng" | "lon" | "longitude" => ((68.0, 97.0), 6),
        _ if money => ((99.0, 4999.0), 2),
        _ => ((0.0, 1000.0), 2),
    };
    // values are rounded to `places`, so a strict bound is one unit in the last place inside
    let unit = 1.0 / 10f64.powi(places);
    let (min, max) = (
        s.get("minimum").and_then(Value::as_f64),
        s.get("maximum").and_then(Value::as_f64),
    );
    let mut lo = match min {
        Some(m) if exclusive(s, "exclusiveMinimum") => m + unit,
        Some(m) => m,
        None => dlo,
    };
    let hi = match max {
        Some(m) if exclusive(s, "exclusiveMaximum") => m - unit,
        Some(m) => m,
        None if dhi > lo => dhi,
        None => lo + 1000.0,
    };
    if min.is_none() && hi < lo {
        // a lone `maximum` below the default range moves the range, as a lone `minimum` does
        lo = hi - 1000.0;
    }
    if let (Some(a), Some(b)) = (min, max)
        && lo > hi
        && a < b
    {
        // exclusive range narrower than one unit: no rounded value fits, so the exact midpoint
        return json!(f64::midpoint(a, b));
    }
    let hi = hi.max(lo);
    // An infinite span (`-1e308..1e308`) would panic inside `random_range`.
    let v = if hi > lo && (hi - lo).is_finite() {
        rng.random_range(lo..=hi)
    } else {
        lo
    };
    json!(round(v, places).clamp(lo, hi))
}

/// **Private.** Array value: random length in `minItems..=maxItems` (random part capped at
/// `MAX_RANDOM_ITEMS`), then cut to the remaining value budget.
fn array_for(
    s: &Value,
    rng: &mut ChaCha8Rng,
    seed: u64,
    group: u64,
    depth: usize,
    budget: &mut usize,
) -> Value {
    let max_items = count(s, "maxItems", false);
    // default at least one item, but never more than `maxItems` (`maxItems: 0` → `[]`)
    let min = count(s, "minItems", true).unwrap_or_else(|| max_items.map_or(1, |m| m.min(1)));
    // limitation: cap random length so `maxItems: 1000000` can't build a huge array; minItems is still honoured
    let max = max_items.unwrap_or(5).min(MAX_RANDOM_ITEMS).max(min);
    // The draw is unchanged by the budget, so seeds stay stable; only the length is cut.
    let n = rng
        .random_range(min..=max)
        .min(u64::try_from(*budget).unwrap_or(u64::MAX));
    let n = usize::try_from(n).expect("length is capped by the value budget");
    let items = s.get("items").cloned().unwrap_or(json!({}));
    Value::Array(
        (0..n)
            .map(|i| {
                value_for(
                    &items,
                    None,
                    child_seed(seed, "[]", i),
                    group,
                    depth + 1,
                    budget,
                )
            })
            .collect(),
    )
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
    let format = s.get("format").and_then(Value::as_str).unwrap_or("");
    let v = match format {
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
    // Only formats built above must stay well-formed; unknown ones (`password`) still get lengths.
    let structured = matches!(
        format,
        "email" | "uuid" | "date-time" | "date" | "uri" | "url" | "ipv4"
    ) || keeps_format(&norm(field), is_id(field));
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
            let n = rng.random_range(1..=9999);
            // a bare `url` field has no stem: no empty path segment (`example.com//1234`)
            match f.strip_suffix("url").unwrap_or(f) {
                "" => format!("https://example.com/{n}"),
                stem => format!("https://example.com/{stem}/{n}"),
            }
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

/// **Private.** Applies `maxLength` (truncate) and `minLength` (pad with `x` in one pass,
/// capped at [`MAX_MIN_LENGTH`]) to a plain string. Never called on formatted or well-formed values.
fn fit_length(mut v: String, s: &Value) -> String {
    if let Some(max) = count(s, "maxLength", false) {
        v = v
            .chars()
            .take(usize::try_from(max).unwrap_or(usize::MAX))
            .collect();
    }
    if let Some(min) = count(s, "minLength", true) {
        let min = usize::try_from(min.min(MAX_MIN_LENGTH)).expect("capped at MAX_MIN_LENGTH");
        let pad = min.saturating_sub(v.chars().count());
        v.extend(std::iter::repeat_n('x', pad));
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
        assert_eq!(super::stable_hash(&[b"a", b"bc"]), 0xa0a3_542c_19b9_00ab);
        assert_ne!(
            super::stable_hash(&[b"ab", b"c"]),
            super::stable_hash(&[b"a", b"bc"])
        );
    }

    #[test]
    fn merge_all_of_unions_properties_and_keeps_the_first_type() {
        use serde_json::json;
        let s = json!({
            "description": "sibling",
            "required": ["a"],
            "properties": { "a": { "type": "integer" } },
            "allOf": [
                { "type": "object", "description": "part", "required": ["a", "b"],
                  "properties": { "b": { "type": "string" } } },
                { "allOf": [{ "type": "array", "properties": { "a": { "type": "string" } } }] }
            ]
        });
        let (m, conflicts) = super::merge_all_of(&s);
        assert_eq!(
            m,
            json!({
                "description": "sibling",
                "type": "object",
                "required": ["a", "b"],
                "properties": { "a": { "type": "string" }, "b": { "type": "string" } }
            })
        );
        assert_eq!(conflicts.len(), 1, "{conflicts:?}");
    }

    #[test]
    fn civil_dates() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
        assert_eq!(civil_from_days(super::BASE_EPOCH / 86_400), (2026, 1, 1));
        assert_eq!(civil_from_days(19_782), (2024, 2, 29));
    }
}
