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
use crate::dictionary::{Dictionary, Gen, IntDist, TextKind, norm, template_key, value_fits};
use crate::pack::{Name, NameOrder, Pack, Place, digit_count, fill};
use crate::words;
mod dist;
mod format;
pub(crate) mod multiple;
mod numbers;
pub(crate) mod pattern;
#[cfg(test)]
mod pattern_tests;
mod reconcile;
#[cfg(test)]
mod tests;
pub(crate) mod text;
pub(crate) mod unique;

use format::{entry_range, format_value, from_dictionary, numeric};
pub(crate) use numbers::count;
use numbers::{array_for, int_for, number_for};
use pattern::with_pattern;
pub(crate) use reconcile::reconcile;
use text::{base64, date_time, email, fit_length, run, uuid};

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
pub(crate) const BASE_EPOCH: i64 = 1_767_225_600;

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
        return pick_enum(choices, field, rng, opts);
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
            reconcile(&mut out, s, seed, opts);
            // `additionalProperties` with a schema: one to three extra seeded keys (a name
            // that is already a property is skipped); `false`/`true`/absent add none.
            if let Some(ap) = s.get("additionalProperties").filter(|a| a.is_object()) {
                let extra = child_seed(seed, "additionalProperties", 0);
                let n = ChaCha8Rng::seed_from_u64(extra).random_range(1..=3usize);
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
        "array" => array_for(s, field, rng, seed, group, depth, budget, opts),
        ty => scalar_for(s, field, ty, rng, group, opts),
    }
}

/// **Private.** The most values [`generate`] can spend on `s`, capped at `MAX_VALUES`: the same
/// walk as `value_for`, with every array at its longest draw, every `oneOf`/`anyOf` at its
/// dearest branch and `additionalProperties` at three keys. A worst case, so it may overcount
/// (`examples` are walked as their schema); paging sizes a page with it (R1.62).
pub(crate) fn value_bound(s: &Value, depth: usize) -> usize {
    if s.get("allOf").is_some() {
        return value_bound(&merge_all_of(s).0, depth);
    }
    if depth > MAX_DEPTH {
        return 0;
    }
    let enum_ = s.get("enum").and_then(Value::as_array);
    if s.get("example").is_some()
        || s.get("const").is_some()
        || enum_.is_some_and(|e| !e.is_empty())
    {
        return 1;
    }
    let child = |c: &Value| value_bound(c, depth + 1);
    let branches = ["oneOf", "anyOf"].iter().find_map(|k| {
        s.get(*k)
            .and_then(Value::as_array)
            .filter(|a| !a.is_empty())
    });
    let inner = if let Some(b) = branches {
        b.iter().map(child).max().unwrap_or(0)
    } else {
        match scalar_type(s) {
            "object" => {
                let props = s.get("properties").and_then(Value::as_object);
                let own = props
                    .into_iter()
                    .flatten()
                    .filter(|(_, ps)| ps.get("writeOnly").and_then(Value::as_bool) != Some(true))
                    .fold(0usize, |n, (_, ps)| n.saturating_add(child(ps)));
                let extra = s
                    .get("additionalProperties")
                    .filter(|a| a.is_object())
                    .map_or(0, |ap| child(ap).saturating_mul(3));
                own.saturating_add(extra)
            }
            "array" => {
                // the longest length `array_for` can draw
                let max_items = count(s, "maxItems", false);
                let min =
                    count(s, "minItems", true).unwrap_or_else(|| max_items.map_or(1, |m| m.min(1)));
                let n = max_items.unwrap_or(5).min(MAX_RANDOM_ITEMS).max(min);
                let items = s.get("items").cloned().unwrap_or(json!({}));
                usize::try_from(n)
                    .unwrap_or(usize::MAX)
                    .saturating_mul(child(&items))
            }
            _ => 0,
        }
    };
    inner.saturating_add(1).min(MAX_VALUES)
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
    // a project `static` or `pick` is the developer's value (pinned): nothing touches it. Any
    // other project generator makes a free value, which still has to meet `pattern`/`multipleOf`.
    let own = from_dictionary(project, false, s, field, ty, rng, group, pack);
    if let Some((v, Gen::Static { .. } | Gen::Pick { .. })) = &own {
        return v.clone();
    }
    let v = match own {
        Some((v, _)) => v,
        None => free_scalar(s, field, ty, rng, group, opts),
    };
    // a free string that breaks its `pattern` is regenerated from it (D2 section 6.1)
    let v = match (ty, s.get("pattern").and_then(Value::as_str)) {
        ("string", Some(p)) => with_pattern(v, p, s, rng),
        _ => v,
    };
    // a free number is moved to the nearest `multipleOf` inside its bounds (D2 section 5.5)
    match ty {
        "integer" | "number" => multiple::snap(v, ty, s),
        _ => v,
    }
}

/// **Private.** The part of [`scalar_for`] after the project dictionary: `format` → built-in
/// dictionary → constraint defaults. Values here are Kaanal's own (free).
fn free_scalar(
    s: &Value,
    field: Option<&str>,
    ty: &str,
    rng: &mut ChaCha8Rng,
    group: u64,
    opts: &Options,
) -> Value {
    let pack = &*opts.pack;
    // a birth date (`format: date` on `dateOfBirth`, `dob`, ...) is an adult's, not "last year"
    let birth_date = ty == "string"
        && s.get("format").and_then(Value::as_str) == Some("date")
        && field
            .and_then(|f| opts.dictionary.lookup(f))
            .and_then(|gens| gens.iter().find(|g| g.fits(s, ty)))
            .is_some_and(|g| matches!(g, Gen::Date { years: Some(_), .. }));
    if ty == "string"
        && !birth_date
        && let Some(v) = format_value(s, rng, group, pack)
    {
        // formats beat lengths (R4.5): never truncated or padded
        return json!(v);
    }
    if let Some((v, _)) =
        from_dictionary(Some(&opts.dictionary), true, s, field, ty, rng, group, pack)
    {
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

/// **Private.** One of a spec field's `enum` values. A dictionary entry for the field's name that
/// carries `weights` (the built-in status family, or the project's) weights the draw; every
/// value stays reachable (unlisted ones weigh 1). A project entry without weights turns that
/// off (D2 section 5.2).
fn pick_enum(
    choices: &[Value],
    field: Option<&str>,
    rng: &mut ChaCha8Rng,
    opts: &Options,
) -> Value {
    let weights = field.and_then(|f| {
        let gens = opts
            .dictionary_project
            .as_deref()
            .and_then(|d| d.lookup(f))
            .or_else(|| opts.dictionary.lookup(f))?;
        gens.iter().find_map(|g| match g {
            Gen::Pick {
                weights: Some(w), ..
            } => Some(&w.0),
            _ => None,
        })
    });
    match weights {
        Some(w) => {
            let refs: Vec<&Value> = choices.iter().collect();
            dist::pick_weighted(&refs, w, rng).clone()
        }
        None => choices.choose(rng).cloned().unwrap_or(Value::Null),
    }
}
