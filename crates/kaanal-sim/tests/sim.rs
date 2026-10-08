use kaanal_core::{ChainRequest, ChaosState, Config, HopKind, NodeChaos, Op, Topology, execute};
use kaanal_sim::{SimConfig, Stream, run};
use std::path::Path;
use std::time::Duration;

/// One service with one slot and exponential 10 ms service; everything else costs 0.
const QUEUED: &str = "version: 1\nnodes:\n  gw: { type: gateway }\n  app: { type: service, concurrency: 1, proc: { mean: 10ms } }\nedges:\n  - { from: gw, to: app }\n";

/// The spec routes owned by `app` in the fixture and the sample (same openapi.yaml).
const ENDPOINTS: [(&str, &str); 3] = [
    ("GET", "/users"),
    ("GET", "/users/{id}"),
    ("POST", "/orders"),
];

fn topo(yaml: &str) -> Topology {
    Topology::build(&Config::from_yaml(yaml).unwrap()).unwrap()
}

fn one_stream(t: &Topology, rate: f64, secs: u64) -> SimConfig {
    SimConfig {
        streams: vec![Stream {
            entry: t.id("app").unwrap(),
            op: Op::Read,
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

/// The Phase One fixture and the shipped sample, as (name, YAML).
fn sample_configs() -> Vec<(&'static str, String)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    [
        ("fixture", "../kaanal-server/tests/fixtures/kaanal.yaml"),
        ("example", "../../examples/nginx-server/kaanal.yaml"),
    ]
    .into_iter()
    .map(|(name, p)| (name, std::fs::read_to_string(root.join(p)).unwrap()))
    .collect()
}

#[test]
fn zero_load_equals_live_exactly() {
    for (name, yaml) in sample_configs() {
        let cfg = Config::from_yaml(&yaml).unwrap();
        let t = Topology::build(&cfg).unwrap();
        let mut flaky_db = ChaosState::default();
        flaky_db.insert(
            t.id("db").unwrap(),
            NodeChaos {
                error_rate: 0.5,
                ..NodeChaos::default()
            },
        );
        for chaos in [ChaosState::default(), flaky_db] {
            let sc = SimConfig {
                streams: ENDPOINTS
                    .iter()
                    .map(|(m, p)| Stream {
                        entry: t.id("app").unwrap(),
                        op: Op::infer(m, p),
                        endpoint_key: format!("{m} {p}"),
                        rate_per_s: 1.0,
                    })
                    .collect(),
                duration: Duration::from_secs(2_000),
                warmup: Duration::ZERO,
                seed: cfg.seed,
            };
            let rep = run(&t, &chaos, &sc).unwrap();
            assert!(rep.requests.len() > 5_000, "{name}: {}", rep.requests.len());
            for r in &rep.requests {
                let s = &sc.streams[r.stream];
                let live = execute(
                    &t,
                    &chaos,
                    &ChainRequest {
                        entry: Some(s.entry),
                        op: s.op,
                        endpoint_key: &s.endpoint_key,
                        n: r.n,
                        seed: cfg.seed,
                    },
                );
                let hops: Vec<_> = r.hops.iter().map(|(h, _)| h.clone()).collect();
                let at = format!("{name} {} n={}", s.endpoint_key, r.n);
                assert_eq!(hops, live.hops, "{at}");
                assert!(
                    r.hops.iter().all(|(_, w)| w.is_zero()),
                    "{at}: no queueing at zero load"
                );
                assert_eq!(r.latency, live.total(), "{at}");
                assert_eq!(r.result, live.result, "{at}");
            }
        }
    }
}

#[test]
fn engine_matches_lindley_recursion() {
    let t = topo(QUEUED);
    let rep = run(&t, &ChaosState::default(), &one_stream(&t, 80.0, 125)).unwrap();
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
    assert!(rep.requests.len() > 9_000, "{}", rep.requests.len());
    let queued = rep
        .requests
        .iter()
        .filter(|r| r.hops.iter().any(|(_, w)| !w.is_zero()))
        .count();
    assert!(queued > 1_000, "the test must really queue: {queued}");
}

#[test]
fn same_seed_same_report() {
    // Determinism, not tie coverage: with continuous arrival gaps exact ties are rare, so the
    // (time, seq) rule is documented and reasoned about, not pinned by this test.
    let t = topo(&QUEUED.replace("{ mean: 10ms }", "10ms"));
    let cfg = one_stream(&t, 90.0, 60);
    let a = run(&t, &ChaosState::default(), &cfg).unwrap();
    assert_eq!(a, run(&t, &ChaosState::default(), &cfg).unwrap());
    let b = run(&t, &ChaosState::default(), &SimConfig { seed: 43, ..cfg }).unwrap();
    assert_ne!(a, b, "a different seed gives a different run");
}

#[test]
fn down_gateway_completes_with_no_hops() {
    let t = topo(QUEUED);
    let mut down = ChaosState::default();
    down.insert(
        t.gateway,
        NodeChaos {
            down: true,
            ..NodeChaos::default()
        },
    );
    let rep = run(&t, &down, &one_stream(&t, 10.0, 10)).unwrap();
    assert_ne!(rep.requests.len(), 0, "the stream must produce requests");
    for r in &rep.requests {
        assert!(r.hops.is_empty() && r.latency.is_zero(), "{r:?}");
        assert!(matches!(
            r.result,
            kaanal_core::ChainResult::Err { status: 503, .. }
        ));
    }
}

#[test]
fn overload_grows_the_queue_but_finishes() {
    let t = topo(QUEUED);
    // ρ = 1.5: arrivals outpace the single slot.
    let rep = run(&t, &ChaosState::default(), &one_stream(&t, 150.0, 20)).unwrap();
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
    let t = topo(QUEUED);
    let rep = run(&t, &ChaosState::default(), &one_stream(&t, 50.0, 20)).unwrap();
    let gw = &rep.nodes[t.gateway];
    assert_eq!(
        (gw.utilization, gw.max_queue, gw.mean_wait),
        (0.0, 0, Duration::ZERO)
    );
    assert_eq!(gw.completed, rep.requests.len() as u64);
}

#[test]
fn warmup_requests_are_not_reported() {
    let t = topo(QUEUED);
    let mut cfg = one_stream(&t, 50.0, 20);
    cfg.warmup = Duration::from_secs(10);
    let rep = run(&t, &ChaosState::default(), &cfg).unwrap();
    assert!(rep.requests.iter().all(|r| r.arrival >= cfg.warmup));
    assert!(
        rep.requests.first().unwrap().n > 0,
        "counters keep running through warmup"
    );
}

#[test]
#[expect(
    clippy::too_many_lines,
    reason = "one table of cases, one per validation rule"
)]
fn rejects_bad_config() {
    let t = topo(QUEUED);
    let ok = || one_stream(&t, 10.0, 10);
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
                c.streams[0].entry = t.gateway;
                c
            },
            "not a service",
        ),
        (
            {
                let mut c = ok();
                c.streams[0].entry = 99;
                c
            },
            "not a node",
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
        let err = run(&t, &ChaosState::default(), &cfg).unwrap_err();
        assert!(err.contains(want), "want '{want}' in: {err}");
    }
}

