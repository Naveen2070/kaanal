//! Retries on an edge (RULES 82-84, step 7a): how many tries a failed call gets, how long it waits
//! between them, and the optional retry budget. Off unless `retry:` is written.
//!
//! Defaults follow the AWS SDK "standard" mode and gRPC: three attempts counting the first, 50 ms base,
//! doubling, 20 s cap, full jitter; the budget is gRPC's A6 retry throttling (`maxTokens` 10,
//! `tokenRatio` 0.1 are the values its documentation shows).

use std::time::Duration;

use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, de};

use crate::exec::Jitter;

/// Most tries a call may make, the first included.
pub const MAX_ATTEMPTS: u8 = 5;
/// Tries when `max_attempts` is not written: the AWS SDK's and gRPC's default.
pub const DEFAULT_MAX_ATTEMPTS: u8 = 3;
/// Longest wait or cap a retry may be configured with.
const MAX_WAIT: Duration = Duration::from_secs(3600);
/// Largest backoff multiplier.
const MAX_MULTIPLIER: u32 = 10;
/// Largest budget (gRPC A6 allows up to 1000 tokens).
const MAX_TOKENS: u32 = 1000;

/// What a failed call does about it (`retry:` on an edge, or on a `call:` step, which wins).
#[derive(Debug, Clone, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RetryConfig {
    /// Tries a call makes, the first included: 2 to 5, default 3 (gRPC `maxAttempts`, the AWS SDK's
    /// `max_attempts`). A failure that is not transient (a 4xx other than 429, a 501) is never retried.
    #[serde(default = "default_attempts", deserialize_with = "attempts")]
    #[schemars(extend("minimum" = 2, "maximum" = 5))]
    pub max_attempts: u8,
    /// Wait before the first retry, above 0 and at most 1h; default 50ms (AWS SDK).
    #[serde(default = "default_backoff", deserialize_with = "wait_backoff")]
    #[schemars(schema_with = "crate::schema::duration_text")]
    pub backoff: Duration,
    /// Each wait is this many times the one before, 1 to 10; default 2.
    #[serde(default = "default_multiplier", deserialize_with = "multiplier")]
    #[schemars(extend("minimum" = 1, "maximum" = 10))]
    pub multiplier: u32,
    /// The longest a wait grows to, above 0 and at most 1h; default 20s (AWS SDK).
    #[serde(default = "default_cap", deserialize_with = "wait_cap")]
    #[schemars(schema_with = "crate::schema::duration_text")]
    pub cap: Duration,
    /// How a wait is randomised: `full` (default, the whole wait is `U(0, wait)`), `equal` (half fixed,
    /// half random) or `none`.
    #[serde(default)]
    pub jitter: Jitter,
    /// Retry a **write** to a database or service too (default false: a retry of a non-idempotent write
    /// can apply it twice). A write to a `storage` node always retries, as S3 PUT and DELETE are
    /// idempotent (RFC 9110 section 9.2.2).
    #[serde(default)]
    pub writes: bool,
    /// A retry budget; absent = none (retries are limited only by `max_attempts`).
    #[serde(default)]
    pub budget: Option<Budget>,
}

/// gRPC A6 retry throttling: a token count that starts full, loses 1 for each failed call and gains
/// `token_ratio` for each success; at or below half of `max_tokens` no retry is made.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Budget {
    /// Size of the budget, 1 to 1000; default 10.
    #[serde(default = "default_tokens", deserialize_with = "tokens")]
    #[schemars(extend("minimum" = 1, "maximum" = 1000))]
    pub max_tokens: u32,
    /// Tokens a success adds back, above 0 and at most 1; default 0.1.
    #[serde(default = "default_ratio", deserialize_with = "ratio")]
    #[schemars(extend("exclusiveMinimum" = 0, "maximum" = 1))]
    pub token_ratio: f64,
}

impl Default for RetryConfig {
    fn default() -> Self {
        RetryConfig {
            max_attempts: DEFAULT_MAX_ATTEMPTS,
            backoff: default_backoff(),
            multiplier: default_multiplier(),
            cap: default_cap(),
            jitter: Jitter::default(),
            writes: false,
            budget: None,
        }
    }
}

impl Default for Budget {
    fn default() -> Self {
        Budget {
            max_tokens: default_tokens(),
            token_ratio: default_ratio(),
        }
    }
}

fn default_attempts() -> u8 {
    DEFAULT_MAX_ATTEMPTS
}
fn default_backoff() -> Duration {
    Duration::from_millis(50)
}
fn default_multiplier() -> u32 {
    2
}
fn default_cap() -> Duration {
    Duration::from_secs(20)
}
fn default_tokens() -> u32 {
    10
}
fn default_ratio() -> f64 {
    0.1
}

/// **Private.** `retry.max_attempts`, 2 to [`MAX_ATTEMPTS`].
fn attempts<'de, D: Deserializer<'de>>(d: D) -> Result<u8, D::Error> {
    let n = i64::deserialize(d).map_err(|_| {
        de::Error::custom(format!(
            "retry.max_attempts: use a whole number of tries, 2 to {MAX_ATTEMPTS} (the first counts)"
        ))
    })?;
    u8::try_from(n)
        .ok()
        .filter(|n| (2..=MAX_ATTEMPTS).contains(n))
        .ok_or_else(|| {
            de::Error::custom(format!(
                "retry.max_attempts: {n} is out of range; use 2 to {MAX_ATTEMPTS} (the first try counts, so 1 would never retry)"
            ))
        })
}

