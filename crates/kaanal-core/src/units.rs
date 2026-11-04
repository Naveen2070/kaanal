//! Byte and rate values in the config: a whole number of bytes (a rate: bytes a second) or text
//! like `250KB`, `1.5MB`, `512B`, `1MB/s`. One parser and one set of `deserialize_with` functions
//! serve every field (node and edge `bandwidth`, a storage node's `throughput`, a journey's `req_bytes` and `resp_bytes`, the
//! `bandwidth_cap` chaos value, in YAML and in the control API's JSON), so they all read and fail
//! the same way. KB = 1024 bytes, the same as Studio's unit picker. Bounds (at least 1, at most
//! 1 GiB) stay with each field's own check and run on the converted number.

use serde::de::{self, Deserializer, Visitor};
use std::fmt;

/// Units a size takes, for the error text.
const SIZE_UNITS: &str = "B, KB, MB, GB";
/// Units a rate takes, for the error text.
const RATE_UNITS: &str = "B, KB, MB, GB, optionally /s for a rate";
/// First `f64` too big for a `u64`.
const U64_END: f64 = 18_446_744_073_709_551_616.0;

/// Parses `text` into bytes. `rate` allows a trailing `/s`. A number without a unit is bytes.
///
/// # Errors
/// What is wrong with `text` and the units that are accepted (no field name; the caller adds it).
pub fn parse(text: &str, rate: bool) -> Result<u64, String> {
    let units = if rate { RATE_UNITS } else { SIZE_UNITS };
    let t = text.trim();
    if t.starts_with('-') {
        return Err(format!("`{t}` is negative; use a number of 0 or more"));
    }
    let digits = |s: &str| s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    let (int, mut rest) = t.split_at(digits(t));
    if int.is_empty() {
        return Err(format!(
            "`{t}` has no number; write a number and a unit ({units})"
        ));
    }
    let mut frac = "";
    if let Some(r) = rest.strip_prefix('.') {
        let n = digits(r);
        if n == 0 {
            return Err(format!(
                "`{t}` has no digits after the point; write a number and a unit ({units})"
            ));
        }
        (frac, rest) = r.split_at(n);
    }
    let lower = rest.trim().to_ascii_lowercase();
    let (unit, per_s) = match lower.strip_suffix("/s") {
        Some(u) => (u.trim_end(), true),
        None => (lower.as_str(), false),
    };
    if per_s && !rate {
        return Err(format!(
            "`{t}` is a size, not a rate; drop the `/s` (units: {units})"
        ));
    }
    let mult: u32 = match unit {
        "" | "b" => 1,
        "kb" | "kib" => 1 << 10,
        "mb" | "mib" => 1 << 20,
        "gb" | "gib" => 1 << 30,
        _ => return Err(format!("unknown unit in `{t}`; use {units}")),
    };
    let too_big = || format!("`{t}` is too large (at most {} bytes)", u64::MAX);
    if frac.is_empty() {
        let n: u64 = int.parse().map_err(|_| too_big())?;
        return n.checked_mul(u64::from(mult)).ok_or_else(too_big);
    }
    // A fraction rounds to the nearest whole byte.
    let v = (format!("{int}.{frac}")
        .parse::<f64>()
        .map_err(|_| too_big())?
        * f64::from(mult))
    .round();
    if v >= U64_END {
        return Err(too_big());
    }
    // ponytail: f64 is exact enough for a fraction of a unit; whole numbers take the exact path above.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let bytes = v as u64;
    Ok(bytes)
}

/// **Private.** What a YAML or JSON value was before it is parsed: the field name is not known
/// to the visitor, so the wrapper turns this into a message that names it.
enum Raw {
    Int(u64),
    Text(String),
    Bad(String),
}

struct RawVisitor;

impl Visitor<'_> for RawVisitor {
    type Value = Raw;

    fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("a whole number of bytes or text like `250KB`")
    }

    fn visit_u64<E: de::Error>(self, v: u64) -> Result<Raw, E> {
        Ok(Raw::Int(v))
    }

    fn visit_i64<E: de::Error>(self, v: i64) -> Result<Raw, E> {
        Ok(u64::try_from(v).map_or_else(
            |_| Raw::Bad(format!("{v} is negative; use a number of 0 or more")),
            Raw::Int,
        ))
    }

    fn visit_f64<E: de::Error>(self, v: f64) -> Result<Raw, E> {
        Ok(Raw::Bad(format!(
            "{v} is not a whole number of bytes; write it in text with a unit, like `1.5KB`"
        )))
    }

    fn visit_str<E: de::Error>(self, v: &str) -> Result<Raw, E> {
        Ok(Raw::Text(v.to_owned()))
    }
}

