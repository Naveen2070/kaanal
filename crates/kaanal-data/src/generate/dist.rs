//! Realistic distributions (D2 spec section 5): integer weighted tables and bucketed uniform draws.
//!
//! No `ln`, `exp`, `powf` or float distribution sampling: Rust documents those as having
//! platform-dependent precision, which would make a golden differ between Windows and Linux.
//! Everything here is integer arithmetic plus the basic float operations (`+ - * /`), which
//! IEEE 754 rounds exactly the same everywhere. Every function draws only from the rng it is
//! given, which is the field's own.

use std::collections::BTreeMap;

use rand::RngExt;
use rand_chacha::ChaCha8Rng;
use serde_json::{Value, json};

use super::numbers::round;
use super::{numeric, value_fits};
use crate::dictionary::{MoneyDist, MoneyStyle};

/// **Private.** Weights of the first decades of a price range, cheapest first; later ones get 2.
const DECADE_WEIGHTS: [f64; 4] = [45.0, 35.0, 15.0, 5.0];
/// **Private.** Most decade buckets a range is cut into.
const MAX_BUCKETS: usize = 12;
/// **Private.** Weights of the four equal-width bands of an integer range (`dist: bands`).
const BAND_WEIGHTS: [u64; 4] = [25, 30, 25, 20];

/// The weight of `v` in `w`: matched by its JSON text (`"paid"`, `true`, `4`), 1 when unlisted.
pub(crate) fn weight_of(value: &Value, weights: &BTreeMap<String, u32>) -> u64 {
    let look = |text: &str| weights.get(text).map_or(1, |n| u64::from(*n));
    match value {
        Value::String(text) => look(text),
        Value::Bool(flag) => look(if *flag { "true" } else { "false" }),
        // a whole number is formatted on the stack: this runs once per candidate per draw
        Value::Number(num) if whole(num).is_some() => {
            let mut buf = [0u8; 20];
            look(int_text(whole(num).unwrap_or(0), &mut buf))
        }
        other => look(&other.to_string()),
    }
}

/// **Private.** A whole-valued JSON number as an `i64`: `4` and `4.0` are the same weight key.
#[expect(
    clippy::cast_possible_truncation,
    reason = "checked whole and below 1e15 just before"
)]
fn whole(num: &serde_json::Number) -> Option<i64> {
    num.as_i64().or_else(|| {
        num.as_f64()
            .filter(|f| f.fract() == 0.0 && f.abs() < 1e15)
            .map(|f| f as i64)
    })
}

/// **Private.** `x` as decimal text in `buf` (20 bytes hold any `i64`), without allocating.
fn int_text(x: i64, buf: &mut [u8; 20]) -> &str {
    let mut at = buf.len();
    let mut rest = x.unsigned_abs();
    loop {
        at -= 1;
        buf[at] = b'0' + u8::try_from(rest % 10).unwrap_or(0);
        rest /= 10;
        if rest == 0 {
            break;
        }
    }
    if x < 0 {
        at -= 1;
        buf[at] = b'-';
    }
    std::str::from_utf8(&buf[at..]).unwrap_or("")
}

/// One of `items`, drawn by `w` (unlisted values weigh 1). Falls back to uniform when every
/// weight is 0 (a dictionary load rejects that, but a spec enum can miss every listed value).
pub(crate) fn pick_weighted<'a>(
    items: &[&'a Value],
    w: &BTreeMap<String, u32>,
    rng: &mut ChaCha8Rng,
) -> &'a Value {
    let weights: Vec<u64> = items.iter().map(|v| weight_of(v, w)).collect();
    let total: u64 = weights.iter().sum();
    if total == 0 {
        return items[rng.random_range(0..items.len())];
    }
    let mut t = rng.random_range(0..total);
    for (item, weight) in items.iter().zip(&weights) {
        if t < *weight {
            return item;
        }
        t -= weight;
    }
    items[items.len() - 1]
}

/// A uniform fraction in `0.0..=1.0` from an integer draw.
fn unit(rng: &mut ChaCha8Rng) -> f64 {
    f64::from(rng.random_range(0..=1_000_000u32)) / 1_000_000.0
}

