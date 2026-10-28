//! A traffic profile as simulator input: rates from weights, sizes from the profile or from a
//! generated body, a duration and a warmup the simulator accepts.

use std::time::Duration;

use axum::http::StatusCode;
use kaanal_core::{JourneyConfig, TrafficConfig, check_profile, endpoint_key};
use kaanal_sim::{SimConfig, Stream};

use super::sample;
use crate::generation::Generation;

/// Default simulated time of a run.
const DEFAULT_DURATION: Duration = Duration::from_secs(60);
/// Longest simulated time a run may ask for.
const MAX_DURATION: Duration = kaanal_core::config::MAX_DURATION;

/// Which profile a run uses.
pub(crate) enum Profile {
    /// A profile of the config's `traffic:` section.
    Named(String),
    /// A profile sent with the request.
    Inline(TrafficConfig),
}

/// What a run request asks for.
pub(crate) struct RunRequest {
    pub profile: Profile,
    pub duration: Option<Duration>,
    pub warmup: Option<Duration>,
    pub seed: Option<u64>,
    pub scenario: Option<String>,
}

/// Why a request cannot be turned into a run: an HTTP status and a message for the client.
#[derive(Debug)]
pub(crate) struct SimError {
    pub status: StatusCode,
    pub message: String,
}

fn bad(message: impl Into<String>) -> SimError {
    SimError {
        status: StatusCode::BAD_REQUEST,
        message: message.into(),
    }
}

/// Up to five route keys worth suggesting for an unknown `key`: those sharing its first path
/// segment, else the first five.
fn close_matches(g: &Generation, key: &str) -> Vec<String> {
    let seg = key
        .split_once(' ')
        .and_then(|(_, p)| p.trim_start_matches('/').split('/').next())
        .unwrap_or("");
    let near: Vec<String> = g
        .route_keys
        .iter()
        .filter(|k| !seg.is_empty() && k.contains(seg))
        .take(5)
        .cloned()
        .collect();
    if near.is_empty() {
        g.route_keys.iter().take(5).cloned().collect()
    } else {
        near
    }
}

/// A run ready to check: the simulator input, plus the journeys whose `resp_bytes` is still the
/// placeholder 0 because the profile left it out. Sampling a body for each costs real work, so
/// it waits until the cheap checks have passed and runs off the async threads
/// ([`Planned::fill_sizes`]).
#[derive(Debug)]
pub(crate) struct Planned {
    /// The simulator input; omitted response sizes are 0 until filled.
    pub cfg: SimConfig,
    fill: Vec<(usize, String)>,
    seed: u64,
}

impl Planned {
    /// Sets each omitted `resp_bytes` to the size of one generated body of its route.
    pub(crate) fn fill_sizes(&mut self, g: &Generation) {
        for (i, key) in &self.fill {
            self.cfg.streams[*i].resp_bytes = sample::sized(g, key, self.seed) as u64;
        }
    }
}

/// One journey as a stream of `rate` requests per second.
fn stream(g: &Generation, j: &JourneyConfig, rate: f64) -> Result<Stream, SimError> {
    let key = endpoint_key(&j.endpoint).map_err(bad)?;
    if g.endpoints.get(&key).is_none() {
        return Err(bad(format!(
            "unknown endpoint '{key}'; the routes include: {}",
            close_matches(g, &key).join(", ")
        )));
    }
    Ok(Stream {
        resp_bytes: j.resp_bytes.unwrap_or(0),
        req_bytes: j.req_bytes.unwrap_or(0),
        endpoint_key: key,
        rate_per_s: rate,
    })
}