impl<'de> de::Deserialize<'de> for Raw {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        d.deserialize_any(RawVisitor)
    }
}

/// **Private.** The shared body of every `deserialize_with` function below: an absent or `null`
/// value is `None`.
fn field<'de, D: Deserializer<'de>>(d: D, name: &str, rate: bool) -> Result<Option<u64>, D::Error> {
    let msg = |m: String| de::Error::custom(format!("{name}: {m}"));
    match <Option<Raw> as de::Deserialize>::deserialize(d)? {
        None => Ok(None),
        Some(Raw::Int(n)) => Ok(Some(n)),
        Some(Raw::Text(t)) => parse(&t, rate).map(Some).map_err(msg),
        Some(Raw::Bad(m)) => Err(msg(m)),
    }
}

/// Parses the rate of a node's `bandwidth: 250KB/s` (no `null` handling: the node type does it).
///
/// # Errors
/// As [`parse`], naming `bandwidth`.
pub fn node_rate<E: de::Error>(text: &str) -> Result<u64, E> {
    parse(text, true).map_err(|m| E::custom(format!("bandwidth: {m}")))
}

macro_rules! field_fns {
    ($($f:ident, $name:literal, $rate:literal;)*) => {$(
        #[doc = concat!("`deserialize_with` for `", $name, "`: a whole number or text with a unit, `None` when absent.")]
        ///
        /// # Errors
        /// A message naming the field and the accepted units.
        pub fn $f<'de, D: Deserializer<'de>>(d: D) -> Result<Option<u64>, D::Error> {
            field(d, $name, $rate)
        }
    )*};
}

field_fns! {
    bandwidth, "bandwidth", true;
    throughput, "throughput", true;
    out, "out", true;
    inbound, "in", true;
    bandwidth_cap, "bandwidth_cap", true;
    req_bytes, "req_bytes", false;
    resp_bytes, "resp_bytes", false;
}

#[cfg(test)]
mod tests {
    use super::parse;

    #[test]
    fn accepted_forms() {
        for (text, rate, want) in [
            ("250KB", false, 256_000),
            ("250 KB", false, 256_000),
            ("250kb", false, 256_000),
            ("250KiB", false, 256_000),
            ("1.5MB", false, 1_572_864),
            ("2GB", false, 2 << 30),
            ("512B", false, 512),
            ("512", false, 512),
            ("0KB", false, 0),
            ("1MB/s", true, 1 << 20),
            ("250KB/s", true, 256_000),
            ("250 kb/S", true, 256_000),
            ("1.0001B", false, 1),
            ("1.5B", false, 2),
            ("18446744073709551615B", false, u64::MAX),
        ] {
            assert_eq!(parse(text, rate), Ok(want), "{text}");
        }
    }

    #[test]
    fn refusals_name_the_problem_and_the_units() {
        for (text, rate, want) in [
            (
                "KB",
                false,
                "`KB` has no number; write a number and a unit (B, KB, MB, GB)",
            ),
            (
                "",
                true,
                "`` has no number; write a number and a unit (B, KB, MB, GB, optionally /s for a rate)",
            ),
            (
                "-5KB",
                false,
                "`-5KB` is negative; use a number of 0 or more",
            ),
            ("5XB", false, "unknown unit in `5XB`; use B, KB, MB, GB"),
            (
                "NaNKB",
                false,
                "`NaNKB` has no number; write a number and a unit (B, KB, MB, GB)",
            ),
            (
                "1e400KB",
                true,
                "unknown unit in `1e400KB`; use B, KB, MB, GB, optionally /s for a rate",
            ),
            (
                "5KB/s",
                false,
                "`5KB/s` is a size, not a rate; drop the `/s` (units: B, KB, MB, GB)",
            ),
            (
                "5.KB",
                false,
                "`5.KB` has no digits after the point; write a number and a unit (B, KB, MB, GB)",
            ),
            (
                "18446744073709551616B",
                false,
                "`18446744073709551616B` is too large (at most 18446744073709551615 bytes)",
            ),
            (
                "17179869184GB",
                false,
                "`17179869184GB` is too large (at most 18446744073709551615 bytes)",
            ),
            (
                "99999999999.5GB",
                false,
                "`99999999999.5GB` is too large (at most 18446744073709551615 bytes)",
            ),
        ] {
            assert_eq!(parse(text, rate), Err(want.to_string()), "{text}");
        }
    }
}
