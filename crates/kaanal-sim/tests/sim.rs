mod common;

use common::{CHAIN_ROUTES, CHAINS, compiled};
use kaanal_core::{
    ChainRequest, ChainResult, ChaosKey, ChaosLayer, ChaosStack, ChaosState, Config,
    EffectiveChaos, Endpoints, ErrorKind, FailKind, HopKind, NodeChaos, Scenario, ScenarioSet,
    Source, Topology, execute,
};
use kaanal_sim::{SimConfig, Stream, run, run_effective};
use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

/// One service with one slot and exponential 10 ms service; everything else costs 0.
const QUEUED: &str = "version: 1\nnodes:\n  gw: { type: gateway }\n  app: { type: service, concurrency: 1, proc: { mean: 10ms } }\nedges:\n  - { from: gw, to: app }\n";

/// The spec routes owned by `app` in the fixture and the sample (same openapi.yaml).
const APP_ROUTES: [(&str, &str); 3] = [
    ("GET /users", "app"),
    ("GET /users/{id}", "app"),
    ("POST /orders", "app"),
];

/// Compiles `GET /users/{id}` on `app` for `yaml`.
fn topo(yaml: &str) -> (Topology, Endpoints) {
    compiled(yaml, &[("GET /users/{id}", "app")])
}

