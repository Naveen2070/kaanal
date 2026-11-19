//! The load sweep (Step 9a) against closed-form oracles, not against its own output: the M/M/1
//! p99 knee, a token bucket on its overload side, the analytic capacity, determinism, a retry
//! storm and a refused point. Fixtures follow `theory.rs` and `retry_storm.rs`.

use kaanal_core::chain::{EndpointsInput, RouteSpec, compile};
use kaanal_core::{Config, EffectiveChaos, Endpoints, Topology};
use kaanal_sim::sweep::{Limit, Options, SweepResult, Verdict, sweep};
use kaanal_sim::{Limits, SimConfig, Stream};
use std::time::Duration;

const MU: f64 = 100.0; // 10 ms mean service at `app`

fn topo(yaml: &str) -> Topology {
    Topology::build(&Config::from_yaml(yaml).unwrap()).unwrap()
}

/// `app` with `concurrency: c` and service `proc`; everything else costs 0.
fn single(c: u32, proc: &str) -> Topology {
    topo(&format!(
        "version: 1\nnodes:\n  gw: {{ type: gateway }}\n  app: {{ type: service, concurrency: {c}, proc: {proc} }}\nedges:\n  - {{ from: gw, to: app }}\n"
    ))
}

/// A gateway with a token bucket of 100 rps and burst 50 in front of a free service.
fn rate_limited() -> Topology {
    topo(
        "version: 1\nnodes:\n  gw: { type: gateway, rate_limit: { rps: 100, burst: 50 } }\n  app: { type: service }\nedges:\n  - { from: gw, to: app }\n",
    )
}

/// The `retry_storm.rs` fixture: two slots at 10 ms (capacity 200 req/s), a 50 ms timeout, and
/// three tries on the edge whose abandoned work keeps running.
fn retry_storm() -> Topology {
    topo(
        "version: 1
nodes:
  gw: { type: gateway }
  app: { type: service, concurrency: { limit: 2, queue: 200 }, timeout: 50ms, proc: { mean: 10ms } }
edges:
  - { from: gw, to: app, retry: { max_attempts: 3 }, abandon: continue }
",
    )
}

fn endpoints(t: &Topology) -> Endpoints {
    let spec = RouteSpec::inferred("GET /x/{id}", t.id("app").unwrap());
    compile(t, &[spec], EndpointsInput::default()).unwrap()
}

/// One stream at `rate`; duration and warmup are placeholders the sweep ignores.
fn base(rate: f64, seed: u64) -> SimConfig {
    SimConfig {
        streams: vec![Stream {
            endpoint_key: "GET /x/{id}".into(),
            rate_per_s: rate,
            req_bytes: 0,
            resp_bytes: 0,
        }],
        duration: Duration::from_secs(1),
        warmup: Duration::ZERO,
        seed,
    }
}

fn run_with(
    t: &Topology,
    rate: f64,
    seed: u64,
    opts: &Options,
    limits: &Limits,
) -> Result<SweepResult, String> {
    let e = endpoints(t);
    sweep(
        t,
        &e,
        &EffectiveChaos::default(),
        &base(rate, seed),
        opts,
        limits,
        &mut |_| {},
    )
}

fn run(t: &Topology, rate: f64, seed: u64, opts: &Options) -> SweepResult {
    run_with(t, rate, seed, opts, &Limits::default()).unwrap()
}

fn opts(p99_ms: u64, error_rate: f64, target_requests: u64) -> Options {
    Options {
        limit: Limit {
            p99: Duration::from_millis(p99_ms),
            error_rate,
        },
        target_requests,
        ..Options::default()
    }
}

fn knee(r: &SweepResult) -> (f64, f64) {
    match r.verdict {
        Verdict::Knee { k, req_s } => (k, req_s),
        ref v => panic!("no knee: {v:?}"),
    }
}

/// M/M/1 sojourn is Exp(mu - lambda), so p99 = ln(100) / (mu - lambda) and the p99 limit `l_s` is
/// crossed at `lambda* = mu - ln(100) / l_s`.
fn mm1_knee(l_s: f64) -> f64 {
    MU - 100f64.ln() / l_s
}