/// **Private.** A duration above 0 and at most [`MAX_WAIT`], named `field` in the message.
fn wait<'de, D: Deserializer<'de>>(d: D, field: &str) -> Result<Duration, D::Error> {
    let v: Duration = humantime_serde::deserialize(d).map_err(|_| {
        de::Error::custom(format!(
            "retry.{field}: use a duration with a unit, like `50ms` or `2s`"
        ))
    })?;
    if v.is_zero() || v > MAX_WAIT {
        return Err(de::Error::custom(format!(
            "retry.{field}: {v:?} is out of range; use above 0 and at most 1h"
        )));
    }
    Ok(v)
}

/// **Private.** `retry.backoff`.
fn wait_backoff<'de, D: Deserializer<'de>>(d: D) -> Result<Duration, D::Error> {
    wait(d, "backoff")
}

/// **Private.** `retry.cap`.
fn wait_cap<'de, D: Deserializer<'de>>(d: D) -> Result<Duration, D::Error> {
    wait(d, "cap")
}

/// **Private.** `retry.multiplier`, 1 to [`MAX_MULTIPLIER`].
fn multiplier<'de, D: Deserializer<'de>>(d: D) -> Result<u32, D::Error> {
    let n = i64::deserialize(d).map_err(|_| {
        de::Error::custom(format!(
            "retry.multiplier: use a whole number, 1 to {MAX_MULTIPLIER}"
        ))
    })?;
    u32::try_from(n)
        .ok()
        .filter(|n| (1..=MAX_MULTIPLIER).contains(n))
        .ok_or_else(|| {
            de::Error::custom(format!(
                "retry.multiplier: {n} is out of range; use 1 (a constant wait) to {MAX_MULTIPLIER}"
            ))
        })
}

/// **Private.** `retry.budget.max_tokens`, 1 to [`MAX_TOKENS`].
fn tokens<'de, D: Deserializer<'de>>(d: D) -> Result<u32, D::Error> {
    let n = i64::deserialize(d).map_err(|_| {
        de::Error::custom(format!(
            "retry.budget.max_tokens: use a whole number, 1 to {MAX_TOKENS}"
        ))
    })?;
    u32::try_from(n)
        .ok()
        .filter(|n| (1..=MAX_TOKENS).contains(n))
        .ok_or_else(|| {
            de::Error::custom(format!(
                "retry.budget.max_tokens: {n} is out of range; use 1 to {MAX_TOKENS}"
            ))
        })
}

/// **Private.** `retry.budget.token_ratio`, above 0 and at most 1.
fn ratio<'de, D: Deserializer<'de>>(d: D) -> Result<f64, D::Error> {
    let v = f64::deserialize(d).map_err(|_| {
        de::Error::custom("retry.budget.token_ratio: use a number above 0 and at most 1, like 0.1")
    })?;
    if v > 0.0 && v <= 1.0 {
        Ok(v)
    } else {
        Err(de::Error::custom(format!(
            "retry.budget.token_ratio: {v} is out of range; use above 0 and at most 1"
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(y: &str) -> Result<RetryConfig, String> {
        serde_yaml_ng::from_str(y).map_err(|e| e.to_string())
    }

    #[test]
    fn an_empty_retry_is_the_documented_defaults() {
        let c = parse("{}").unwrap();
        assert_eq!(c, RetryConfig::default());
        assert_eq!(
            (c.max_attempts, c.multiplier, c.jitter, c.writes),
            (3, 2, Jitter::Full, false)
        );
        assert_eq!(
            (c.backoff, c.cap),
            (Duration::from_millis(50), Duration::from_secs(20))
        );
        assert!(c.budget.is_none());
    }

    #[test]
    fn a_full_retry_parses() {
        let c = parse(
            "max_attempts: 5\nbackoff: 100ms\nmultiplier: 3\ncap: 2s\njitter: equal\nwrites: true\n\
             budget: { max_tokens: 20, token_ratio: 0.2 }",
        )
        .unwrap();
        assert_eq!(c.max_attempts, 5);
        assert_eq!(c.backoff, Duration::from_millis(100));
        assert_eq!((c.multiplier, c.jitter, c.writes), (3, Jitter::Equal, true));
        assert_eq!(
            c.budget,
            Some(Budget {
                max_tokens: 20,
                token_ratio: 0.2
            })
        );
        assert_eq!(parse("budget: {}").unwrap().budget, Some(Budget::default()));
    }

    #[test]
    fn every_bad_value_names_its_field() {
        let cases = [
            ("max_attempts: 1", "retry.max_attempts"),
            ("max_attempts: 6", "retry.max_attempts"),
            ("max_attempts: 0", "retry.max_attempts"),
            ("max_attempts: many", "retry.max_attempts"),
            ("backoff: 0s", "retry.backoff"),
            ("backoff: 2h", "retry.backoff"),
            ("backoff: soon", "retry.backoff"),
            ("cap: 0s", "retry.cap"),
            ("multiplier: 0", "retry.multiplier"),
            ("multiplier: 11", "retry.multiplier"),
            ("jitter: wild", "jitter"),
            ("budget: { max_tokens: 0 }", "retry.budget.max_tokens"),
            ("budget: { max_tokens: 1001 }", "retry.budget.max_tokens"),
            ("budget: { token_ratio: 0 }", "retry.budget.token_ratio"),
            ("budget: { token_ratio: -1 }", "retry.budget.token_ratio"),
            ("budget: { token_ratio: 1.5 }", "retry.budget.token_ratio"),
            ("max: 3", "unknown field"),
        ];
        for (y, want) in cases {
            let e = parse(y).expect_err(y);
            assert!(e.contains(want), "`{y}` -> {e}");
        }
    }
}