/// A price in `lo..=hi`: many cheap values, a few expensive. The range is cut at the powers of ten
/// inside it; a bucket's weight is its decade's weight (cheapest decade first) times the share of
/// the decade it covers. A range that is not positive is drawn uniformly.
pub(crate) fn decade(lo: f64, hi: f64, rng: &mut ChaCha8Rng) -> f64 {
    // below 1e-6 there is no price to speak of (and the power-of-ten walk below would not end)
    if !(lo >= 1e-6 && hi > lo && hi.is_finite()) {
        return if hi > lo && (hi - lo).is_finite() {
            lo + (hi - lo) * unit(rng)
        } else {
            lo
        };
    }
    // the largest power of ten at or below `lo`, by repeated multiplication (exact for small ones)
    let mut d = 1.0f64;
    while d > lo {
        d /= 10.0;
    }
    while d * 10.0 <= lo {
        d *= 10.0;
    }
    let mut buckets: Vec<(f64, f64, f64)> = Vec::new();
    let (mut a, mut idx) = (lo, 0usize);
    loop {
        let next = d * 10.0;
        // the last bucket allowed reaches `hi`, so the top of a huge range stays possible
        let last = buckets.len() + 1 >= MAX_BUCKETS;
        let b = if last { hi } else { next.min(hi) };
        let base = DECADE_WEIGHTS.get(idx).copied().unwrap_or(2.0);
        buckets.push((a, b, base * ((b - a) / (9.0 * d)).min(1.0)));
        idx += 1;
        if next >= hi || last {
            break;
        }
        (a, d) = (next, next);
    }
    let total: f64 = buckets.iter().map(|b| b.2).sum();
    let mut t = unit(rng) * total;
    for (a, b, w) in &buckets {
        if t <= *w {
            return a + (b - a) * unit(rng);
        }
        t -= w;
    }
    let (a, b, _) = buckets[buckets.len() - 1];
    a + (b - a) * unit(rng)
}

/// An integer in `lo..=hi` from four equal-width bands weighted 25/30/25/20 (how ages are
/// modelled: most people are 30 to 60). Needs `lo <= hi`; a range under 4 wide is uniform.
pub(crate) fn bands(lo: i64, hi: i64, rng: &mut ChaCha8Rng) -> i64 {
    let width = i128::from(hi) - i128::from(lo) + 1;
    if width < 4 {
        return rng.random_range(lo..=hi);
    }
    let mut t = rng.random_range(0..BAND_WEIGHTS.iter().sum::<u64>());
    let mut band = 0;
    for (i, w) in BAND_WEIGHTS.iter().enumerate() {
        band = i;
        if t < *w {
            break;
        }
        t -= w;
    }
    let edge = |i: usize| i128::from(lo) + width * i128::try_from(i).unwrap_or(0) / 4;
    let (from, to) = (edge(band), edge(band + 1) - 1);
    i64::try_from(from + i128::from(rng.random_range(0..=u64::try_from(to - from).unwrap_or(0))))
        .unwrap_or(lo)
}

/// Seconds back from the base date inside a window of `window` seconds, leaning recent: two
/// draws in three fall in the most recent third.
pub(crate) fn recency_secs(window: i64, rng: &mut ChaCha8Rng) -> i64 {
    let window = window.max(3);
    let third = window / 3;
    if rng.random_range(0..3) < 2 {
        rng.random_range(0..third)
    } else {
        rng.random_range(third..window)
    }
}

/// A price ending in `.99` half the time, `.00` a quarter, `.95` a tenth, `.49` one in twenty and
/// any cents otherwise, on the whole part of `v`.
fn retail(v: f64, rng: &mut ChaCha8Rng) -> f64 {
    let cents = match rng.random_range(0..100) {
        0..50 => 99,
        50..75 => 0,
        75..85 => 95,
        85..90 => 49,
        _ => rng.random_range(0..100),
    };
    round(v.floor() + f64::from(cents) / 100.0, 2)
}

/// A money value of scalar type `ty` for schema `s` in the entry's default range. `Decade`
/// skews it toward cheap, `Retail` gives non-integer values a retail ending; either falls back to
/// the plain uniform value when the result would not fit the schema's bounds.
#[expect(
    clippy::cast_possible_truncation,
    reason = "f64 -> i64 `as` saturates; the value is then checked against the schema's bounds"
)]
pub(crate) fn money(
    s: &Value,
    ty: &str,
    rng: &mut ChaCha8Rng,
    range: (f64, f64),
    (dist, style): (MoneyDist, MoneyStyle),
) -> Value {
    let (lo, hi) = (
        s.get("minimum").and_then(Value::as_f64).unwrap_or(range.0),
        s.get("maximum").and_then(Value::as_f64).unwrap_or(range.1),
    );
    let skewed = (dist == MoneyDist::Decade && lo > 0.0 && hi > lo && hi.is_finite()).then(|| {
        let v = decade(lo, hi, rng);
        if ty == "integer" {
            json!(round(v, 0) as i64)
        } else {
            json!(round(v, 2))
        }
    });
    let value = match skewed.filter(|c| value_fits(c, s, ty)) {
        Some(v) => v,
        None => numeric(s, ty, rng, range, 2),
    };
    if style == MoneyStyle::Retail
        && ty != "integer"
        && let Some(x) = value.as_f64()
    {
        let candidate = json!(retail(x, rng));
        if value_fits(&candidate, s, ty) {
            return candidate;
        }
    }
    value
}
