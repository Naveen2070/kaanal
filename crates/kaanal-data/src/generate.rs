//! The data generator: deterministic, realistic JSON from an `OpenAPI` 3.0 schema.
//!
//! Same seed → same output, always. An `allOf` is first merged into one schema; then
//! field-pick priority (D1.8): `example` → `const`/`enum` → `oneOf`/`anyOf` (a branch picked
//! by the field's own seed) → type, and within a
//! scalar type: project dictionary → `format` → built-in dictionary → constraint defaults.
//! A dictionary entry that cannot fit the type or bounds is skipped. Every field derives its own seed from
//! `(parent_seed, name, index)`, so schema edits never reshuffle sibling values
//! (rule R4.4).

use rand::seq::IndexedRandom;
use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use serde_json::{Map, Value, json};

use crate::dataset::Options;
use crate::dictionary::{Dictionary, Gen, TextKind, norm, template_key, value_fits};
use crate::pack::{Name, NameOrder, Pack, Place, digit_count, fill};
use crate::words;

/// **Private.** Recursion ceiling; deeper nesting yields `null` instead of overflowing.
pub(crate) const MAX_DEPTH: usize = 16;
/// **Private.** Cap on random array length so `maxItems: 1000000` can't build a huge array.
const MAX_RANDOM_ITEMS: u64 = 20;
/// **Private.** Most values one body may hold. Nested arrays multiply (`20^5` = 3.2M) and
/// `minItems` is honoured, so a per-array cap alone cannot bound a body; past this, values are `null`.
pub(crate) const MAX_VALUES: usize = 10_000;
/// **Private.** Longest string a `minLength` can ask for; a larger value is capped here and
/// warned at startup (R6.10d). ponytail: 10 000 is the default of an open question in RULES.md.
pub(crate) const MAX_MIN_LENGTH: u64 = 10_000;
/// **Private.** Timestamps are "the last 365 days before 2026-01-01", fixed so output never changes between runs.
const BASE_EPOCH: i64 = 1_767_225_600;

/// Deterministic JSON for an `OpenAPI` 3.0 schema whose `$ref`s are already inlined.
/// `allOf` is merged first; then example → const/enum → oneOf/anyOf → type (project dictionary →
/// format → built-in dictionary → constraint defaults). Names, places, phones, currency and
/// country come from `opts.pack`.
pub fn generate(schema: &Value, field: Option<&str>, seed: u64, opts: &Options) -> Value {
    let mut budget = MAX_VALUES;
    value_for(schema, field, seed, seed, 0, &mut budget, opts)
}

/// **Private.** Property `name` (schema `ps`) of an object generated with `seed`: the value
/// [`generate`] gives that property, so an empty form's scalars match the full body (R4.4).
pub(crate) fn property(
    ps: &Value,
    name: &str,
    seed: u64,
    depth: usize,
    budget: &mut usize,
    opts: &Options,
) -> Value {
    value_for(
        ps,
        Some(name),
        child_seed(seed, name, 0),
        seed,
        depth,
        budget,
        opts,
    )
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
pub(crate) fn child_seed(parent: u64, name: &str, index: usize) -> u64 {
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
    opts: &Options,
) -> Value {
    if s.get("allOf").is_some() {
        // Merge the schemas, then generate once: parts share one seed, so one person (R6.10a).
        return value_for(&merge_all_of(s).0, field, seed, group, depth, budget, opts);
    }
    if depth > MAX_DEPTH || *budget == 0 {
        return Value::Null;
    }
    *budget -= 1;
    let rng = &mut ChaCha8Rng::seed_from_u64(seed);
    if let Some(ex) = s.get("example") {
        return ex.clone();
    }
    if let Some(e) = pick_example(s, rng) {
        return e;
    }
    if let Some(c) = s.get("const") {
        return c.clone();
    }
    if let Some(choices) = s
        .get("enum")
        .and_then(Value::as_array)
        .filter(|e| !e.is_empty())
    {
        return choices.choose(rng).cloned().unwrap_or(Value::Null);
    }
    if let Some(branches) = ["oneOf", "anyOf"].iter().find_map(|k| {
        s.get(*k)
            .and_then(Value::as_array)
            .filter(|a| !a.is_empty())
    }) {
        // R4.4: the pick comes from this value's own seed, never a parent's or sibling's
        // rng, so adding a property elsewhere never changes the branch. A discriminator
        // tag is the branch's single-value `enum`/`const` (a `mapping` became one in `api`).
        let pick = ChaCha8Rng::seed_from_u64(child_seed(seed, "branch", 0))
            .random_range(0..branches.len());
        return value_for(
            &branches[pick],
            field,
            child_seed(seed, "oneOf", 0),
            group,
            depth + 1,
            budget,
            opts,
        );
    }
    match scalar_type(s) {
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
                            opts,
                        ),
                    );
                }
            }
            // `additionalProperties` with a schema: up to three extra seeded keys (a name
            // that is already a property is skipped); `false`/`true`/absent add none.
            if let Some(ap) = s.get("additionalProperties").filter(|a| a.is_object()) {
                let extra = child_seed(seed, "additionalProperties", 0);
                let n = ChaCha8Rng::seed_from_u64(extra).random_range(0..=3usize);
                for i in 0..n {
                    let k = child_seed(extra, "key", i);
                    let name = format!("key{}", k % 10_000);
                    if out.contains_key(&name) {
                        continue;
                    }
                    let v = value_for(ap, None, k, seed, depth + 1, budget, opts);
                    out.insert(name, v);
                }
            }
            Value::Object(out)
        }
        "array" => array_for(s, rng, seed, group, depth, budget, opts),
        ty => scalar_for(s, field, ty, rng, group, opts),
    }
}

