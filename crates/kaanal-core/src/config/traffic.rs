//! `traffic:`: named load profiles for the simulator. A profile is a total rate and a weighted
//! mix of journeys; a journey is one endpoint with a weight and optional payload sizes. The
//! simulator runs each journey as an independent Poisson stream, so a journey is a request
//! stream, not a user session (no think time, no state).

use super::{
    Deserialize, Deserializer, Duration, IndexMap, JsonSchema, Located, PathSeg, de, keys,
};
use crate::scenario::endpoint_key;

/// Fewest requests per second a profile may ask for (the simulator's slowest stream).
const MIN_RPS: f64 = 1e-6;
/// Most requests per second a profile may ask for (the simulator's fastest stream).
const MAX_RPS: f64 = 1e6;
/// Most journeys in one profile (each becomes a simulator stream).
const MAX_JOURNEYS: usize = 64;
/// Longest simulated time a profile or a run may ask for.
pub const MAX_DURATION: Duration = Duration::from_secs(3600);
/// Largest payload size a journey may declare, in bytes.
const MAX_BYTES: u64 = 1 << 30;

/// One named load profile.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TrafficConfig {
    /// Total requests per second across all journeys (0.000001 to 1000000).
    #[serde(deserialize_with = "rps_in_range")]
    pub rps: f64,
    /// The mix: each journey gets `rps × weight ÷ sum of weights`.
    pub journeys: Vec<JourneyConfig>,
    /// Simulated time of a run (default 60s); a run request may override it.
    #[serde(default, with = "humantime_serde")]
    #[schemars(schema_with = "crate::schema::duration_text")]
    pub duration: Option<Duration>,
    /// Simulated time before results count (default a tenth of the duration).
    #[serde(default, with = "humantime_serde")]
    #[schemars(schema_with = "crate::schema::duration_text")]
    pub warmup: Option<Duration>,
}

/// One endpoint in a profile's mix.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct JourneyConfig {
    /// The endpoint key, `"METHOD /path"`, as in `endpoints:`.
    pub endpoint: String,
    /// Relative share of the profile's rate (above 0, default 1).
    #[serde(default = "one", deserialize_with = "weight_above_zero")]
    pub weight: f64,
    /// Size of each request body: bytes as a whole number, or text with a unit like `2KB` (KB =
    /// 1024 bytes; at most 1 GiB). Default 0: no link or `per_kb` cost.
    #[serde(default, deserialize_with = "crate::units::req_bytes")]
    #[schemars(schema_with = "crate::schema::size")]
    pub req_bytes: Option<u64>,
    /// Size of each response body, written like `req_bytes` (default: the size of one generated
    /// body for a route with a response schema, else 0).
    #[serde(default, deserialize_with = "crate::units::resp_bytes")]
    #[schemars(schema_with = "crate::schema::size")]
    pub resp_bytes: Option<u64>,
}

/// **Private.** serde default for [`JourneyConfig::weight`].
fn one() -> f64 {
    1.0
}

/// **Private.** A rate the simulator accepts.
fn rps_in_range<'de, D: Deserializer<'de>>(d: D) -> Result<f64, D::Error> {
    let v = f64::deserialize(d)?;
    if (MIN_RPS..=MAX_RPS).contains(&v) {
        Ok(v)
    } else {
        Err(de::Error::custom(format!(
            "rps must be between 0.000001 and 1000000 (got {v})"
        )))
    }
}

/// **Private.** A weight that makes the journey possible.
fn weight_above_zero<'de, D: Deserializer<'de>>(d: D) -> Result<f64, D::Error> {
    let v = f64::deserialize(d)?;
    if v > 0.0 && v.is_finite() {
        Ok(v)
    } else {
        Err(de::Error::custom(format!(
            "weight must be above 0 and finite (got {v})"
        )))
    }
}

/// **Private.** What serde cannot place, for every profile: names, journeys and payloads.
pub(super) fn check_traffic(traffic: &IndexMap<String, TrafficConfig>) -> Result<(), Located> {
    traffic
        .iter()
        .try_for_each(|(name, p)| check_profile(name, p))
}

/// What serde cannot place in one profile: its name (the scenario-name rule), 1 to 64 journeys,
/// a repeated endpoint and oversize payloads, each at its own key. Also the check for a profile
/// that arrives as JSON (a run request) instead of from `kaanal.yaml`.
///
/// # Errors
/// The first problem, placed under `traffic.NAME`.
pub fn check_profile(name: &str, p: &TrafficConfig) -> Result<(), Located> {
    if crate::valid_name(name).is_err() {
        return Err(Located::at_value(
            keys(&["traffic", name]),
            format!(
                "invalid traffic profile name '{name}': use 1-64 chars of a-z, 0-9, '_' or '-', starting with a letter or digit"
            ),
        ));
    }
    let at = |tail: &[PathSeg]| [keys(&["traffic", name, "journeys"]), tail.to_vec()].concat();
    if p.journeys.is_empty() {
        return Err(Located::at_value(
            at(&[]),
            format!("profile '{name}' needs at least one journey"),
        ));
    }
    if p.journeys.len() > MAX_JOURNEYS {
        return Err(Located::at_value(
            at(&[]),
            format!(
                "profile '{name}' has {} journeys; at most {MAX_JOURNEYS}",
                p.journeys.len()
            ),
        ));
    }
    // the simulator keys streams by the normalized endpoint, so `GET /a` and `get /a` are one
    let norm = |raw: &str| endpoint_key(raw).unwrap_or_else(|_| raw.to_string());
    for (i, j) in p.journeys.iter().enumerate() {
        if let Err(m) = endpoint_key(&j.endpoint) {
            return Err(Located::at_value(
                at(&[PathSeg::Index(i), PathSeg::Key("endpoint".into())]),
                m,
            ));
        }
        if p.journeys[..i]
            .iter()
            .any(|o| norm(&o.endpoint) == norm(&j.endpoint))
        {
            return Err(Located::at_value(
                at(&[PathSeg::Index(i), PathSeg::Key("endpoint".into())]),
                format!(
                    "endpoint '{}' appears twice in profile '{name}'; give it one journey and a bigger weight",
                    j.endpoint
                ),
            ));
        }
        for (field, v) in [("req_bytes", j.req_bytes), ("resp_bytes", j.resp_bytes)] {
            if v.is_some_and(|b| b > MAX_BYTES) {
                return Err(Located::at_value(
                    at(&[PathSeg::Index(i), PathSeg::Key(field.into())]),
                    format!("{field} must be at most {MAX_BYTES} (1 GiB)"),
                ));
            }
        }
    }
    let at_key = |k: &str| keys(&["traffic", name, k]);
    if p.duration.is_some_and(|d| d.is_zero() || d > MAX_DURATION) {
        return Err(Located::at_value(
            at_key("duration"),
            "duration must be above 0 and at most 1h".to_string(),
        ));
    }
    if let (Some(w), Some(d)) = (p.warmup, p.duration)
        && w >= d
    {
        return Err(Located::at_value(
            at_key("warmup"),
            "warmup must be shorter than the duration".to_string(),
        ));
    }
    Ok(())
}