fn one_stream(rate: f64, secs: u64) -> SimConfig {
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

fn nanos(d: Duration) -> i128 {
    i128::try_from(d.as_nanos()).unwrap()
}

/// A gateway-less topology (R1.23): requests enter at `app`; `db` costs come from the
/// postgres preset.
const GATEWAYLESS: &str = "version: 1\nseed: 7\nnodes:\n  app: { type: service, proc: { p50: 5ms, p99: 20ms } }\n  db: { type: database, flavor: postgres }\nedges:\n  - { from: app, to: db, net: { p50: 1ms, p99: 2ms } }\n";

/// (name, YAML, the `(key, owner)` routes it streams).
type Sample = (&'static str, String, Vec<(&'static str, &'static str)>);

/// The Phase One fixture, the shipped sample, a gateway-less topology and the A2 chains.
fn sample_configs() -> Vec<Sample> {
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
    ]
}

/// R7.5 drift guard: streams every `keys` endpoint at 1 req/s for 2000 s and asserts each
/// record equals the independent `execute()` outcome (hops, latency, result, no queue wait).
/// Returns the report and how many failures cut a parallel sibling.
fn assert_zero_load_equal(
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
fn assert_effective_equal(
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

fn zero_load_config(keys: &[&str], seed: u64) -> SimConfig {
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

fn compare_with_execute(
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

const SEED: u64 = 5;
const GW: &str = "type: gateway, proc: 1ms";
const DB: &str = "type: database, ops: { read: 2ms, write: 2ms, scan: 2ms }";
const SLOW_DB: &str = "type: database, ops: { read: 10ms, write: 10ms, scan: 10ms }";
const KEY: &str = "GET /users/{id}";

/// `GET /users/{id}` on `app` behind `nginx` and in front of `db`; `nginx` and `db` are the
/// bodies of their flow maps, so a test sets `timeout:` and costs itself.
fn build_with(nginx: &str, db: &str) -> (Topology, Endpoints) {
    compiled(
        &format!(
            "version: 1\nseed: {SEED}\nnodes:\n  nginx: {{ {nginx} }}\n  app: {{ type: service, proc: 2ms }}\n  db: {{ {db} }}\nedges:\n  - {{ from: nginx, to: app, net: 1ms }}\n  - {{ from: app, to: db, net: 1ms }}\n"
        ),
        &[(KEY, "app")],
    )
}

/// `assert_zero_load_equal` for the single endpoint `KEY`.
fn zero_load(
    t: &Topology,
    e: &Endpoints,
    chaos: &ChaosState,
    label: &str,
) -> kaanal_sim::SimReport {
    assert_zero_load_equal(t, e, chaos, label, &[KEY], SEED).0
}

/// Every reported request, as a `u64` for comparing with counters.
fn all(rep: &kaanal_sim::SimReport) -> u64 {
    rep.requests.len() as u64
}

#[test]
fn sim_reports_timeouts_per_node_and_errors_by_kind() {
    // gateway deadline 2ms in front of a db whose op takes 10ms: every request times out
    let (t, e) = build_with("type: gateway, proc: 1ms, timeout: 2ms", SLOW_DB);
    let rep = run(&t, &e, &ChaosState::default(), &one_stream(50.0, 200)).unwrap();
    let nginx = t.id("nginx").unwrap();
    assert!(rep.requests.len() > 5_000);
    assert_eq!(rep.nodes[nginx].timed_out, rep.requests.len() as u64);
    assert_eq!(
        rep.errors_by_kind(),
        BTreeMap::from([("timeout", rep.requests.len() as u64)])
    );
}

#[test]
fn zero_load_equals_execute_for_every_kind() {
    // R7.5: at zero load the sim record equals the independent execute() outcome.
    let db_timeout = format!("{DB}, timeout: 3s");
    // (chaos kind, label, db body, error kind, whether a span deadline fires at db). A hang
    // that ends through its planned Timeout `Fail` (no span: the default cap) is not an
    // engine deadline, so db.timed_out stays 0 although errors_by_kind says `timeout`.
    // With a node timeout the db span's deadline (3 s, from the edge start) comes 1 ms before
    // the hang hop's end (edge 1 ms + 3 s cap), so the deadline fires and is counted at db.
    for (kind, name, db, want, deadline) in [
        (ErrorKind::Status, "status", DB, "status", false),
        (ErrorKind::Reset, "reset", DB, "reset", false),
        (ErrorKind::Malformed, "malformed", DB, "malformed", false),
        (ErrorKind::Hang, "hang default cap", DB, "timeout", false),
        (
            ErrorKind::Hang,
            "hang node timeout",
            &*db_timeout,
            "timeout",
            true,
        ),
    ] {
        let (t, e) = build_with(GW, db);
        let chaos = ChaosState::from([(
            t.id("db").unwrap(),
            NodeChaos {
                error_rate: 1.0,
                error_kind: kind,
                ..NodeChaos::default()
            },
        )]);
        let rep = zero_load(&t, &e, &chaos, name);
        assert_eq!(
            rep.errors_by_kind(),
            BTreeMap::from([(want, all(&rep))]),
            "{name}: every request must fail with that kind"
        );
        let timed_out = rep.nodes[t.id("db").unwrap()].timed_out;
        assert_eq!(timed_out, if deadline { all(&rep) } else { 0 }, "{name}");
    }
}

#[test]
fn zero_load_equals_execute_with_spans() {
    // db's 3 ms deadline covers its 1 ms edge and a 10 ms op: it really times out at zero load.
    let (t, e) = build_with(GW, &format!("{SLOW_DB}, timeout: 3ms"));
    let rep = zero_load(&t, &e, &ChaosState::default(), "db span");
    assert_eq!(
        rep.errors_by_kind(),
        BTreeMap::from([("timeout", all(&rep))])
    );
    assert_eq!(rep.nodes[t.id("db").unwrap()].timed_out, all(&rep));
}

fn hang() -> NodeChaos {
    NodeChaos {
        error_rate: 1.0,
        error_kind: ErrorKind::Hang,
        ..NodeChaos::default()
    }
}

#[test]
fn zero_load_equals_execute_for_a_hang_under_spans() {
    // (a) db span 3 ms (from the edge start) vs a hang hop capped at the same 3 ms that
    // starts after the 1 ms edge: the deadline fires first, 1 ms before the planned Fail (Z6).
    let (t, e) = build_with(GW, &format!("{DB}, timeout: 3ms"));
    let chaos = ChaosState::from([(t.id("db").unwrap(), hang())]);
    let rep = zero_load(&t, &e, &chaos, "hang under db span");
    assert_eq!(
        rep.errors_by_kind(),
        BTreeMap::from([("timeout", all(&rep))])
    );
    assert_eq!(rep.nodes[t.id("db").unwrap()].timed_out, all(&rep));

    // The hanging db has no timeout (60 s cap); the gateway's root span (1 s) fires first.
    let (t, e) = build_with("type: gateway, proc: 1ms, timeout: 1s", DB);
    let chaos = ChaosState::from([(t.id("db").unwrap(), hang())]);
    let rep = zero_load(&t, &e, &chaos, "hang under gateway span");
    assert_eq!(
        rep.errors_by_kind(),
        BTreeMap::from([("timeout", all(&rep))])
    );
    assert_eq!(rep.nodes[t.id("nginx").unwrap()].timed_out, all(&rep));
    assert_eq!(rep.nodes[t.id("db").unwrap()].timed_out, 0);
}

#[test]
fn zero_load_equals_execute_for_a_deadline_in_a_parallel_branch() {
    // (b) db's 2 ms span fires inside one branch while the other (`slow`, 50 ms) is cut.
    let (t, e) = compiled(
        &format!(
            "version: 1\nseed: {SEED}\nnodes:\n  nginx: {{ {GW} }}\n  app: {{ type: service, proc: 2ms }}\n  db: {{ {SLOW_DB}, timeout: 2ms }}\n  slow: {{ type: external-api, proc: 50ms }}\nedges:\n  - {{ from: nginx, to: app, net: 1ms }}\n  - {{ from: app, to: db, net: 1ms }}\n  - {{ from: app, to: slow, net: 1ms }}\nendpoints:\n  \"GET /x\": {{ chain: [ {{ parallel: [ db, slow ] }} ] }}\n"
        ),
        &[("GET /x", "app")],
    );
    let (rep, cut) = assert_zero_load_equal(
        &t,
        &e,
        &ChaosState::default(),
        "parallel",
        &["GET /x"],
        SEED,
    );
    assert!(cut > 0, "the deadline must cancel the sibling branch");
    assert_eq!(
        rep.errors_by_kind(),
        BTreeMap::from([("timeout", all(&rep))])
    );
    assert_eq!(rep.nodes[t.id("db").unwrap()].timed_out, all(&rep));
    assert_eq!(rep.nodes[t.id("slow").unwrap()].timed_out, 0);
}

#[test]
fn zero_load_equals_execute_for_a_hang_in_a_parallel_branch() {
    // A hang in one branch (60 s cap, planned `Fail` at the hang hop's end) while the sibling
    // (`slow`, 50 ms) finishes long before: the failure ends the block at the hang's end.
    let (t, e) = compiled(
        &format!(
            "version: 1\nseed: {SEED}\nnodes:\n  nginx: {{ {GW} }}\n  app: {{ type: service, proc: 2ms }}\n  db: {{ {DB} }}\n  slow: {{ type: external-api, proc: 50ms }}\nedges:\n  - {{ from: nginx, to: app, net: 1ms }}\n  - {{ from: app, to: db, net: 1ms }}\n  - {{ from: app, to: slow, net: 1ms }}\nendpoints:\n  \"GET /x\": {{ chain: [ {{ parallel: [ db, slow ] }} ] }}\n"
        ),
        &[("GET /x", "app")],
    );
    let chaos = ChaosState::from([(t.id("db").unwrap(), hang())]);
    let (rep, _) = assert_zero_load_equal(&t, &e, &chaos, "hang in parallel", &["GET /x"], SEED);
    assert_eq!(
        rep.errors_by_kind(),
        BTreeMap::from([("timeout", all(&rep))])
    );
    assert_eq!(
        rep.nodes[t.id("db").unwrap()].timed_out,
        0,
        "no deadline fired, the hang ran to its Fail"
    );
}

#[test]
fn zero_load_equals_execute_for_endpoint_chaos() {
    // (c) endpoint-level chaos: a 502 at the front door, a Delay hop, and a node chaos that
    // applies only through this endpoint.
    let (t, e) = build_with(GW, DB);
    let db = t.id("db").unwrap();
    let with = |key: ChaosKey, layer: ChaosLayer| {
        let mut c = ChaosState::default();
        c.set(key, layer);
        c
    };
    let front = || ChaosKey::Endpoint(KEY.into());
    let fail = with(
        front(),
        ChaosLayer {
            error_rate: Some(1.0),
            error_status: Some(502),
            ..ChaosLayer::default()
        },
    );
    let rep = zero_load(&t, &e, &fail, "endpoint 502");
    assert_eq!(
        rep.errors_by_kind(),
        BTreeMap::from([("status", all(&rep))])
    );
    assert!(
        rep.requests
            .iter()
            .all(|r| matches!(r.result, ChainResult::Err { status: 502, .. }))
    );

    let slow = with(
        front(),
        ChaosLayer {
            latency_add: Some(Duration::from_millis(20)),
            ..ChaosLayer::default()
        },
    );
    let rep = zero_load(&t, &e, &slow, "endpoint latency");
    assert!(rep.errors_by_kind().is_empty());
    assert!(
        rep.requests
            .iter()
            .all(|r| r.hops.iter().any(|(h, _)| h.kind == HopKind::Delay))
    );

    let at = with(
        ChaosKey::NodeAt(db, KEY.into()),
        ChaosLayer {
            error_rate: Some(1.0),
            ..ChaosLayer::default()
        },
    );
    let rep = zero_load(&t, &e, &at, "node-at db");
    assert_eq!(
        rep.errors_by_kind(),
        BTreeMap::from([("status", all(&rep))])
    );
}

#[test]
fn zero_load_equals_live_exactly() {
    for (name, yaml, routes) in sample_configs() {
        let seed = Config::from_yaml(&yaml).unwrap().seed;
        let (t, e) = compiled(&yaml, &routes);
        // Failing databases, external APIs and caches (a failing cache is a miss, E9).
        let flaky: ChaosState = ["db", "pay", "c"]
            .iter()
            .filter_map(|n| t.id(n))
            .map(|id| {
                (
                    id,
                    NodeChaos {
                        error_rate: 0.5,
                        ..NodeChaos::default()
                    },
                )
            })
            .collect();
        for chaos in [ChaosState::default(), flaky] {
            let keys: Vec<&str> = routes.iter().map(|(k, _)| *k).collect();
            let (rep, cut) = assert_zero_load_equal(&t, &e, &chaos, name, &keys, seed);
            assert!(rep.requests.len() > 5_000, "{name}: {}", rep.requests.len());
            if name == "chains" && !chaos.is_empty() {
                assert!(
                    cut > 100,
                    "failures inside parallel must be exercised: {cut}"
                );
            }
        }
    }
}

fn layer(rate: f64, kind: ErrorKind, status: u16) -> ChaosLayer {
    ChaosLayer {
        error_rate: Some(rate),
        error_kind: Some(kind),
        error_status: Some(status),
        ..ChaosLayer::default()
    }
}

fn scenario(name: &str, entries: Vec<(ChaosKey, ChaosLayer)>) -> Scenario {
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

/// Two scenarios plus a live override, compiled; asserts sim == execute at zero load.
fn stack_drift(
    label: &str,
    s1: Vec<(ChaosKey, ChaosLayer)>,
    s2: Vec<(ChaosKey, ChaosLayer)>,
    live: Vec<(ChaosKey, ChaosLayer)>,
) -> kaanal_sim::SimReport {
    let (t, e) = build_with(GW, DB);
    let mut set = ScenarioSet::default();
    set.insert(scenario("s1", s1), Source::Runtime).unwrap();
    set.insert(scenario("s2", s2), Source::Runtime).unwrap();
    let mut stack = ChaosStack::default();
    stack.activate(&set, "s1").unwrap();
    stack.activate(&set, "s2").unwrap();
    for (k, l) in live {
        stack.live_mut().set(k, l);
    }
    let eff = stack.compile(&set);
    assert!(!eff.is_empty(), "{label}: the stack must compile to chaos");
    assert_effective_equal(&t, &e, &eff, label, &[KEY], SEED).0
}

#[test]
fn zero_load_equals_execute_for_a_scenario_stack() {
    let (t, _) = build_with(GW, DB);
    let (db, app) = (t.id("db").unwrap(), t.id("app").unwrap());
    // A live latency bump rides on top of every case.
    let live = || {
        vec![(
            ChaosKey::Node(t.id("nginx").unwrap()),
            ChaosLayer {
                latency_add: Some(Duration::from_millis(3)),
                ..ChaosLayer::default()
            },
        )]
    };
    for (kind, name) in [
        (ErrorKind::Status, "status"),
        (ErrorKind::Reset, "reset"),
        (ErrorKind::Hang, "hang"),
        (ErrorKind::Malformed, "malformed"),
    ] {
        let rep = stack_drift(
            name,
            vec![(ChaosKey::Node(db), layer(0.5, kind, 502))],
            vec![(ChaosKey::Node(app), layer(0.2, ErrorKind::Status, 503))],
            live(),
        );
        let want = if name == "hang" { "timeout" } else { name };
        assert!(
            rep.errors_by_kind().contains_key(want),
            "{name}: its fault must fire: {:?}",
            rep.errors_by_kind()
        );
    }

    // Banded mix: 30% reset under 30% 503 on one node (about 51% total, two kinds).
    let rep = stack_drift(
        "banded",
        vec![(ChaosKey::Node(db), layer(0.3, ErrorKind::Status, 503))],
        vec![(ChaosKey::Node(db), layer(0.3, ErrorKind::Reset, 503))],
        live(),
    );
    let by = rep.errors_by_kind();
    assert!(
        by.contains_key("reset") && by.contains_key("status"),
        "{by:?}"
    );

    // node@endpoint pair.
    let rep = stack_drift(
        "node-at",
        vec![(
            ChaosKey::NodeAt(db, KEY.into()),
            layer(0.4, ErrorKind::Reset, 503),
        )],
        vec![(ChaosKey::Node(db), layer(0.2, ErrorKind::Status, 503))],
        live(),
    );
    assert!(rep.errors_by_kind().contains_key("reset"));

    // endpoint-level entry with latency.
    let front = ChaosLayer {
        latency_add: Some(Duration::from_millis(20)),
        ..layer(0.3, ErrorKind::Status, 502)
    };
    let rep = stack_drift(
        "endpoint",
        vec![(ChaosKey::Endpoint(KEY.into()), front)],
        vec![(ChaosKey::Node(db), layer(0.2, ErrorKind::Malformed, 503))],
        live(),
    );
    assert!(rep.errors_by_kind().contains_key("status"));

    // A live override calms a scenario: nothing fails, and the sim still agrees with execute.
    let calm = ChaosLayer {
        replace: Some(true),
        ..ChaosLayer::default()
    };
    let rep = stack_drift(
        "calmed",
        vec![(ChaosKey::Node(db), layer(1.0, ErrorKind::Reset, 503))],
        vec![(ChaosKey::Node(db), layer(1.0, ErrorKind::Status, 503))],
        vec![(ChaosKey::Node(db), calm)],
    );
    assert!(rep.errors_by_kind().is_empty());
}

/// Asserts every request's queue wait at `app` equals the Lindley recursion, exactly.
fn assert_lindley(t: &Topology, rep: &kaanal_sim::SimReport) {
    let app = t.id("app").unwrap();
    let (mut prev_a, mut prev_s, mut prev_w) = (0i128, 0i128, 0i128);
    for (k, r) in rep.requests.iter().enumerate() {
        let (hop, wait) = r
            .hops
            .iter()
            .find(|(h, _)| h.node == app && h.kind == HopKind::Proc)
            .unwrap();
        let a = nanos(r.arrival);
        // Lindley: W(k+1) = max(0, W(k) + S(k) − (A(k+1) − A(k))), W(0) = 0.
        let expected = if k == 0 {
            0
        } else {
            (prev_w + prev_s - (a - prev_a)).max(0)
        };
        assert_eq!(nanos(*wait), expected, "request {k}");
        (prev_a, prev_s, prev_w) = (a, nanos(hop.dur), expected);
    }
}

#[test]
fn engine_matches_lindley_recursion() {
    let (t, e) = topo(QUEUED);
    let rep = run(&t, &e, &ChaosState::default(), &one_stream(80.0, 125)).unwrap();
    assert_lindley(&t, &rep);
    assert!(rep.requests.len() > 9_000, "{}", rep.requests.len());
    let queued = rep
        .requests
        .iter()
        .filter(|r| r.hops.iter().any(|(_, w)| !w.is_zero()))
        .count();
    assert!(queued > 1_000, "the test must really queue: {queued}");
}

#[test]
fn simultaneous_events_keep_fifo_order() {
    // 1e6 req/s: the mean gap is 1 µs, so ~0.05% of gaps round to 0 ns and arrivals tie with
    // hop ends and with each other's timestamps. This pins FIFO queue order under equal times. The 800 ns fixed service keeps the queue busy (ρ = 0.8).
    let (t, e) = topo(&QUEUED.replace("{ mean: 10ms }", "800ns"));
    let rep = run(&t, &e, &ChaosState::default(), &one_stream(1e6, 1)).unwrap();
    let ties = rep
        .requests
        .windows(2)
        .filter(|w| w[0].arrival == w[1].arrival)
        .count();
    assert!(ties > 100, "the test must really produce ties: {ties}");
    assert_lindley(&t, &rep);
}

#[test]
fn same_seed_same_report() {
    // Determinism only. FIFO queue order under equal timestamps is pinned by
    // `simultaneous_events_keep_fifo_order`; the (time, seq) rule itself is reasoned, not tested.
    let (t, e) = topo(&QUEUED.replace("{ mean: 10ms }", "10ms"));
    let cfg = one_stream(90.0, 60);
    let a = run(&t, &e, &ChaosState::default(), &cfg).unwrap();
    assert_eq!(a, run(&t, &e, &ChaosState::default(), &cfg).unwrap());
    let b = run(
        &t,
        &e,
        &ChaosState::default(),
        &SimConfig { seed: 43, ..cfg },
    )
    .unwrap();
    assert_ne!(a, b, "a different seed gives a different run");
}

#[test]
fn down_gateway_completes_with_no_hops() {
    let (t, e) = topo(QUEUED);
    let mut down = ChaosState::default();
    down.insert(
        t.gateway.unwrap(),
        NodeChaos {
            down: true,
            ..NodeChaos::default()
        },
    );
    let rep = run(&t, &e, &down, &one_stream(10.0, 10)).unwrap();
    assert_ne!(rep.requests.len(), 0, "the stream must produce requests");
    for r in &rep.requests {
        assert!(r.hops.is_empty() && r.latency.is_zero(), "{r:?}");
        assert!(matches!(r.result, ChainResult::Err { status: 503, .. }));
    }
}

#[test]
fn overload_grows_the_queue_but_finishes() {
    let (t, e) = topo(QUEUED);
    // ρ = 1.5: arrivals outpace the single slot.
    let rep = run(&t, &e, &ChaosState::default(), &one_stream(150.0, 20)).unwrap();
    let app = &rep.nodes[t.id("app").unwrap()];
    assert!(app.max_queue > 500, "{app:?}");
    assert!(
        (0.95..=1.0).contains(&app.utilization),
        "busy time is clipped to the window: {app:?}"
    );
    assert_eq!(app.completed, rep.requests.len() as u64);
}

#[test]
fn unlimited_nodes_report_zero_utilization_and_no_queue() {
    let (t, e) = topo(QUEUED);
    let rep = run(&t, &e, &ChaosState::default(), &one_stream(50.0, 20)).unwrap();
    let gw = &rep.nodes[t.gateway.unwrap()];
    assert_eq!(
        (gw.utilization, gw.max_queue, gw.mean_wait),
        (0.0, 0, Duration::ZERO)
    );
    assert_eq!(gw.completed, rep.requests.len() as u64);
}

#[test]
fn warmup_requests_are_not_reported() {
    let (t, e) = topo(QUEUED);
    let mut cfg = one_stream(50.0, 20);
    cfg.warmup = Duration::from_secs(10);
    let rep = run(&t, &e, &ChaosState::default(), &cfg).unwrap();
    assert!(rep.requests.iter().all(|r| r.arrival >= cfg.warmup));
    assert!(
        rep.requests.first().unwrap().n > 0,
        "counters keep running through warmup"
    );
}

#[test]
fn rejects_bad_config() {
    let (t, e) = topo(QUEUED);
    let ok = || one_stream(10.0, 10);
    let cases: Vec<(SimConfig, &str)> = vec![
        (
            {
                let mut c = ok();
                c.streams.clear();
                c
            },
            "no streams",
        ),
        (
            {
                let mut c = ok();
                c.streams[0].rate_per_s = 0.0;
                c
            },
            "rate_per_s",
        ),
        (
            {
                let mut c = ok();
                c.streams[0].rate_per_s = f64::NAN;
                c
            },
            "rate_per_s",
        ),
        (
            {
                let mut c = ok();
                c.streams[0].rate_per_s = 1e7;
                c
            },
            "rate_per_s",
        ),
        (
            {
                let mut c = ok();
                c.streams[0].rate_per_s = 1e-7;
                c
            },
            "rate_per_s",
        ),
        (
            {
                let mut c = ok();
                c.streams[0].endpoint_key = "GET /nope".into();
                c
            },
            "unknown endpoint",
        ),
        (
            {
                let mut c = ok();
                c.duration = Duration::ZERO;
                c
            },
            "duration",
        ),
        (
            {
                let mut c = ok();
                c.warmup = c.duration;
                c
            },
            "warmup",
        ),
        (
            {
                let mut c = ok();
                c.duration = Duration::from_hours(400 * 24);
                c
            },
            "365 days",
        ),
        (
            {
                let mut c = ok();
                let s = c.streams[0].clone();
                c.streams.push(s);
                c
            },
            "duplicate",
        ),
        (
            {
                let mut c = ok();
                c.streams[0].rate_per_s = 100_000.0;
                c.duration = Duration::from_secs(100);
                c
            },
            "workload too large",
        ),
    ];
    for (cfg, want) in cases {
        let err = run(&t, &e, &ChaosState::default(), &cfg).unwrap_err();
        assert!(err.contains(want), "want '{want}' in: {err}");
    }
}

#[test]
fn request_counters_start_at_zero_and_run_on_consecutively() {
    // Live mode's per-endpoint counter starts at 0; the zero-load test replays each record's own `n`,
    // so this is what pins the sim to live's numbering.
    let (t, e) = topo(QUEUED);
    let rep = run(&t, &e, &ChaosState::default(), &one_stream(50.0, 20)).unwrap();
    let ns: Vec<u64> = rep.requests.iter().map(|r| r.n).collect();
    assert_eq!(ns, (0..ns.len() as u64).collect::<Vec<_>>());
}

#[test]
fn utilization_divides_by_the_slot_count() {
    // 2 slots, 10 ms exponential service, 100 req/s: 1.0 busy slot on average, so 0.5 per slot.
    let (t, e) = topo(&QUEUED.replace("concurrency: 1", "concurrency: 2"));
    let mut cfg = one_stream(100.0, 200);
    cfg.warmup = Duration::from_secs(10);
    let rep = run(&t, &e, &ChaosState::default(), &cfg).unwrap();
    let u = rep.nodes[t.id("app").unwrap()].utilization;
    assert!((u - 0.5).abs() < 0.03, "utilization {u}");
}

#[test]
fn hand_edited_absurd_service_time_is_rejected_not_a_panic() {
    let (mut t, e) = topo(QUEUED);
    let app = t.id("app").unwrap();
    t.nodes[app].proc = kaanal_core::Dist::Fixed(Duration::MAX);
    let err = run(&t, &e, &ChaosState::default(), &one_stream(1.0, 10)).unwrap_err();
    assert!(err.contains("node 'app'") && err.contains("10h"), "{err}");
    t.nodes[app].proc = kaanal_core::Dist::Exp {
        mean: Duration::MAX,
    };
    assert!(run(&t, &e, &ChaosState::default(), &one_stream(1.0, 10)).is_err());
}

#[test]
fn clock_overflow_is_an_error_not_a_silently_wrong_report() {
    // 9 h per request through one slot: 600k arrivals queue up to ~5.4e6 h = 6e2 years > u64 ns.
    let (mut t, e) = topo(QUEUED);
    let app = t.id("app").unwrap();
    t.nodes[app].proc = kaanal_core::Dist::Fixed(Duration::from_hours(9));
    let err = run(&t, &e, &ChaosState::default(), &one_stream(1000.0, 600)).unwrap_err();
    assert!(err.contains("clock overflow"), "{err}");
}

#[test]
fn absurd_chaos_latency_is_rejected_not_a_panic() {
    // The live limits (R6.9): latency_add over 5 min, or a non-5xx status, is an error.
    let (t, e) = topo(QUEUED);
    let app = t.id("app").unwrap();
    let with = |c: NodeChaos| ChaosState::from([(app, c)]);
    let cap = NodeChaos {
        latency_add: Duration::from_secs(300),
        ..NodeChaos::default()
    };
    assert!(run(&t, &e, &with(cap.clone()), &one_stream(1.0, 10)).is_ok());
    let over = NodeChaos {
        latency_add: Duration::from_secs(300) + Duration::from_millis(1),
        ..NodeChaos::default()
    };
    let err = run(&t, &e, &with(over), &one_stream(1.0, 10)).unwrap_err();
    assert!(err.contains("latency_add"), "{err}");
    let teapot = NodeChaos {
        error_status: 418,
        ..cap
    };
    let err = run(&t, &e, &with(teapot), &one_stream(1.0, 10)).unwrap_err();
    assert!(err.contains("error_status"), "{err}");
}

#[test]
fn streams_name_compiled_endpoints() {
    // D15: any compiled endpoint may be a stream, a static one included.
    let (t, e) = compiled(
        "version: 1\nnodes:\n  gw: { type: gateway, proc: 1ms }\n  shop: { type: monolith, proc: 5ms }\nedges:\n  - { from: gw, to: shop }\nendpoints:\n  \"GET /health\": { override: { body: 1 } }\n",
        &[("GET /shop/{id}", "shop")],
    );
    let stream = |key: &str| SimConfig {
        streams: vec![Stream {
            endpoint_key: key.into(),
            rate_per_s: 10.0,
        }],
        duration: Duration::from_secs(10),
        warmup: Duration::ZERO,
        seed: 1,
    };
    let none = ChaosState::default();
    let rep = run(&t, &e, &none, &stream("GET /shop/{id}")).unwrap();
    assert_ne!(rep.requests.len(), 0);
    assert!(
        rep.requests
            .iter()
            .all(|r| r.latency == Duration::from_millis(6)),
        "gw 1 ms + shop 5 ms"
    );
    let rep = run(&t, &e, &none, &stream("GET /health")).unwrap();
    assert!(
        rep.requests
            .iter()
            .all(|r| r.hops.len() == 1 && r.latency == Duration::from_millis(1))
    );
    let err = run(&t, &e, &none, &stream("GET /nope")).unwrap_err();
    assert_eq!(err, "stream 0: unknown endpoint 'GET /nope'");
}

#[test]
fn hop_budget_counts_chain_length() {
    // R6.13: app.proc + 100 × (net + svc.proc + 100 × (net + db op)) = 20 201 hops per request.
    let (t, e) = compiled(
        "version: 1\nnodes:\n  app: { type: service }\n  svc: { type: service }\n  db: { type: database, flavor: postgres }\nedges:\n  - { from: app, to: svc }\n  - { from: svc, to: db }\nendpoints:\n  \"GET /x\": { chain: [ { call: svc, repeat: 100, chain: [ { call: db, repeat: 100 } ] } ] }\n",
        &[("GET /x", "app")],
    );
    assert_eq!(e.get("GET /x").unwrap().chain.hops, 20_201);
    let cfg = SimConfig {
        streams: vec![Stream {
            endpoint_key: "GET /x".into(),
            rate_per_s: 1.0,
        }],
        duration: Duration::from_secs(1_000),
        warmup: Duration::ZERO,
        seed: 1,
    };
    let err = run(&t, &e, &ChaosState::default(), &cfg).unwrap_err();
    assert_eq!(
        err,
        "workload too large: about 1000 arrivals with up to 20201000 hops in total (max 20000000); lower the rate, the duration or the chain length"
    );
}

#[test]
fn hop_budget_sums_streams_and_skips_warmup() {
    // The record budget counts reported arrivals; a second cap counts all simulated hops.
    // Two 20 201-hop endpoints. Each stream alone is under 20 M records at 0.6 req/s, the sum
    // is not; and a warmup shrinks the count to the reported arrivals only.
    let chain = "chain: [ { call: svc, repeat: 100, chain: [ { call: db, repeat: 100 } ] } ]";
    let (t, e) = compiled(
        &format!(
            "version: 1\nnodes:\n  app: {{ type: service }}\n  svc: {{ type: service }}\n  db: {{ type: database, flavor: postgres }}\nedges:\n  - {{ from: app, to: svc }}\n  - {{ from: svc, to: db }}\nendpoints:\n  \"GET /x\": {{ {chain} }}\n  \"GET /y\": {{ {chain} }}\n"
        ),
        &[("GET /x", "app"), ("GET /y", "app")],
    );
    let stream = |key: &str, rate_per_s| Stream {
        endpoint_key: key.into(),
        rate_per_s,
    };
    let cfg = |streams, warmup| SimConfig {
        streams,
        duration: Duration::from_secs(1_000),
        warmup: Duration::from_secs(warmup),
        seed: 1,
    };
    let none = ChaosState::default();
    let err = run(
        &t,
        &e,
        &none,
        &cfg(vec![stream("GET /x", 0.6), stream("GET /y", 0.6)], 0),
    )
    .unwrap_err();
    assert!(
        err.contains("about 1200 arrivals with up to 24241200 hops"),
        "{err}"
    );
    let err = run(&t, &e, &none, &cfg(vec![stream("GET /x", 1.2)], 100)).unwrap_err();
    assert!(
        err.contains("about 1080 arrivals with up to 21817080 hops"),
        "only post-warmup arrivals count: {err}"
    );
    // A long warmup keeps the records small but not the CPU time: warmup arrivals count
    // against the total-work cap (30 reported arrivals, 3 000 simulated).
    let err = run(&t, &e, &none, &cfg(vec![stream("GET /x", 3.0)], 990)).unwrap_err();
    assert!(
        err.starts_with("simulated work too large: about 60603000 hops in total"),
        "{err}"
    );
}

#[test]
fn zero_cost_failure_ties_go_to_the_first_branch() {
    // D17: both branches fail at t0 = 0, one of them through a zero-duration hop. The lower
    // branch (`s`) wins, in `execute()` and in the engine (simulator and live mode alike).
    let (t, e) = compiled(
        "version: 1\nnodes:\n  app: { type: service }\n  z: { type: service }\n  s: { type: external-api }\n  a: { type: external-api }\nedges:\n  - { from: app, to: z }\n  - { from: app, to: s }\n  - { from: app, to: a }\nendpoints:\n  \"GET /x\": { chain: [ { parallel: [ [ z, s ], [ a ] ] } ] }\n",
        &[("GET /x", "app")],
    );
    let (s, a) = (t.id("s").unwrap(), t.id("a").unwrap());
    let chaos = ChaosState::from([
        (
            s,
            NodeChaos {
                down: true,
                ..NodeChaos::default()
            },
        ),
        (
            a,
            NodeChaos {
                error_rate: 1.0,
                error_status: 500,
                ..NodeChaos::default()
            },
        ),
    ]);
    let cfg = rare();
    let rep = run(&t, &e, &chaos, &cfg).unwrap();
    assert_ne!(rep.requests.len(), 0);
    for r in &rep.requests {
        let live = execute(
            &t,
            &chaos.compile(),
            &ChainRequest {
                endpoint: e.get("GET /x").unwrap(),
                n: r.n,
                seed: cfg.seed,
                pin_delay: std::time::Duration::ZERO,
            },
        );
        assert_eq!(
            live.result,
            ChainResult::Err {
                node: s,
                status: 503,
                kind: FailKind::Status,
                retry_after: None
            }
        );
        assert_eq!(
            r.result,
            ChainResult::Err {
                node: s,
                status: 503,
                kind: FailKind::Status,
                retry_after: None
            }
        );
    }
}

/// One stream on `GET /x` at 0.01 req/s for 10 000 s: about 100 requests, rarely overlapping.
fn rare() -> SimConfig {
    SimConfig {
        streams: vec![Stream {
            endpoint_key: "GET /x".into(),
            rate_per_s: 0.01,
        }],
        duration: Duration::from_secs(10_000),
        warmup: Duration::ZERO,
        seed: 3,
    }
}

#[test]
fn cancelled_siblings_keep_their_slot_but_queued_ones_leave() {
    // Each request calls `slow` (one slot, 10 ms) twice in parallel next to a branch that
    // fails at 5 ms. The running call keeps its slot to 10 ms (cut to 5 ms in the record);
    // the queued call leaves its queue and never runs.
    let (t, e) = compiled(
        "version: 1\nnodes:\n  app: { type: service }\n  slow: { type: external-api, concurrency: 1, proc: 10ms }\n  wait: { type: external-api, proc: 5ms }\n  bad: { type: external-api }\nedges:\n  - { from: app, to: slow }\n  - { from: app, to: wait }\n  - { from: app, to: bad }\nendpoints:\n  \"GET /x\": { chain: [ { parallel: [ slow, slow, [ wait, bad ] ] } ] }\n",
        &[("GET /x", "app")],
    );
    let (slow, bad) = (t.id("slow").unwrap(), t.id("bad").unwrap());
    let down = ChaosState::from([(
        bad,
        NodeChaos {
            down: true,
            ..NodeChaos::default()
        },
    )]);
    let cfg = rare();
    let rep = run(&t, &e, &down, &cfg).unwrap();
    assert!(rep.requests.len() > 50, "{}", rep.requests.len());
    let mut busy = Duration::ZERO;
    for r in &rep.requests {
        assert_eq!(r.latency, Duration::from_millis(5));
        assert_eq!(
            r.result,
            ChainResult::Err {
                node: bad,
                status: 503,
                kind: FailKind::Status,
                retry_after: None
            }
        );
        let ran: Vec<_> = r
            .hops
            .iter()
            .filter(|(h, _)| h.node == slow && h.kind == HopKind::Proc)
            .collect();
        assert_eq!(
            ran.len(),
            1,
            "one call runs, the queued one never does: {r:?}"
        );
        for (h, _) in ran {
            assert_eq!(
                h.start + h.dur,
                Duration::from_millis(5),
                "cut at the failure: {h:?}"
            );
            let from = r.arrival + h.start;
            busy += cfg
                .duration
                .min(from + Duration::from_millis(10))
                .saturating_sub(from);
        }
    }
    // Utilization counts the full 10 ms of every call that started: the slot was kept.
    let util = rep.nodes[slow].utilization;
    assert!(
        (util * cfg.duration.as_secs_f64() - busy.as_secs_f64()).abs() < 1e-9,
        "{util} vs {busy:?}"
    );
}

/// `app` with fixed-cost external APIs: `a` 3 ms, `s` 4 ms.
const FORK: &str = "version: 1\nnodes:\n  app: { type: service }\n  a: { type: external-api, proc: 3ms }\n  s: { type: external-api, proc: 4ms }\nedges:\n  - { from: app, to: a }\n  - { from: app, to: s }\nendpoints:\n";

#[test]
fn fork_join_ends_with_the_slowest_branch() {
    let (t, e) = compiled(
        &format!("{FORK}  \"GET /x\": {{ chain: [ {{ parallel: [ a, s ] }} ] }}\n"),
        &[("GET /x", "app")],
    );
    let rep = run(&t, &e, &ChaosState::default(), &rare()).unwrap();
    assert_ne!(rep.requests.len(), 0);
    assert!(
        rep.requests
            .iter()
            .all(|r| r.latency == Duration::from_millis(4))
    );
}

#[test]
fn one_slot_serializes_parallel_branches() {
    let yaml = FORK.replace("proc: 4ms }", "proc: 4ms, concurrency: 1 }");
    let (t, e) = compiled(
        &format!("{yaml}  \"GET /x\": {{ chain: [ {{ parallel: [ s, s, s ] }} ] }}\n"),
        &[("GET /x", "app")],
    );
    let rep = run(&t, &e, &ChaosState::default(), &rare()).unwrap();
    let s = t.id("s").unwrap();
    let first = &rep.requests[0];
    let at_s: Vec<(Duration, Duration)> = first
        .hops
        .iter()
        .filter(|(h, _)| h.node == s && h.kind == HopKind::Proc)
        .map(|(h, w)| (h.start, *w))
        .collect();
    let ms = Duration::from_millis;
    assert_eq!(
        at_s,
        [(ms(0), ms(0)), (ms(4), ms(4)), (ms(8), ms(8))],
        "FIFO in branch order"
    );
    assert_eq!(first.latency, ms(12));
    assert!(rep.requests.iter().all(|r| r.latency >= ms(12)));
}

#[test]
fn failure_ties_go_to_the_lowest_branch_like_live() {
    // Both branches fail at 2 ms. The higher branch reaches its Fail first in event order
    // (its 2 ms hop was scheduled before the lower branch's second 1 ms hop), so the lower
    // branch must replace it at the same instant (D17).
    let (t, e) = compiled(
        "version: 1\nnodes:\n  app: { type: service }\n  p: { type: external-api, proc: 1ms }\n  q: { type: external-api, proc: 2ms }\n  s: { type: external-api }\n  a: { type: external-api }\nedges:\n  - { from: app, to: p }\n  - { from: app, to: q }\n  - { from: app, to: s }\n  - { from: app, to: a }\nendpoints:\n  \"GET /x\": { chain: [ { parallel: [ [ p, p, s ], [ q, a ] ] } ] }\n",
        &[("GET /x", "app")],
    );
    let (s, a) = (t.id("s").unwrap(), t.id("a").unwrap());
    let chaos = ChaosState::from([
        (
            s,
            NodeChaos {
                down: true,
                ..NodeChaos::default()
            },
        ),
        (
            a,
            NodeChaos {
                error_rate: 1.0,
                error_status: 500,
                ..NodeChaos::default()
            },
        ),
    ]);
    let cfg = rare();
    let rep = run(&t, &e, &chaos, &cfg).unwrap();
    assert_ne!(rep.requests.len(), 0);
    for r in &rep.requests {
        assert_eq!(
            r.result,
            ChainResult::Err {
                node: s,
                status: 503,
                kind: FailKind::Status,
                retry_after: None
            }
        );
        let live = execute(
            &t,
            &chaos.compile(),
            &ChainRequest {
                endpoint: e.get("GET /x").unwrap(),
                n: r.n,
                seed: cfg.seed,
                pin_delay: std::time::Duration::ZERO,
            },
        );
        let hops: Vec<_> = r.hops.iter().map(|(h, _)| h.clone()).collect();
        assert_eq!((hops, r.latency), (live.hops.clone(), live.total()));
    }
}

#[test]
fn branch_waits_under_load() {
    // R7.6: the queueing branch is exercised by parallel blocks, not only by sequences.
    let (t, e) = compiled(
        "version: 1\nnodes:\n  app: { type: service }\n  s: { type: service, concurrency: 1, proc: { mean: 10ms } }\nedges:\n  - { from: app, to: s }\nendpoints:\n  \"GET /x\": { chain: [ { parallel: [ s, s ] } ] }\n",
        &[("GET /x", "app")],
    );
    let cfg = SimConfig {
        streams: vec![Stream {
            endpoint_key: "GET /x".into(),
            rate_per_s: 30.0,
        }],
        duration: Duration::from_secs(100),
        warmup: Duration::from_secs(10),
        seed: 9,
    };
    let rep = run(&t, &e, &ChaosState::default(), &cfg).unwrap();
    let s = t.id("s").unwrap();
    assert!(
        rep.requests.iter().all(|r| r
            .hops
            .iter()
            .any(|(h, w)| h.node == s && h.kind == HopKind::Proc && !w.is_zero())),
        "the second branch always queues behind the first"
    );
    assert!(rep.nodes[s].mean_wait > Duration::ZERO);
}

#[test]
fn login_is_a_stream_key_and_auth_concurrency_queues_logins() {
    // Spec §6: the built-in route is a compiled endpoint, so it streams with no simulator change.
    let yaml = "version: 1\nnodes:\n  gw: { type: gateway }\n  login: { type: auth, concurrency: 1, proc: { mean: 10ms } }\n  users: { type: database, flavor: postgres }\nedges:\n  - { from: gw, to: login }\n  - { from: login, to: users }\n";
    let (t, e) = compiled(yaml, &[]);
    let cfg = SimConfig {
        streams: vec![Stream {
            endpoint_key: "POST /login".into(),
            rate_per_s: 50.0,
        }],
        duration: Duration::from_secs(20),
        warmup: Duration::ZERO,
        seed: 3,
    };
    let rep = run(&t, &e, &ChaosState::default(), &cfg).unwrap();
    let (login, users) = (t.id("login").unwrap(), t.id("users").unwrap());
    assert!(rep.requests.len() > 500, "{}", rep.requests.len());
    assert!(
        rep.requests
            .iter()
            .all(|r| matches!(r.result, ChainResult::Ok))
    );
    // X12: every login reads the user store.
    assert!(rep.requests.iter().all(|r| {
        r.hops
            .iter()
            .any(|(h, _)| h.node == users && h.kind == HopKind::Op(kaanal_core::Op::Read))
    }));
    // One slot at rho = 0.5: logins queue at the auth node.
    let s = &rep.nodes[login];
    assert!(s.max_queue >= 1 && s.mean_wait > Duration::ZERO, "{s:?}");
}

#[test]
fn login_and_jwks_equal_live_at_zero_load() {
    // R7.5: at zero load the simulator replays the live executor exactly. The built-in routes
    // are compiled endpoints, so they must obey the same guard as the spec routes
    // (`zero_load_equals_live_exactly` streams only the `CHAINS` routes).
    let nodes = "  login: { type: auth, proc: { mean: 8ms } }\n  users: { type: database, flavor: postgres }\n  sess: { type: cache, flavor: redis }\n  mail: { type: external-api, proc: 4ms }\n";
    let behind = "  - { from: login, to: users }\n  - { from: login, to: sess }\n  - { from: login, to: mail }\n";
    let configs = [
        (
            "edge with calls: 2",
            format!(
                "version: 1\nnodes:\n  gw: {{ type: gateway }}\n{nodes}edges:\n  - {{ from: gw, to: login, calls: 2 }}\n{behind}"
            ),
        ),
        (
            "no edge",
            format!(
                "version: 1\nnodes:\n  gw: {{ type: gateway }}\n  app: {{ type: service }}\n{nodes}edges:\n  - {{ from: gw, to: app }}\n{behind}"
            ),
        ),
        (
            "auth only",
            format!("version: 1\nnodes:\n{nodes}edges:\n{behind}"),
        ),
    ];
    for (name, yaml) in configs {
        let seed = Config::from_yaml(&yaml).unwrap().seed;
        let (t, e) = compiled(&yaml, &[]);
        let flaky: ChaosState = ["users", "sess", "mail"]
            .iter()
            .filter_map(|n| t.id(n))
            .map(|id| {
                (
                    id,
                    NodeChaos {
                        error_rate: 0.3,
                        ..NodeChaos::default()
                    },
                )
            })
            .collect();
        for chaos in [ChaosState::default(), flaky] {
            let sc = SimConfig {
                streams: ["POST /login", "GET /.well-known/jwks.json"]
                    .iter()
                    .map(|k| Stream {
                        endpoint_key: (*k).to_string(),
                        rate_per_s: 1.0,
                    })
                    .collect(),
                duration: Duration::from_secs(1_000),
                warmup: Duration::ZERO,
                seed,
            };
            let rep = run(&t, &e, &chaos, &sc).unwrap();
            assert!(rep.requests.len() > 1_500, "{name}: {}", rep.requests.len());
            let mut failed = 0;
            for r in &rep.requests {
                let key = &sc.streams[r.stream].endpoint_key;
                let live = execute(
                    &t,
                    &chaos.compile(),
                    &ChainRequest {
                        endpoint: e.get(key).unwrap(),
                        n: r.n,
                        seed,
                        pin_delay: std::time::Duration::ZERO,
                    },
                );
                let at = format!("{name} {key} n={}", r.n);
                let hops: Vec<_> = r.hops.iter().map(|(h, _)| h.clone()).collect();
                assert_eq!(hops, live.hops, "{at}");
                assert!(r.hops.iter().all(|(_, w)| w.is_zero()), "{at}: no queueing");
                assert_eq!(r.latency, live.total(), "{at}");
                assert_eq!(r.result, live.result, "{at}");
                failed += usize::from(r.result != ChainResult::Ok);
            }
            // Chaos on auth's dependencies fails logins (never the proc-only JWKS), so the
            // flaky run really exercises the failure path.
            assert_eq!(failed > 0, !chaos.is_empty(), "{name}: {failed} failures");
        }
    }
}

#[test]
fn too_many_streams_are_rejected_not_truncated() {
    let (t, e) = topo(QUEUED);
    let mut cfg = one_stream(1.0, 10);
    cfg.streams = (0..65_536)
        .map(|i| Stream {
            endpoint_key: format!("GET /s{i}"),
            rate_per_s: 1.0,
        })
        .collect();
    let err = run(&t, &e, &ChaosState::default(), &cfg).unwrap_err();
    assert!(err.contains("at most 65535 streams"), "{err}");
}

#[test]
fn bounded_queue_and_rate_limit_counts_match_the_failed_requests() {
    // 120/s offered into 80 rps: the limiter refuses a third (429) and passes smoothed 80/s (load 0.8)
    // to a one-slot node with a queue of 2, which then overflows (503) on its own.
    let yaml = "version: 1\nnodes:\n  gw: { type: gateway, rate_limit: { rps: 80, burst: 5 } }\n  app: { type: service, concurrency: { limit: 1, queue: 2 }, proc: { mean: 10ms } }\nedges:\n  - { from: gw, to: app }\n";
    let (t, e) = topo(yaml);
    let rep = run(&t, &e, &ChaosState::default(), &one_stream(120.0, 30)).unwrap();
    let (gw, app) = (t.id("gw").unwrap(), t.id("app").unwrap());
    let by = |status: u16| {
        rep.requests
            .iter()
            .filter(|r| matches!(r.result, ChainResult::Err { status: s, .. } if s == status))
            .count() as u64
    };
    assert!(by(429) > 100 && by(503) > 100, "{} {}", by(429), by(503));
    assert_eq!(rep.nodes[gw].limited, by(429));
    assert_eq!(rep.nodes[app].overflowed, by(503));
    assert!(rep.nodes[app].max_queue <= 2, "the queue cap holds");
    assert!(rep.requests.iter().all(|r| match r.result {
        ChainResult::Err {
            status: 429,
            retry_after,
            ..
        } => retry_after.is_some(),
        _ => true,
    }));
}
