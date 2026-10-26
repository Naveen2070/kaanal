//! `multipleOf` (D2 spec section 5.5): a generated number is moved to the nearest allowed multiple.
//!
//! Decimals are handled as integers scaled by a power of ten (money: `0.01`), never with float
//! modulo, so `19.99` is a multiple of `0.01` exactly. A multiple that does not fit the bounds,
//! a fractional `multipleOf` on an integer field (other than `1/n`, which every integer meets),
//! or bounds too large to count in units, leaves the value as generated and is a startup warning
//! ([`problem`]).

use serde_json::{Value, json};

use super::numbers::{exclusive, int_bound};

/// **Private.** Most decimal places of a `multipleOf` that is handled exactly.
const MAX_PLACES: u32 = 6;

/// **Private.** Largest magnitude, in units, the arithmetic accepts: far below `i128`, so no sum
/// or product of a bound, a value and a step can overflow.
const MAX_UNITS: f64 = 1e15;

/// **Private.** The positive finite `multipleOf` of `s`, if any.
fn multiple_of(s: &Value) -> Option<f64> {
    s.get("multipleOf")
        .and_then(Value::as_f64)
        .filter(|m| m.is_finite() && *m > 0.0)
}

/// **Private.** Whether every integer is a multiple of `m`: `1`, `0.5`, `0.25`, `1/n`.
fn every_integer(m: f64) -> bool {
    m <= 1.0 && ((1.0 / m).round() - 1.0 / m).abs() < 1e-9
}

/// **Private.** `m` as (integer step, scale): `0.25` is `(25, 100)`. `None` past [`MAX_PLACES`].
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    reason = "a handful of decimal digits, checked by the round trip"
)]
fn scaled(m: f64) -> Option<(i128, i128)> {
    (0..=MAX_PLACES).find_map(|p| {
        let scale = 10i128.pow(p);
        let step = (m * scale as f64).round();
        let fits = (1.0..MAX_UNITS).contains(&step) && (step / scale as f64 - m).abs() < 1e-12;
        fits.then_some((step as i128, scale))
    })
}

/// **Private.** `v` in units of `1/scale`, rounded; `None` when it is not finite or too large.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    reason = "checked against MAX_UNITS first"
)]
fn units(v: f64, scale: i128) -> Option<i128> {
    let u = v * scale as f64;
    (u.is_finite() && u.abs() <= MAX_UNITS).then(|| u.round() as i128)
}

/// **Private.** Nearest multiple of `step` to `x` inside `lo..=hi` (all in the same unit, all
/// within [`MAX_UNITS`], so nothing here can overflow).
fn nearest(x: i128, step: i128, lo: Option<i128>, hi: Option<i128>) -> Option<i128> {
    let mut c = (x + step / 2).div_euclid(step) * step;
    if let Some(lo) = lo
        && c < lo
    {
        c = (lo + step - 1).div_euclid(step) * step;
    }
    if let Some(hi) = hi
        && c > hi
    {
        c = hi.div_euclid(step) * step;
    }
    (lo.is_none_or(|l| c >= l) && hi.is_none_or(|h| c <= h)).then_some(c)
}

/// The integer bounds of `s` in units of `1/scale`, exclusive ones made inclusive. A float that
/// is within rounding error of a whole unit (`1.1 * 100 = 110.00000000000001`) counts as that
/// unit. `None` when a bound is too large to count.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    reason = "checked against MAX_UNITS first"
)]
fn bounds(s: &Value, scale: i128) -> Option<(Option<i128>, Option<i128>)> {
    let at = |k: &str, excl: bool, up: bool| -> Option<Option<i128>> {
        let Some(v) = s.get(k).and_then(Value::as_f64) else {
            return Some(None);
        };
        let u = v * scale as f64;
        if !u.is_finite() || u.abs() > MAX_UNITS {
            return None;
        }
        let near = u.round();
        let whole = (u - near).abs() < 1e-6;
        let n = if whole {
            near
        } else if up {
            u.ceil()
        } else {
            u.floor()
        } as i128;
        // a strict bound that lands on a whole unit moves one unit inward
        Some(Some(match (whole && excl, up) {
            (true, true) => n + 1,
            (true, false) => n - 1,
            _ => n,
        }))
    };
    Some((
        at("minimum", exclusive(s, "exclusiveMinimum"), true)?,
        at("maximum", exclusive(s, "exclusiveMaximum"), false)?,
    ))
}

/// `v` moved to the nearest multiple of the schema's `multipleOf` inside its bounds, or `v`
/// unchanged when the schema has none or no multiple fits. `ty` is `integer` or `number`.
#[expect(
    clippy::cast_possible_truncation,
    clippy::cast_precision_loss,
    reason = "step and bounds are within MAX_UNITS"
)]
pub(super) fn snap(v: Value, ty: &str, s: &Value) -> Value {
    let Some(m) = multiple_of(s) else { return v };
    if ty == "integer" {
        let (Some(x), true) = (v.as_i64(), m.fract() == 0.0 && m < MAX_UNITS) else {
            return v;
        };
        let (lo, hi) = (
            int_bound(s, "minimum", true).map(i128::from),
            int_bound(s, "maximum", false).map(i128::from),
        );
        return nearest(i128::from(x), m as i128, lo, hi)
            .and_then(|c| i64::try_from(c).ok())
            .map_or(v, |c| json!(c));
    }
    let (Some(x), Some((step, scale))) = (v.as_f64(), scaled(m)) else {
        return v;
    };
    let (Some(at), Some((lo, hi))) = (units(x, scale), bounds(s, scale)) else {
        return v;
    };
    nearest(at, step, lo, hi).map_or(v, |c| json!(c as f64 / scale as f64))
}

/// Why `multipleOf` on a field with schema `s` cannot be honoured, or `None` when it can (or
/// when there is none, or the field is not a number). The line is added to the startup warnings.
#[expect(
    clippy::cast_possible_truncation,
    reason = "the integer step is below MAX_UNITS, checked just above"
)]
pub(crate) fn problem(s: &Value) -> Option<String> {
    let kind = super::scalar_type(s);
    if !matches!(kind, "integer" | "number") {
        return None;
    }
    let m = multiple_of(s)?;
    let fits = if kind == "integer" {
        if every_integer(m) {
            return None;
        }
        if m.fract() != 0.0 || m >= MAX_UNITS {
            return Some(format!(
                "multipleOf {m} on an integer field is not supported; generated values may not match"
            ));
        }
        let (lo, hi) = (
            int_bound(s, "minimum", true).map(i128::from),
            int_bound(s, "maximum", false).map(i128::from),
        );
        nearest(lo.or(hi).unwrap_or(0), m as i128, lo, hi).is_some()
    } else {
        let Some((step, scale)) = scaled(m) else {
            return Some(format!(
                "multipleOf {m} has more than {MAX_PLACES} decimal places or is too large and is not supported; generated values may not match"
            ));
        };
        let Some((lo, hi)) = bounds(s, scale) else {
            return Some(format!(
                "multipleOf {m} with a minimum or maximum beyond 1e15 units is not supported; generated values may not match"
            ));
        };
        nearest(lo.or(hi).unwrap_or(0), step, lo, hi).is_some()
    };
    (!fits).then(|| {
        format!(
            "multipleOf {m} has no multiple between minimum and maximum; generated values ignore it"
        )
    })
}
