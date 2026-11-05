//! A circuit breaker on an edge (RULES 82-84, step 7c): after enough failed calls the caller stops
//! calling for a while. Off unless `breaker:` is written.
//!
//! The model is resilience4j's count-based sliding-window breaker (Nygard, *Release It!*, 2007): closed,
//! open and half-open; defaults are resilience4j's (window 100, minimum calls 100, failure rate 50%, 60 s
//! open, 10 half-open calls).

use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, de};

/// Largest sliding window.
const MAX_WINDOW: u16 = 1000;
/// Most calls a half-open breaker lets through.
const MAX_PROBES: u16 = 100;
/// Longest `open_for`.
const MAX_OPEN: Duration = Duration::from_secs(3600);

/// When a caller stops calling an edge, and when it tries again (`breaker:` on an edge).
#[derive(Debug, Clone, Copy, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BreakerConfig {
    /// Calls the sliding window remembers, 1 to 1000; default 100.
    #[serde(default = "default_window", deserialize_with = "window")]
    #[schemars(extend("minimum" = 1, "maximum" = 1000))]
    pub window: u16,
    /// Calls that must be in the window before the failure rate is looked at, at least 1; default 100.
    /// Above `window` it counts as `window` (as resilience4j does for a count-based window).
    #[serde(default = "default_min_calls", deserialize_with = "min_calls")]
    #[schemars(extend("minimum" = 1))]
    pub min_calls: u16,
    /// Failure rate at or above which the breaker opens, above 0 and at most 1; default 0.5.
    #[serde(default = "default_rate", deserialize_with = "rate")]
    #[schemars(extend("exclusiveMinimum" = 0, "maximum" = 1))]
    pub failure_rate: f64,
    /// How long it stays open before the next call is let through, above 0 and at most 1h; default 60s.
    #[serde(default = "default_open", deserialize_with = "open_for")]
    #[schemars(schema_with = "crate::schema::duration_text")]
    pub open_for: Duration,
    /// Calls a half-open breaker lets through to decide, 1 to 100; default 10.
    #[serde(default = "default_probes", deserialize_with = "probes")]
    #[schemars(extend("minimum" = 1, "maximum" = 100))]
    pub half_open_calls: u16,
}

impl Default for BreakerConfig {
    fn default() -> Self {
        BreakerConfig {
            window: default_window(),
            min_calls: default_min_calls(),
            failure_rate: default_rate(),
            open_for: default_open(),
            half_open_calls: default_probes(),
        }
    }
}

impl BreakerConfig {
    /// The window as the engine uses it: calls needed before the rate counts (`min_calls` capped to
    /// `window`).
    #[must_use]
    pub fn min_effective(&self) -> u16 {
        self.min_calls.min(self.window)
    }

    /// The failure rate in thousandths, so the engine compares integers.
    #[must_use]
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "failure_rate is validated to 0.001..=1, so the product is 1..=1000"
    )]
    pub fn rate_milli(&self) -> u16 {
        (self.failure_rate * 1000.0).round() as u16
    }
}

fn default_window() -> u16 {
    100
}
fn default_min_calls() -> u16 {
    100
}
fn default_rate() -> f64 {
    0.5
}
fn default_open() -> Duration {
    Duration::from_secs(60)
}
fn default_probes() -> u16 {
    10
}

/// **Private.** A whole number `lo..=hi`, named `field` in the message.
fn count<'de, D: Deserializer<'de>>(d: D, field: &str, lo: u16, hi: u16) -> Result<u16, D::Error> {
    let n = i64::deserialize(d).map_err(|_| {
        de::Error::custom(format!("breaker.{field}: use a whole number, {lo} to {hi}"))
    })?;
    u16::try_from(n)
        .ok()
        .filter(|n| (lo..=hi).contains(n))
        .ok_or_else(|| {
            de::Error::custom(format!(
                "breaker.{field}: {n} is out of range; use {lo} to {hi}"
            ))
        })
}