#[test]
fn request_counters_start_at_zero_and_run_on_consecutively() {
    // Live mode's per-endpoint counter starts at 0; the zero-load test replays each record's own `n`,
    // so this is what pins the sim to live's numbering.
    let t = topo(QUEUED);
    let rep = run(&t, &ChaosState::default(), &one_stream(&t, 50.0, 20)).unwrap();
    let ns: Vec<u64> = rep.requests.iter().map(|r| r.n).collect();
    assert_eq!(ns, (0..ns.len() as u64).collect::<Vec<_>>());
}

#[test]
fn utilization_divides_by_the_slot_count() {
    // 2 slots, 10 ms exponential service, 100 req/s: 1.0 busy slot on average, so 0.5 per slot.
    let t = topo(&QUEUED.replace("concurrency: 1", "concurrency: 2"));
    let mut cfg = one_stream(&t, 100.0, 200);
    cfg.warmup = Duration::from_secs(10);
    let rep = run(&t, &ChaosState::default(), &cfg).unwrap();
    let u = rep.nodes[t.id("app").unwrap()].utilization;
    assert!((u - 0.5).abs() < 0.03, "utilization {u}");
}

#[test]
fn absurd_chaos_latency_is_rejected_not_a_panic() {
    let t = topo(QUEUED);
    let mut chaos = ChaosState::default();
    chaos.insert(
        t.id("app").unwrap(),
        NodeChaos {
            latency_add: Duration::from_hours(2),
            ..NodeChaos::default()
        },
    );
    let err = run(&t, &chaos, &one_stream(&t, 1.0, 10)).unwrap_err();
    assert!(err.contains("latency_add"), "{err}");
}
