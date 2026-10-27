//! Shared helpers: the compared configs, `assert_zero_load_equal`, `compare_with_execute` and the build helpers.

use crate::shared::{CHAIN_ROUTES, CHAINS, compiled};

use kaanal_core::{
    ChainRequest, ChaosKey, ChaosLayer, ChaosState, EffectiveChaos, Endpoints, Scenario, Topology,
    execute,
};

use kaanal_sim::{SimConfig, Stream, run, run_effective};

use std::path::Path;

use std::time::Duration;

/// One service with one slot and exponential 10 ms service; everything else costs 0.
pub(crate) const QUEUED: &str = "version: 1\nnodes:\n  gw: { type: gateway }\n  app: { type: service, concurrency: 1, proc: { mean: 10ms } }\nedges:\n  - { from: gw, to: app }\n";

/// The spec routes owned by `app` in the fixture and the sample (same openapi.yaml).
pub(crate) const APP_ROUTES: [(&str, &str); 3] = [
    ("GET /users", "app"),
    ("GET /users/{id}", "app"),
    ("POST /orders", "app"),
];

/// Compiles `GET /users/{id}` on `app` for `yaml`.
pub(crate) fn topo(yaml: &str) -> (Topology, Endpoints) {
    compiled(yaml, &[("GET /users/{id}", "app")])
}

pub(crate) fn one_stream(rate: f64, secs: u64) -> SimConfig {
    SimConfig {
        streams: vec![Stream {
            endpoint_key: "GET /users/{id}".into(),
            rate_per_s: rate,
        }],
        duration: Duration::from_secs(secs),
        warmup: Duration::ZERO,
        seed: 42,
    }
}

pub(crate) fn nanos(d: Duration) -> i128 {
    i128::try_from(d.as_nanos()).unwrap()
}

/// A gateway-less topology (R1.23): requests enter at `app`; `db` costs come from the
/// postgres preset.
pub(crate) const GATEWAYLESS: &str = "version: 1\nseed: 7\nnodes:\n  app: { type: service, proc: { p50: 5ms, p99: 20ms } }\n  db: { type: database, flavor: postgres }\nedges:\n  - { from: app, to: db, net: { p50: 1ms, p99: 2ms } }\n";

/// Branch probabilities (an edge default, a step override, inside `parallel`) for the drift tests.
pub(crate) const PROB: &str = "version: 1\nseed: 13\nnodes:\n  gw: { type: gateway, proc: { mean: 1ms } }\n  app: { type: service, proc: { mean: 2ms } }\n  db: { type: database, flavor: postgres, timeout: 2s }\n  pay: { type: external-api, proc: { p50: 5ms, p99: 30ms } }\nedges:\n  - { from: gw, to: app, net: 1ms }\n  - { from: app, to: db, net: { mean: 1ms }, prob: 0.6 }\n  - { from: app, to: pay, net: 1ms, prob: 0.3 }\nendpoints:\n  \"GET /users/{id}\": { chain: [ { call: db, repeat: 2, prob: 0.5 }, { parallel: [ pay, { call: db, prob: 0.4 } ] }, pay ] }\n";

/// (name, YAML, the `(key, owner)` routes it streams).
pub(crate) type Sample = (&'static str, String, Vec<(&'static str, &'static str)>);

/// The Phase One fixture, the shipped sample, a gateway-less topology and the A2 chains.
pub(crate) fn sample_configs() -> Vec<Sample> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let file = |p: &str| std::fs::read_to_string(root.join(p)).unwrap();
    vec![
        (
            "fixture",
            file("../kaanal-server/tests/fixtures/kaanal.yaml"),
            APP_ROUTES.to_vec(),
        ),
        (
            "example",
            file("../../examples/nginx-server/kaanal.yaml"),
            APP_ROUTES.to_vec(),
        ),
        ("gatewayless", GATEWAYLESS.to_string(), APP_ROUTES.to_vec()),
        ("chains", CHAINS.to_string(), CHAIN_ROUTES.to_vec()),
        ("prob", PROB.to_string(), APP_ROUTES.to_vec()),
    ]
}

/// R7.5 drift guard: streams every `keys` endpoint at 1 req/s for 2000 s and asserts each
/// record equals the independent `execute()` outcome (hops, latency, result, no queue wait).
/// Returns the report and how many failures cut a parallel sibling.
pub(crate) fn assert_zero_load_equal(
    t: &Topology,
    e: &Endpoints,
    chaos: &ChaosState,
    label: &str,
    keys: &[&str],
    seed: u64,
) -> (kaanal_sim::SimReport, usize) {
    let sc = zero_load_config(keys, seed);
    let rep = run(t, e, chaos, &sc).unwrap();
    let cut = compare_with_execute(t, e, &chaos.compile(), &rep, &sc, label);
    (rep, cut)
}

