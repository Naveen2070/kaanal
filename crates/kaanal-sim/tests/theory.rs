//! The simulator against closed-form queueing theory (NFR-11, R7.3). All runs are
//! seeded, so each test is deterministic; the 5% tolerance checks the model.

use kaanal_core::chain::{EndpointsInput, RouteSpec, compile};
use kaanal_core::{ChaosState, Config, Endpoints, HopKind, Topology};
use kaanal_sim::{SimConfig, SimReport, Stream, run};
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

/// Runs one stream at `rate` long enough for ~`requests` reported requests (10% warmup).
fn sim(t: &Topology, rate: f64, requests: f64) -> SimReport {
    let spec = RouteSpec::inferred("GET /x/{id}", t.id("app").unwrap());
    let e = compile(t, &[spec], EndpointsInput::default()).unwrap();
    sim_with(t, &e, rate, requests)
}

/// Like [`sim`], for endpoints the caller compiled (`GET /x/{id}` must be one of them).
fn sim_with(t: &Topology, e: &Endpoints, rate: f64, requests: f64) -> SimReport {
    let duration = requests / rate / 0.9;
    let cfg = SimConfig {
        streams: vec![Stream {
            endpoint_key: "GET /x/{id}".into(),
            rate_per_s: rate,
        }],
        duration: Duration::from_secs_f64(duration),
        warmup: Duration::from_secs_f64(duration * 0.1),
        seed: 7,
    };
    run(t, e, &ChaosState::default(), &cfg).unwrap()
}

fn mean_secs(xs: impl Iterator<Item = Duration>) -> f64 {
    let (sum, n) = xs.fold((0.0, 0u32), |(s, n), d| (s + d.as_secs_f64(), n + 1));
    sum / f64::from(n)
}

/// Mean queue wait before `app`'s own hop.
fn app_wait(t: &Topology, rep: &SimReport) -> f64 {
    let app = t.id("app").unwrap();
    mean_secs(rep.requests.iter().flat_map(|r| {
        r.hops
            .iter()
            .filter(move |(h, _)| h.node == app && h.kind == HopKind::Proc)
            .map(|(_, w)| *w)
    }))
}

fn assert_close(got: f64, want: f64, what: &str) {
    let rel = (got - want).abs() / want;
    eprintln!(
        "{what}: got {got:.6} s, theory {want:.6} s ({:.2}% off)",
        rel * 100.0
    );
    assert!(
        rel < 0.05,
        "{what}: got {got:.6} s, theory {want:.6} s ({:.2}% off)",
        rel * 100.0
    );
}

/// Erlang C mean queue wait for M/M/c.
fn erlang_c_wait(c: u32, lam: f64, mu: f64) -> f64 {
    let a = lam / mu;
    let rho = a / f64::from(c);
    let (mut sum, mut term) = (0.0, 1.0); // term = a^k / k!
    for k in 0..c {
        if k > 0 {
            term *= a / f64::from(k);
        }
        sum += term;
    }
    let tail = term * a / f64::from(c) / (1.0 - rho); // a^c / c! / (1 − ρ)
    tail / (sum + tail) / (f64::from(c) * mu - lam)
}

#[test]
fn mm1_matches_theory() {
    let t = single(1, "{ mean: 10ms }");
    for lam in [50.0, 80.0] {
        // High load is autocorrelated: at rho 0.8 one 200k-request run varies by about 1.8% (sd over
        // 16 seeds, unbiased), so it gets 3x the requests rather than a looser tolerance.
        let requests = if lam > 70.0 { 600_000.0 } else { 200_000.0 };
        let rep = sim(&t, lam, requests);
        let got = mean_secs(rep.requests.iter().map(|r| r.latency));
        assert_close(
            got,
            1.0 / (MU - lam),
            &format!("M/M/1 sojourn at ρ = {}", lam / MU),
        );
    }
}

#[test]
fn mmc_matches_erlang_c() {
    let t = single(3, "{ mean: 10ms }");
    let lam = 0.7 * 3.0 * MU;
    let rep = sim(&t, lam, 400_000.0);
    assert_close(
        app_wait(&t, &rep),
        erlang_c_wait(3, lam, MU),
        "M/M/3 wait at ρ = 0.7",
    );
}

#[test]
fn md1_matches_pollaczek_khinchine() {
    let t = single(1, "10ms");
    let (lam, rho) = (70.0, 0.7);
    let rep = sim(&t, lam, 400_000.0);
    assert_close(
        app_wait(&t, &rep),
        rho / (2.0 * MU * (1.0 - rho)),
        "M/D/1 wait at ρ = 0.7",
    );
}

#[test]
fn tandem_matches_jackson() {
    let t = topo(
        "version: 1\nnodes:\n  gw: { type: gateway }\n  app: { type: service, concurrency: 1, proc: { mean: 10ms } }\n  db: { type: database, concurrency: 1, ops: { read: { mean: 5ms }, write: 1ms, scan: 1ms } }\nedges:\n  - { from: gw, to: app }\n  - { from: app, to: db }\n",
    );
    let (lam, mu2) = (50.0, 200.0);
    let rep = sim(&t, lam, 200_000.0);
    let got = mean_secs(rep.requests.iter().map(|r| r.latency));
    assert_close(got, 1.0 / (MU - lam) + 1.0 / (mu2 - lam), "tandem sojourn");
}

#[test]
fn gatewayless_mm1_matches_theory() {
    // R1.23: no gateway node; requests enter at `app`. Queueing must not change.
    let t = topo(
        "version: 1\nnodes:\n  app: { type: service, concurrency: 1, proc: { mean: 10ms } }\n",
    );
    assert_eq!(t.gateway, None);
    let lam = 50.0;
    let rep = sim(&t, lam, 200_000.0);
    let got = mean_secs(rep.requests.iter().map(|r| r.latency));
    assert_close(
        got,
        1.0 / (MU - lam),
        "gateway-less M/M/1 sojourn at ρ = 0.5",
    );
}

#[test]
fn two_way_fork_join_matches_nelson_tantawi() {
    // Every request forks to two identical M/M/1 servers and joins. For K = 2 the mean
    // response time is exact (Flatto & Hahn 1984; Nelson & Tantawi 1985):
    // R2 = (12 − ρ) / 8 × 1 / (μ − λ). Same 5% tolerance as the other theory tests (R7.5);
    // 400k requests because the max of two queues varies more than one queue at ρ = 0.5.
    let src = "version: 1\nnodes:\n  app: { type: service }\n  s1: { type: service, concurrency: 1, proc: { mean: 10ms } }\n  s2: { type: service, concurrency: 1, proc: { mean: 10ms } }\nedges:\n  - { from: app, to: s1 }\n  - { from: app, to: s2 }\nendpoints:\n  \"GET /x/{id}\": { chain: [ { parallel: [ s1, s2 ] } ] }\n";
    let cfg = Config::from_yaml(src).unwrap();
    let t = Topology::build(&cfg).unwrap();
    let spec = RouteSpec::inferred("GET /x/{id}", t.id("app").unwrap());
    let e = compile(&t, &[spec], EndpointsInput::inline(&cfg)).unwrap();
    let lam = 50.0;
    let rep = sim_with(&t, &e, lam, 400_000.0);
    let got = mean_secs(rep.requests.iter().map(|r| r.latency));
    let rho = lam / MU;
    assert_close(
        got,
        (12.0 - rho) / 8.0 / (MU - lam),
        "2-way fork-join sojourn at ρ = 0.5",
    );
}