#[test]
fn mm1_knee_matches_theory() {
    // Measured over seeds 1..=8 at 200 000 requests a point (`mm1_knee_spread_over_seeds`,
    // 2026-11-14): mean +0.20%, sd 0.81%, worst +1.70%; the knees land on the bisection grid
    // (76.61, 77.44, 78.29 req/s). 3% is the worst seen plus more than one sd, and two grid steps.
    let t = single(1, "{ mean: 10ms }");
    let want = mm1_knee(0.2);
    let e = endpoints(&t);
    let mut calls = 0;
    let r = sweep(
        &t,
        &e,
        &EffectiveChaos::default(),
        &base(20.0, 7),
        &opts(200, 1.0, 200_000),
        &Limits::default(),
        &mut |_| calls += 1,
    )
    .unwrap();
    let (_, got) = knee(&r);
    let rel = (got - want).abs() / want;
    eprintln!(
        "M/M/1 knee: got {got:.2} req/s, theory {want:.2} ({:.2}% off)",
        rel * 100.0
    );
    assert!(rel < 0.03, "knee {got} vs {want}");
    assert_eq!(calls, r.points.len(), "on_point once per point run");
}

/// Prints the M/M/1 knee over eight seeds; the tolerance above comes from it.
/// `cargo test -p kaanal-sim --release --test sweep -- --ignored --nocapture`
#[test]
#[ignore = "measurement, about a minute in release"]
fn mm1_knee_spread_over_seeds() {
    let t = single(1, "{ mean: 10ms }");
    let want = mm1_knee(0.2);
    let rels: Vec<f64> = (1..=8)
        .map(|seed| {
            let (_, got) = knee(&run(&t, 20.0, seed, &opts(200, 1.0, 200_000)));
            let rel = (got - want) / want;
            eprintln!("seed {seed}: knee {got:.2} req/s ({:+.2}%)", rel * 100.0);
            rel
        })
        .collect();
    let n = f64::from(u32::try_from(rels.len()).unwrap());
    let mean = rels.iter().sum::<f64>() / n;
    let sd = (rels.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / (n - 1.0)).sqrt();
    eprintln!(
        "mean {:+.2}%, sd {:.2}%, worst {:.2}%",
        mean * 100.0,
        sd * 100.0,
        rels.iter().fold(0f64, |m, r| m.max(r.abs())) * 100.0
    );
}

#[test]
fn rate_limit_knee_is_twice_the_limit_at_half_errors() {
    // Well above R the bucket is empty almost always and passes exactly R, so the error share is
    // 1 - R/lambda and an error limit of 0.5 is crossed at lambda* = 2R. A token bucket also
    // refuses some requests below R (a finite-buffer loss with Poisson arrivals), so R/(1-e) is
    // not an oracle for other error limits; 0.5 keeps the crossing on the overload side.
    let r = run(&rate_limited(), 40.0, 7, &opts(3_600_000, 0.5, 20_000));
    let (_, got) = knee(&r);
    let rel = (got - 200.0).abs() / 200.0;
    eprintln!(
        "rate-limit knee: got {got:.2} req/s, theory 200 ({:.2}% off)",
        rel * 100.0
    );
    assert!(rel < 0.03, "knee {got}");
}

#[test]
fn the_knee_is_at_or_below_the_analytic_limit() {
    let r = run(
        &single(1, "{ mean: 10ms }"),
        20.0,
        7,
        &opts(200, 1.0, 20_000),
    );
    let (_, got) = knee(&r);
    let app = r.points[0].nodes.iter().find(|n| n.name == "app").unwrap();
    let cap = app.saturates_at_req_s.unwrap();
    assert!(got <= cap, "knee {got} above the analytic {cap}");
}

#[test]
fn same_seed_same_sweep() {
    let t = single(1, "{ mean: 10ms }");
    let o = opts(200, 1.0, 20_000);
    assert_eq!(run(&t, 20.0, 7, &o), run(&t, 20.0, 7, &o));
}

#[test]
fn first_to_saturate_is_named_and_cause_reported() {
    // The ladder runs to 8 x 20 = 160 req/s, past app's capacity of 100.
    let r = run(
        &single(1, "{ mean: 10ms }"),
        20.0,
        7,
        &opts(200, 1.0, 20_000),
    );
    let f = r.first_to_saturate.as_ref().expect("app saturates");
    assert_eq!(f.node, "app");
    let a = f.analytic_req_s.unwrap();
    assert!((a - MU).abs() / MU < 0.05, "analytic {a}");
    assert!(f.req_s >= f.k * 20.0 - 1e-9 && f.req_s <= f.k * 20.0 + 1e-9);

    // No node with slots: nothing can saturate, the bucket's refusals cross the limit.
    let r = run(&rate_limited(), 40.0, 7, &opts(3_600_000, 0.5, 20_000));
    assert_eq!(r.dominant_cause, Some("rate_limit"));
    assert!(r.first_to_saturate.is_none());
    assert!(
        r.warnings.iter().any(|w| w.contains("no node saturated")),
        "{:?}",
        r.warnings
    );
}

