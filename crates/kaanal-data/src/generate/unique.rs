//! `uniqueItems` for arrays of scalars (D2 spec section 5.5).
//!
//! A duplicate is redrawn from a fresh seed of its own slot (`child_seed(seed, "unique", i*64+k)`),
//! at most [`TRIES`] times, then dropped, so the array is always unique and may be shorter than
//! its random length (shorter than `minItems` only when the items have too few distinct values,
//! which [`problem`] warns about). Arrays of objects or arrays are not handled.

use std::collections::HashSet;

use serde_json::Value;

use super::numbers::{count, int_bound};
use super::{Options, child_seed, scalar_type, value_for};

/// **Private.** Redraws per slot before a duplicate is dropped.
const TRIES: usize = 64;

/// Whether `uniqueItems: true` on an array whose `items` schema is `items` is handled.
pub(super) fn handles(s: &Value) -> bool {
    s.get("uniqueItems").and_then(Value::as_bool) == Some(true)
        && s.get("items").is_none_or(is_scalar)
}

/// **Private.** What makes two items the same: JSON Schema compares numbers by value, so
/// `1` and `1.0` are one.
fn identity(v: &Value) -> String {
    match v {
        Value::Number(n) => n.as_f64().map_or_else(|| v.to_string(), |f| f.to_string()),
        other => other.to_string(),
    }
}

/// **Private.** A scalar `items` schema (string, number, integer or boolean).
fn is_scalar(items: &Value) -> bool {
    matches!(
        scalar_type(items),
        "string" | "integer" | "number" | "boolean"
    )
}

/// **Private.** How many distinct values `items` can take, when that is small and known: an
/// `enum`, a boolean, or an integer range with both bounds.
fn capacity(items: &Value) -> Option<u64> {
    if let Some(e) = items
        .get("enum")
        .and_then(Value::as_array)
        .filter(|e| !e.is_empty())
    {
        let distinct: HashSet<String> = e.iter().map(identity).collect();
        return u64::try_from(distinct.len()).ok();
    }
    match scalar_type(items) {
        "boolean" => Some(2),
        "integer" => {
            let (lo, hi) = (
                int_bound(items, "minimum", true)?,
                int_bound(items, "maximum", false)?,
            );
            u64::try_from(i128::from(hi) - i128::from(lo) + 1).ok()
        }
        _ => None,
    }
}

/// Removes duplicates from `out`, the elements of the array described by `s` (see [`handles`]),
/// redrawing each from its own slot seed.
pub(super) fn dedupe(
    out: Vec<Value>,
    s: &Value,
    field: Option<&str>,
    (seed, group, depth): (u64, u64, usize),
    opts: &Options,
) -> Vec<Value> {
    // no `items` generates plain strings, as `array_for` does
    let fallback = serde_json::json!({});
    let items = s.get("items").unwrap_or(&fallback);
    let mut seen: HashSet<String> = HashSet::new();
    let mut kept = Vec::with_capacity(out.len());
    for (i, mut v) in out.into_iter().enumerate() {
        let mut k = 0;
        while !seen.insert(identity(&v)) {
            k += 1;
            if k > TRIES {
                break;
            }
            let fresh = child_seed(seed, "unique", i * TRIES + k);
            v = value_for(items, field, fresh, group, depth + 1, &mut 16, opts);
        }
        if k <= TRIES {
            kept.push(v);
        }
    }
    kept
}

/// Why `uniqueItems` on an array with schema `s` is not (fully) honoured, or `None`.
pub(crate) fn problem(s: &Value) -> Option<String> {
    if s.get("uniqueItems").and_then(Value::as_bool) != Some(true) {
        return None;
    }
    let fallback = serde_json::json!({});
    let items = s.get("items").unwrap_or(&fallback);
    if !is_scalar(items) {
        return Some(
            "uniqueItems is only supported for arrays of strings, numbers, integers and booleans; duplicates may remain"
                .to_string(),
        );
    }
    let (cap, min) = (capacity(items)?, count(s, "minItems", true)?);
    (cap < min).then(|| {
        format!(
            "uniqueItems: the items have only {cap} distinct values but minItems is {min}; arrays are shorter than minItems"
        )
    })
}