/// [`assert_zero_load_equal`] for chaos compiled elsewhere (a scenario stack), through
/// `run_effective`.
pub(crate) fn assert_effective_equal(
    t: &Topology,
    e: &Endpoints,
    chaos: &EffectiveChaos,
    label: &str,
    keys: &[&str],
    seed: u64,
) -> (kaanal_sim::SimReport, usize) {
    let sc = zero_load_config(keys, seed);
    let rep = run_effective(t, e, chaos, &sc).unwrap();
    let cut = compare_with_execute(t, e, chaos, &rep, &sc, label);
    (rep, cut)
}

pub(crate) fn zero_load_config(keys: &[&str], seed: u64) -> SimConfig {
    SimConfig {
        streams: keys
            .iter()
            .map(|k| Stream {
                endpoint_key: (*k).to_string(),
                rate_per_s: 1.0,
            })
            .collect(),
        duration: Duration::from_secs(2_000),
        warmup: Duration::ZERO,
        seed,
    }
}

pub(crate) fn compare_with_execute(
    t: &Topology,
    e: &Endpoints,
    chaos: &EffectiveChaos,
    rep: &kaanal_sim::SimReport,
    sc: &SimConfig,
    label: &str,
) -> usize {
    let seed = sc.seed;
    let mut cut = 0;
    for r in &rep.requests {
        let key = &sc.streams[r.stream].endpoint_key;
        let live = execute(
            t,
            chaos,
            &ChainRequest {
                endpoint: e.get(key).unwrap(),
                n: r.n,
                seed,
                pin_delay: std::time::Duration::ZERO,
            },
        );
        let at = format!("{label} {key} n={}", r.n);
        let hops: Vec<_> = r.hops.iter().map(|(h, _)| h.clone()).collect();
        assert_eq!(hops, live.hops, "{at}");
        assert!(
            r.hops.iter().all(|(_, w)| w.is_zero()),
            "{at}: no queueing at zero load"
        );
        assert_eq!(r.latency, live.total(), "{at}");
        assert_eq!(r.result, live.result, "{at}");
        cut += usize::from(
            live.fail
                .is_some_and(|f| !live.plan.cancelled_by(f).is_empty()),
        );
    }
    cut
}

pub(crate) const SEED: u64 = 5;

pub(crate) const GW: &str = "type: gateway, proc: 1ms";

pub(crate) const DB: &str = "type: database, ops: { read: 2ms, write: 2ms, scan: 2ms }";

pub(crate) const SLOW_DB: &str = "type: database, ops: { read: 10ms, write: 10ms, scan: 10ms }";

pub(crate) const KEY: &str = "GET /users/{id}";

/// `GET /users/{id}` on `app` behind `nginx` and in front of `db`; `nginx` and `db` are the
/// bodies of their flow maps, so a test sets `timeout:` and costs itself.
pub(crate) fn build_with(nginx: &str, db: &str) -> (Topology, Endpoints) {
    compiled(
        &format!(
            "version: 1\nseed: {SEED}\nnodes:\n  nginx: {{ {nginx} }}\n  app: {{ type: service, proc: 2ms }}\n  db: {{ {db} }}\nedges:\n  - {{ from: nginx, to: app, net: 1ms }}\n  - {{ from: app, to: db, net: 1ms }}\n"
        ),
        &[(KEY, "app")],
    )
}

/// `assert_zero_load_equal` for the single endpoint `KEY`.
pub(crate) fn zero_load(
    t: &Topology,
    e: &Endpoints,
    chaos: &ChaosState,
    label: &str,
) -> kaanal_sim::SimReport {
    assert_zero_load_equal(t, e, chaos, label, &[KEY], SEED).0
}

/// Every reported request, as a `u64` for comparing with counters.
pub(crate) fn all(rep: &kaanal_sim::SimReport) -> u64 {
    rep.requests.len() as u64
}

pub(crate) fn scenario(name: &str, entries: Vec<(ChaosKey, ChaosLayer)>) -> Scenario {
    let mut layers = ChaosState::default();
    for (k, l) in entries {
        layers.set(k, l);
    }
    Scenario {
        name: name.into(),
        description: None,
        layers,
        pins: kaanal_core::PinLayer::default(),
    }
}