#[test]
fn a_retry_storm_can_recover() {
    let r = run(&retry_storm(), 40.0, 7, &opts(1000, 0.05, 20_000));
    let (k, _) = knee(&r);
    let lim = &r.limit;
    // the knee is the first crossing: every point at or below it passes, the next one fails
    assert!(r.points.iter().filter(|p| p.k <= k).all(|p| lim.passes(p)));
    let next = r.points.iter().find(|p| p.k > k).unwrap();
    assert!(!lim.passes(next), "{next:?}");
    // Measured: this fixture is monotonic under a fresh run per point (no hysteresis carries
    // from one point to the next), so `recovers_at` stays None; if it ever recovers, it is above.
    eprintln!("retry storm: knee k = {k}, recovers_at {:?}", r.recovers_at);
    if let Some(at) = r.recovers_at {
        assert!(at > k, "recovers at {at}, knee {k}");
        assert!(r.warnings.iter().any(|w| w.contains("recovers")));
    }
}

#[test]
fn a_refused_lowest_point_names_itself() {
    let tiny = Limits {
        max_hop_records: 10.0,
        max_sim_hops: 10.0,
    };
    let e = run_with(
        &single(1, "{ mean: 10ms }"),
        20.0,
        7,
        &Options::default(),
        &tiny,
    )
    .unwrap_err();
    assert!(e.contains("lowest point") && e.contains("k = 0.25"), "{e}");
}

#[test]
fn bad_options_are_refused_before_any_run() {
    let t = single(1, "{ mean: 10ms }");
    for (o, needle) in [
        (
            Options {
                max_factor: f64::NAN,
                ..Options::default()
            },
            "max_factor",
        ),
        (
            Options {
                max_factor: f64::INFINITY,
                ..Options::default()
            },
            "max_factor",
        ),
        (
            Options {
                max_factor: 0.2,
                ..Options::default()
            },
            "max_factor",
        ),
        (
            Options {
                target_requests: 999,
                ..Options::default()
            },
            "target_requests",
        ),
    ] {
        let e = run_with(&t, 20.0, 7, &o, &Limits::default()).unwrap_err();
        assert!(e.contains(needle), "{e}");
    }
}

/// A gateway in front of a free service: nothing queues, nothing fails.
fn free() -> Topology {
    topo(
        "version: 1\nnodes:\n  gw: { type: gateway }\n  app: { type: service }\nedges:\n  - { from: gw, to: app }\n",
    )
}

#[test]
fn a_rate_over_the_engine_cap_cuts_the_ladder() {
    // 200 000 req/s x 8 would pass the engine's 1e6 req/s per stream, so the ladder ends below it
    let r = run(&free(), 200_000.0, 7, &opts(3_600_000, 1.0, 20_000));
    assert!(r.points.iter().all(|p| p.req_s <= 1e6), "{:?}", r.points);
    let at = r.stopped_at.expect("the ladder was cut");
    assert!(at * 200_000.0 > 1e6, "stopped at {at}");
    assert!(
        r.warnings.iter().any(|w| w.contains("cap")),
        "{:?}",
        r.warnings
    );
    assert!(
        matches!(r.verdict, Verdict::HoldsTo { .. }),
        "{:?}",
        r.verdict
    );
}

#[test]
fn a_point_over_the_hop_caps_stops_the_ladder_and_says_at_what_rate() {
    // no slots and a 1 s mean: nothing queues, but each window is at least 10 s, so the requests
    // per point grow with k past `target_requests` and the hop-record cap refuses the top points
    let t = topo(
        "version: 1\nnodes:\n  gw: { type: gateway }\n  app: { type: service, proc: { mean: 1s } }\nedges:\n  - { from: gw, to: app }\n",
    );
    let caps = Limits {
        max_hop_records: 100_000.0,
        max_sim_hops: 1e9,
    };
    let r = run_with(&t, 1000.0, 7, &opts(3_600_000, 1.0, 20_000), &caps).unwrap();
    let k = r.stopped_at.expect("a point was refused");
    let req_s = k * 1000.0;
    let want = format!("ladder stopped at {k:.3}x ({req_s:.0} req/s): run too large");
    assert_eq!(
        r.warnings.iter().filter(|w| **w == want).count(),
        1,
        "{:?}",
        r.warnings
    );
}

#[test]
fn a_base_rate_over_the_cap_at_the_lowest_point_is_a_hard_error() {
    let e = run_with(
        &free(),
        5_000_000.0,
        7,
        &opts(3_600_000, 1.0, 20_000),
        &Limits::default(),
    )
    .unwrap_err();
    assert!(e.contains("lowest point") && e.contains("k = 0.25"), "{e}");
}