/// **Private.** One entry of `examples` (an array, or an `OpenAPI` map of `{ value }`),
/// chosen from the field's own rng; `None` when absent or empty.
fn pick_example(s: &Value, rng: &mut ChaCha8Rng) -> Option<Value> {
    match s.get("examples")? {
        Value::Array(a) => a.choose(rng).cloned(),
        Value::Object(m) => {
            let vals: Vec<&Value> = m.values().filter_map(|e| e.get("value")).collect();
            vals.choose(rng).map(|v| (*v).clone())
        }
        _ => None,
    }
}

/// **Private.** The type the generator treats `s` as: `object`, `array`, `integer`,
/// `number`, `boolean`, else `string` (no `type` with `properties` is an object). A type
/// array (`["string", "null"]`) uses its first non-`null` entry; `nullable` is ignored,
/// so a real value is always emitted.
pub(crate) fn scalar_type(s: &Value) -> &'static str {
    let ty = match s.get("type") {
        Some(Value::Array(a)) => a.iter().find_map(|t| t.as_str().filter(|t| *t != "null")),
        t => t.and_then(Value::as_str),
    };
    match ty {
        Some("object") => "object",
        Some("array") => "array",
        Some("integer") => "integer",
        Some("number") => "number",
        Some("boolean") => "boolean",
        None if s.get("properties").is_some()
            || s.get("additionalProperties").is_some_and(Value::is_object) =>
        {
            "object"
        }
        _ => "string",
    }
}

