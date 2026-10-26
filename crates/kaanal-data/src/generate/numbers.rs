//! Integers, numbers and arrays: bounds, counts and rounding (`multipleOf` is `multiple.rs`,
//! `uniqueItems` is `unique.rs`).

use super::{
    ChaCha8Rng, MAX_RANDOM_ITEMS, Options, Rng, Value, child_seed, json, unique, value_for,
};

/// **Private.** Integer value: a default range `(lo, hi)` (from a dictionary entry, else
/// `0..=1000`), bounded by `minimum`/`maximum` and their exclusive variants. Needs `lo <= hi`.
pub(super) fn int_for(s: &Value, rng: &mut ChaCha8Rng, (mut lo, mut hi): (i64, i64)) -> i64 {
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
pub(super) fn int_bound(s: &Value, key: &str, lower: bool) -> Option<i64> {
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
pub(super) fn exclusive(s: &Value, key: &str) -> bool {
    s.get(key).and_then(Value::as_bool).unwrap_or(false)
}

/// **Private.** Float value: a default range `(dlo, dhi)` rounded to `places` (from a
/// dictionary entry, else `0..=1000` at 2 places), then `minimum`/`maximum` with one-unit
/// slack for exclusive limits.
pub(super) fn number_for(
    s: &Value,
    rng: &mut ChaCha8Rng,
    (dlo, dhi): (f64, f64),
    places: i32,
) -> Value {
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
#[expect(
    clippy::too_many_arguments,
    reason = "private helper, one call site per layer"
)]
pub(super) fn array_for(
    s: &Value,
    field: Option<&str>,
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
    let out: Vec<Value> = (0..n)
        .map(|i| {
            value_for(
                &items,
                field,
                child_seed(seed, "[]", i),
                group,
                depth + 1,
                budget,
                opts,
            )
        })
        .collect();
    if unique::handles(s) {
        return Value::Array(unique::dedupe(out, s, field, (seed, group, depth), opts));
    }
    Value::Array(out)
}

/// **Private.** Rounds to `places` decimal digits.
pub(super) fn round(v: f64, places: i32) -> f64 {
    let p = 10f64.powi(places);
    (v * p).round() / p
}