/// The simulator input for `req` on the system `g`.
///
/// # Errors
/// 404 for a profile name the config does not have; 400 for an invalid inline profile, an
/// endpoint that is not a route, or a duration or warmup out of range.
pub(crate) fn to_sim(g: &Generation, req: &RunRequest) -> Result<Planned, SimError> {
    let p = match &req.profile {
        Profile::Named(n) => g.traffic.get(n).ok_or_else(|| SimError {
            status: StatusCode::NOT_FOUND,
            message: format!("no traffic profile named '{n}'"),
        })?,
        Profile::Inline(p) => {
            check_profile("inline", p).map_err(|e| bad(e.msg))?;
            p
        }
    };
    let duration = req.duration.or(p.duration).unwrap_or(DEFAULT_DURATION);
    if duration.is_zero() || duration > MAX_DURATION {
        return Err(bad("duration must be above 0 and at most 1 hour"));
    }
    // the profile's own warmup only stands if it still fits a duration the request shortened
    let warmup = match (req.warmup, p.warmup) {
        (Some(w), _) => w,
        (None, Some(w)) if w < duration => w,
        _ => duration / 10,
    };
    if warmup >= duration {
        return Err(bad("warmup must be shorter than the duration"));
    }
    let seed = req.seed.unwrap_or(g.seed);
    let total: f64 = p.journeys.iter().map(|j| j.weight).sum();
    if !total.is_finite() {
        return Err(bad(
            "the journey weights add up to too much; use smaller weights",
        ));
    }
    let streams = p
        .journeys
        .iter()
        // weight over total first: rps times a huge weight would overflow on its own
        .map(|j| stream(g, j, p.rps * (j.weight / total)))
        .collect::<Result<Vec<_>, _>>()?;
    let fill = p
        .journeys
        .iter()
        .zip(&streams)
        .enumerate()
        .filter(|(_, (j, _))| j.resp_bytes.is_none())
        .map(|(i, (_, s))| (i, s.endpoint_key.clone()))
        .collect();
    Ok(Planned {
        cfg: SimConfig {
            streams,
            duration,
            warmup,
            seed,
        },
        fill,
        seed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::AppState;
    use kaanal_core::Config;
    use std::sync::Arc;

    const YAML: &str = "version: 1
seed: 42
nodes:
  nginx: { type: gateway }
  app: { type: service, spec: openapi.yaml, proc: 1ms }
edges:
  - { from: nginx, to: app }
traffic:
  peak:
    rps: 100
    journeys:
      - { endpoint: 'GET /users', weight: 3 }
      - { endpoint: 'get /users/{id}', weight: 1, resp_bytes: 77 }
";

    fn generation() -> Arc<Generation> {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
        let cfg = Config::from_yaml(YAML).unwrap();
        AppState::build(&cfg, &dir, "t".into(), 0)
            .unwrap()
            .generation()
            .unwrap()
    }

    fn named(n: &str) -> RunRequest {
        RunRequest {
            profile: Profile::Named(n.into()),
            duration: None,
            warmup: None,
            seed: None,
            scenario: None,
        }
    }

    #[test]
    fn weights_become_rates_that_add_up_to_rps() {
        let s = to_sim(&generation(), &named("peak")).unwrap().cfg;
        let rates: Vec<f64> = s.streams.iter().map(|x| x.rate_per_s).collect();
        assert!((rates[0] - 75.0).abs() < 1e-9 && (rates[1] - 25.0).abs() < 1e-9);
        assert_eq!(
            s.streams[1].endpoint_key, "GET /users/{id}",
            "key normalized"
        );
        assert_eq!(s.streams[1].resp_bytes, 77, "an explicit size is kept");
    }

    #[test]
    fn an_unknown_endpoint_is_a_400_that_names_it() {
        let mut r = named("x");
        r.profile = Profile::Inline(
            serde_json::from_value(serde_json::json!({
                "rps": 5, "journeys": [ { "endpoint": "GET /usres" } ]
            }))
            .unwrap(),
        );
        let e = to_sim(&generation(), &r).unwrap_err();
        assert_eq!(e.status, StatusCode::BAD_REQUEST);
        assert!(
            e.message.contains("GET /usres") && e.message.contains("GET /users"),
            "{}",
            e.message
        );
    }

    #[test]
    fn the_request_seed_reaches_the_simulator() {
        let g = generation();
        assert_eq!(
            to_sim(&g, &named("peak")).unwrap().cfg.seed,
            42,
            "the config's"
        );
        let mut r = named("peak");
        r.seed = Some(7);
        assert_eq!(to_sim(&g, &r).unwrap().cfg.seed, 7);
    }

    #[test]
    fn weights_that_overflow_are_a_400_that_names_them() {
        let mut r = named("x");
        r.profile = Profile::Inline(
            serde_json::from_value(serde_json::json!({
                "rps": 5,
                "journeys": [
                    { "endpoint": "GET /users", "weight": 1e308 },
                    { "endpoint": "GET /users/{id}", "weight": 1e308 }
                ]
            }))
            .unwrap(),
        );
        let e = to_sim(&generation(), &r).unwrap_err();
        assert!(e.message.contains("weights"), "{}", e.message);
    }

    #[test]
    fn one_huge_weight_is_a_share_not_an_overflow() {
        let mut r = named("x");
        r.profile = Profile::Inline(
            serde_json::from_value(serde_json::json!({
                "rps": 10,
                "journeys": [ { "endpoint": "GET /users", "weight": 1e308 } ]
            }))
            .unwrap(),
        );
        let s = to_sim(&generation(), &r).unwrap().cfg;
        assert!((s.streams[0].rate_per_s - 10.0).abs() < 1e-9);
    }

    #[test]
    fn a_shortened_duration_drops_a_warmup_that_no_longer_fits() {
        let g = generation();
        let mut r = named("peak");
        r.duration = Some(Duration::from_secs(20));
        r.profile = Profile::Inline(
            serde_json::from_value(serde_json::json!({
                "rps": 5, "warmup": "30s", "journeys": [ { "endpoint": "GET /users" } ]
            }))
            .unwrap(),
        );
        let s = to_sim(&g, &r).unwrap().cfg;
        assert_eq!(s.warmup, Duration::from_secs(2), "a tenth of 20s");
    }

    #[test]
    fn a_head_is_charged_no_body_as_the_mock_does() {
        let g = generation();
        assert!(sample::sized(&g, "GET /users", 42) > 2);
        assert_eq!(sample::sized(&g, "HEAD /users", 42), 0);
    }

    #[test]
    fn a_journey_that_names_no_route_is_a_warning_at_load() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
        let yaml = YAML.replace("'get /users/{id}'", "'GET /usres'");
        let cfg = Config::from_yaml(&yaml).unwrap();
        let g = AppState::build(&cfg, &dir, "t".into(), 0)
            .unwrap()
            .generation()
            .unwrap();
        assert!(
            g.warnings
                .iter()
                .any(|w| w.contains("GET /usres") && w.contains("not a route")),
            "{:?}",
            g.warnings
        );
    }

    #[test]
    fn a_named_profile_that_does_not_exist_is_404() {
        let e = to_sim(&generation(), &named("nope")).unwrap_err();
        assert_eq!(e.status, StatusCode::NOT_FOUND);
    }

    #[test]
    fn omitted_response_bytes_come_from_a_generated_body() {
        let g = generation();
        let mut planned = to_sim(&g, &named("peak")).unwrap();
        assert_eq!(
            planned.cfg.streams[0].resp_bytes, 0,
            "not sampled until asked"
        );
        planned.fill_sizes(&g);
        let s = planned.cfg;
        let body = sample::sample(&g, "GET /users", None).unwrap();
        assert!(body.bytes > 2, "the fixture's list route generates a body");
        assert_eq!(s.streams[0].resp_bytes, body.bytes as u64);
    }

    #[test]
    fn warmup_defaults_to_a_tenth_and_duration_is_capped_at_an_hour() {
        let g = generation();
        let s = to_sim(&g, &named("peak")).unwrap().cfg;
        assert_eq!(
            (s.duration, s.warmup),
            (Duration::from_secs(60), Duration::from_secs(6))
        );
        let mut r = named("peak");
        r.duration = Some(Duration::from_secs(3601));
        assert_eq!(to_sim(&g, &r).unwrap_err().status, StatusCode::BAD_REQUEST);
        r.duration = Some(Duration::from_secs(10));
        r.warmup = Some(Duration::from_secs(10));
        assert!(to_sim(&g, &r).unwrap_err().message.contains("warmup"));
    }
}