/// **Private.** A scalar value, in D1.8 order: project dictionary → `format` → built-in
/// dictionary → constraint defaults.
fn scalar_for(
    s: &Value,
    field: Option<&str>,
    ty: &str,
    rng: &mut ChaCha8Rng,
    group: u64,
    opts: &Options,
) -> Value {
    let pack = &*opts.pack;
    let project = opts.dictionary_project.as_deref();
    if let Some(v) = from_dictionary(project, false, s, field, ty, rng, group, pack) {
        return v;
    }
    if ty == "string"
        && let Some(v) = format_value(s, rng, group, pack)
    {
        // formats beat lengths (R4.5): never truncated or padded
        return json!(v);
    }
    if let Some(v) = from_dictionary(Some(&opts.dictionary), true, s, field, ty, rng, group, pack) {
        return v;
    }
    match ty {
        "integer" => json!(int_for(s, rng, (0, 1000))),
        "number" => number_for(s, rng, (0.0, 1000.0), 2),
        "boolean" => json!(rng.random_bool(0.5)),
        _ => json!(fit_length(words::pick(words::WORDS, rng).to_string(), s)),
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

/// **Private.** Integer value: a default range `(lo, hi)` (from a dictionary entry, else
/// `0..=1000`), bounded by `minimum`/`maximum` and their exclusive variants. Needs `lo <= hi`.
fn int_for(s: &Value, rng: &mut ChaCha8Rng, (mut lo, mut hi): (i64, i64)) -> i64 {
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
    if s.get("format").and_then(Value::as_str) == Some("int32") {
        // both ends move, so `lo <= hi` still holds
        lo = lo.clamp(i64::from(i32::MIN), i64::from(i32::MAX));
        hi = hi.clamp(i64::from(i32::MIN), i64::from(i32::MAX));
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

/// **Private.** Float value: a default range `(dlo, dhi)` rounded to `places` (from a
/// dictionary entry, else `0..=1000` at 2 places), then `minimum`/`maximum` with one-unit
/// slack for exclusive limits.
fn number_for(s: &Value, rng: &mut ChaCha8Rng, (dlo, dhi): (f64, f64), places: i32) -> Value {
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
    opts: &Options,
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
                    opts,
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

/// **Private.** A value for a known string `format`; `None` for an unknown one,
/// which then falls through to the dictionary and lengths.
fn format_value(s: &Value, rng: &mut ChaCha8Rng, group: u64, pack: &Pack) -> Option<String> {
    Some(match s.get("format").and_then(Value::as_str)? {
        "email" => email(rng, group, pack),
        "uuid" => uuid(rng),
        "date-time" => date_time(rng),
        "date" => date_time(rng)[..10].to_string(),
        "uri" | "url" => format!("https://example.com/{}", words::pick(words::WORDS, rng)),
        // RFC 5737 documentation ranges, never a real host
        "ipv4" => format!(
            "{}.{}",
            ["198.51.100", "203.0.113"][rng.random_range(0..2)],
            rng.random_range(1..=254)
        ),
        // RFC 3849 documentation prefix
        "ipv6" => format!(
            "2001:db8::{:x}:{:x}",
            rng.random_range(0..=0xffff),
            rng.random_range(1..=0xffff)
        ),
        "hostname" => format!("{}.example.com", words::pick(words::WORDS, rng)),
        "byte" => base64(&rng.random::<[u8; 12]>()),
        // formats beat lengths, but a password is plain text: fit it
        "password" => {
            const ALNUM: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
            let p = (0..16)
                .map(|_| char::from(ALNUM[rng.random_range(0..ALNUM.len())]))
                .collect();
            fit_length(p, s)
        }
        "duration" => format!(
            "P{}DT{}H{}M",
            rng.random_range(0..=30),
            rng.random_range(0..24),
            rng.random_range(0..60)
        ),
        _ => return None,
    })
}

/// **Private.** The first generator of `field`'s entry in `d` that fits `ty` and the
/// schema's bounds, run; `None` when there is no entry or none fits (design 11a rule 2).
/// A `builtin` entry must also meet the string lengths (its values are never cut or
/// padded); a project one is sent as written and warned at startup instead.
#[expect(
    clippy::too_many_arguments,
    reason = "private helper, one call site per layer"
)]
fn from_dictionary(
    d: Option<&Dictionary>,
    builtin: bool,
    s: &Value,
    field: Option<&str>,
    ty: &str,
    rng: &mut ChaCha8Rng,
    group: u64,
    pack: &Pack,
) -> Option<Value> {
    let f = field?;
    let g = d?
        .lookup(f)?
        .iter()
        .find(|g| g.fits(s, ty) && (!builtin || g.length_fits(s, ty)))?;
    Some(run(g, s, f, ty, rng, group, pack))
}

/// **Private.** `(min, max)` from optional entry bounds and a default range: a lone bound
/// keeps the default's other end when it can, else extends by `step` (keeps `lo <= hi`).
fn entry_range<T: PartialOrd + Copy>(
    min: Option<T>,
    max: Option<T>,
    (dlo, dhi): (T, T),
    up: impl Fn(T) -> T,
    down: impl Fn(T) -> T,
) -> (T, T) {
    match (min, max) {
        (Some(a), Some(b)) => (a, b),
        (Some(a), None) => (a, if a <= dhi { dhi } else { up(a) }),
        (None, Some(b)) => (if dlo <= b { dlo } else { down(b) }, b),
        (None, None) => (dlo, dhi),
    }
}

/// **Private.** A float range into an `integer` field (rounded, so `lo <= hi` holds) or a
/// `number` field (`places` decimals); either way clamped to the schema's bounds.
#[expect(
    clippy::cast_possible_truncation,
    reason = "f64 -> i64 `as` saturates, which is the wanted clamp"
)]
fn numeric(s: &Value, ty: &str, rng: &mut ChaCha8Rng, (lo, hi): (f64, f64), places: i32) -> Value {
    if ty == "integer" {
        json!(int_for(s, rng, (lo.round() as i64, hi.round() as i64)))
    } else {
        number_for(s, rng, (lo, hi), places)
    }
}

/// **Private.** Runs one dictionary generator for field `f` of scalar type `ty` (it fits).
/// Numbers clamp to the schema's bounds; text is fitted to its lengths in characters,
/// except well-formed values (email, phone, url, uuid, timestamps; R4.5).
fn run(
    g: &Gen,
    s: &Value,
    f: &str,
    ty: &str,
    rng: &mut ChaCha8Rng,
    group: u64,
    pack: &Pack,
) -> Value {
    let (up, down) = (|x: f64| x + 1000.0, |x: f64| x - 1000.0);
    let v = match g {
        Gen::Integer { min, max } => {
            let (lo, hi) = entry_range(
                *min,
                *max,
                (0, 1000),
                |x: i64| x.saturating_add(1000),
                |x: i64| x.saturating_sub(1000),
            );
            let n = json!(int_for(s, rng, (lo, hi)));
            // a `number` field may have bounds no whole number fits (0.1..0.9): then a float
            if ty == "number" && !value_fits(&n, s, ty) {
                #[expect(clippy::cast_precision_loss, reason = "a default range only")]
                return number_for(s, rng, (lo as f64, hi as f64), 2);
            }
            return n;
        }
        Gen::Money { min, max } => {
            return numeric(
                s,
                ty,
                rng,
                entry_range(*min, *max, (99.0, 4999.0), up, down),
                2,
            );
        }
        Gen::Latitude { min, max } | Gen::Longitude { min, max } => {
            let range = geo_range(pack, matches!(g, Gen::Latitude { .. }));
            return numeric(s, ty, rng, entry_range(*min, *max, range, up, down), 6);
        }
        // user-written values are sent as written, never cut or padded (R5.7); a project
        // value outside the lengths is warned at startup, a built-in one never chosen
        Gen::Pick { from } => {
            let fit: Vec<&Value> = from.iter().filter(|v| value_fits(v, s, ty)).collect();
            return (*fit.choose(rng).expect("`fits` found one")).clone();
        }
        Gen::Static { value } => return value.clone(),
        // well-formed: no length fitting (R4.5)
        Gen::Email {} => return json!(email(rng, group, pack)),
        Gen::Phone {} => return json!(phone(rng, pack)),
        Gen::Url {} => return json!(url(f, rng)),
        Gen::Uuid {} => return json!(uuid(rng)),
        Gen::Timestamp {} => return json!(date_time(rng)),
        Gen::Date {} => return json!(date_time(rng)[..10].to_string()),
        Gen::FirstName {} => person(group, pack).0.name.clone(),
        Gen::LastName {} => person(group, pack).1.name.clone(),
        Gen::FullName {} => {
            let (given, family) = person(group, pack);
            match pack.name_order {
                NameOrder::GivenFamily => format!("{} {}", given.name, family.name),
                NameOrder::FamilyGiven => format!("{} {}", family.name, given.name),
            }
        }
        Gen::Username {} => username(rng, group, pack),
        Gen::City {} => place(group, pack).city.clone(),
        // a stateless locale answers with the city (design 11a rule 4)
        Gen::State {} => {
            let p = place(group, pack);
            p.state.as_ref().unwrap_or(&p.city).clone()
        }
        Gen::Postcode {} => postcode(group, pack),
        Gen::Country {} => pack.country.clone(),
        Gen::Street {} => street(group, pack).to_string(),
        Gen::Address {} => address(rng, group, pack),
        Gen::Currency {} => pack.currency.code.clone(),
        Gen::Text { kind } => match kind {
            TextKind::Sentence => sentence(rng),
            TextKind::Paragraph => {
                let n = rng.random_range(3..=5);
                (0..n).map(|_| sentence(rng)).collect::<Vec<_>>().join(" ")
            }
        },
        Gen::Template { value } => {
            return json!(
                fill(value, |k| {
                    let (letters, n) = template_key(k)?;
                    Some(if letters {
                        (0..n)
                            .map(|_| char::from(b'A' + rng.random_range(0..26u8)))
                            .collect()
                    } else {
                        digits(rng, n)
                    })
                })
                .expect("templates are checked when the dictionary loads")
            );
        }
    };
    json!(fit_length(v, s))
}

/// **Private.** The default `(min, max)` of a `latitude` (`lat`) or `longitude` field: the
/// pack's `geo` block, else the whole globe.
fn geo_range(pack: &Pack, lat: bool) -> (f64, f64) {
    let [lo, hi] = match (pack.geo, lat) {
        (Some(g), true) => g.lat,
        (Some(g), false) => g.lon,
        (None, true) => [-90.0, 90.0],
        (None, false) => [-180.0, 180.0],
    };
    (lo, hi)
}

/// **Private.** `https://example.com/<stem>/<n>`, the stem being the field name without a
/// trailing `url` (`avatarUrl` → `avatar`); a bare `url` has no empty segment.
fn url(f: &str, rng: &mut ChaCha8Rng) -> String {
    let n = rng.random_range(1..=9999);
    let f = norm(f);
    match f.strip_suffix("url").unwrap_or(&f) {
        "" => format!("https://example.com/{n}"),
        stem => format!("https://example.com/{stem}/{n}"),
    }
}

/// **Private.** A phone number in one of the pack's formats.
fn phone(rng: &mut ChaCha8Rng, pack: &Pack) -> String {
    let format = pack
        .phone
        .formats
        .choose(rng)
        .expect("formats is non-empty");
    format!("{} {}", pack.phone.prefix, digits_template(format, rng))
}

/// **Private.** The object's address from the pack's template.
fn address(rng: &mut ChaCha8Rng, group: u64, pack: &Pack) -> String {
    let p = place(group, pack);
    fill(&pack.address, |k| {
        Some(match k {
            "number" => rng.random_range(1..=999).to_string(),
            "street" => street(group, pack).to_string(),
            "city" => p.city.clone(),
            "state" => p.state.clone().unwrap_or_default(),
            "postcode" => postcode(group, pack),
            _ => return None,
        })
    })
    .expect("the address template is checked when the pack loads")
}

/// **Private.** Index into a list of `len` items from the object's group seed and a tag.
fn group_index(group: u64, tag: &[u8], len: usize) -> usize {
    let i = stable_hash(&[&group.to_le_bytes(), tag]) % len as u64;
    usize::try_from(i).expect("i < len")
}

/// **Private.** The object's street: `street` and the address's `{street}` agree.
fn street(group: u64, pack: &Pack) -> &str {
    &pack.streets[group_index(group, b"street", pack.streets.len())]
}

/// **Private.** The place row of the object a field belongs to: same object, same place.
fn place(group: u64, pack: &Pack) -> &Place {
    &pack.places[group_index(group, b"place", pack.places.len())]
}

/// **Private.** The object's postcode: the place's pattern with `{n}` filled from the
/// group seed, so `postcode` and `address` agree.
fn postcode(group: u64, pack: &Pack) -> String {
    let rng = &mut ChaCha8Rng::seed_from_u64(stable_hash(&[&group.to_le_bytes(), b"postcode"]));
    digits_template(&place(group, pack).postcode, rng)
}

/// **Private.** A `{n}`-digit pattern (phone format, postcode) filled with seeded digits.
fn digits_template(pattern: &str, rng: &mut ChaCha8Rng) -> String {
    fill(pattern, |k| digit_count(k).map(|n| digits(rng, n)))
        .expect("patterns are checked when the pack loads")
}

/// **Private.** The (given, family) name of the person an object describes: same
/// object, same person, so `firstName`, `lastName`, `name` and `email` agree.
fn person(group: u64, pack: &Pack) -> (&Name, &Name) {
    (
        &pack.first_names[group_index(group, b"first", pack.first_names.len())],
        &pack.last_names[group_index(group, b"last", pack.last_names.len())],
    )
}

/// **Private.** `first.lastNN@example.com` for the object's person — always a reserved domain (rule R5.7).
/// Local part is [`Pack::ascii_local`]: `latin:` form, else folded, else `user<n>`.
fn email(rng: &mut ChaCha8Rng, group: u64, pack: &Pack) -> String {
    let (first, last) = person(group, pack);
    let n = rng.random_range(1..=99);
    let (first, last) = (pack.ascii_local(first, n), pack.ascii_local(last, n));
    let fallback = format!("user{n}");
    if first == fallback && last == fallback {
        // both names fell back: one `user<n>`, not `user7.user77`
        return format!("{fallback}@example.com");
    }
    format!("{first}.{last}{n}@example.com")
}

/// **Private.** `first_lastNN` for the object's person, ASCII like the email's local part.
fn username(rng: &mut ChaCha8Rng, group: u64, pack: &Pack) -> String {
    let (first, last) = person(group, pack);
    let n = rng.random_range(1..=99);
    let (first, last) = (pack.ascii_local(first, n), pack.ascii_local(last, n));
    if first == format!("user{n}") && last == first {
        return first;
    }
    format!("{first}_{last}{n}")
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

/// **Private.** Standard base64 with padding.
fn base64(b: &[u8]) -> String {
    const A: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for c in b.chunks(3) {
        let n = c.iter().fold(0u32, |n, x| n << 8 | u32::from(*x)) << (8 * (3 - c.len()));
        for i in 0..=c.len() {
            out.push(char::from(A[(n >> (18 - 6 * i) & 63) as usize]));
        }
        out.extend(std::iter::repeat_n('=', 3 - c.len()));
    }
    out
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