/// **Private.** `breaker.window`.
fn window<'de, D: Deserializer<'de>>(d: D) -> Result<u16, D::Error> {
    count(d, "window", 1, MAX_WINDOW)
}

/// **Private.** `breaker.min_calls` (no upper bound beyond the type: it is capped to the window).
fn min_calls<'de, D: Deserializer<'de>>(d: D) -> Result<u16, D::Error> {
    count(d, "min_calls", 1, u16::MAX)
}

/// **Private.** `breaker.half_open_calls`.
fn probes<'de, D: Deserializer<'de>>(d: D) -> Result<u16, D::Error> {
    count(d, "half_open_calls", 1, MAX_PROBES)
}

/// **Private.** `breaker.failure_rate`, above 0 and at most 1.
fn rate<'de, D: Deserializer<'de>>(d: D) -> Result<f64, D::Error> {
    let v = f64::deserialize(d).map_err(|_| {
        de::Error::custom("breaker.failure_rate: use a number above 0 and at most 1, like 0.5")
    })?;
    // below half a thousandth the integer rate would round to 0
    if (0.001..=1.0).contains(&v) {
        Ok(v)
    } else {
        Err(de::Error::custom(format!(
            "breaker.failure_rate: {v} is out of range; use 0.001 to 1"
        )))
    }
}

/// **Private.** `breaker.open_for`, above 0 and at most 1h.
fn open_for<'de, D: Deserializer<'de>>(d: D) -> Result<Duration, D::Error> {
    let v: Duration = humantime_serde::deserialize(d).map_err(|_| {
        de::Error::custom("breaker.open_for: use a duration with a unit, like `30s` or `1m`")
    })?;
    if v.is_zero() || v > MAX_OPEN {
        return Err(de::Error::custom(format!(
            "breaker.open_for: {v:?} is out of range; use above 0 and at most 1h"
        )));
    }
    Ok(v)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(y: &str) -> Result<BreakerConfig, String> {
        serde_yaml_ng::from_str(y).map_err(|e| e.to_string())
    }

    #[test]
    fn an_empty_breaker_is_resilience4js_defaults() {
        let c = parse("{}").unwrap();
        assert_eq!(c, BreakerConfig::default());
        assert_eq!((c.window, c.min_calls, c.half_open_calls), (100, 100, 10));
        assert_eq!((c.rate_milli(), c.open_for), (500, Duration::from_secs(60)));
    }

    #[test]
    fn a_full_breaker_parses_and_min_calls_caps_to_the_window() {
        let c = parse(
            "window: 20\nmin_calls: 50\nfailure_rate: 0.25\nopen_for: 5s\nhalf_open_calls: 3",
        )
        .unwrap();
        assert_eq!((c.window, c.min_calls, c.min_effective()), (20, 50, 20));
        assert_eq!((c.rate_milli(), c.half_open_calls), (250, 3));
        assert_eq!(c.open_for, Duration::from_secs(5));
    }

    #[test]
    fn every_bad_value_names_its_field() {
        let cases = [
            ("window: 0", "breaker.window"),
            ("window: 1001", "breaker.window"),
            ("window: wide", "breaker.window"),
            ("min_calls: 0", "breaker.min_calls"),
            ("failure_rate: 0", "breaker.failure_rate"),
            ("failure_rate: 1.5", "breaker.failure_rate"),
            ("failure_rate: half", "breaker.failure_rate"),
            ("open_for: 0s", "breaker.open_for"),
            ("open_for: 2h", "breaker.open_for"),
            ("open_for: soon", "breaker.open_for"),
            ("half_open_calls: 0", "breaker.half_open_calls"),
            ("half_open_calls: 101", "breaker.half_open_calls"),
        ];
        for (y, field) in cases {
            let e = parse(y).unwrap_err();
            assert!(e.contains(field), "{y}: {e}");
        }
        assert!(parse("window: 5\nbogus: 1").is_err());
    }
}
