//! The simulator against closed-form queueing theory (NFR-11, R7.3). All runs are
//! seeded, so each test is deterministic; the 5% tolerance checks the model.

use kaanal_core::chain::{EndpointsInput, RouteSpec, compile};
use kaanal_core::{ChainResult, ChaosState, Config, Endpoints, HopKind, Topology};
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

/// `app` with `concurrency: { limit: c, queue: q }` and exponential 10 ms service.
fn bounded(c: u32, q: u32) -> Topology {
    topo(&format!(
        "version: 1\nnodes:\n  gw: {{ type: gateway }}\n  app: {{ type: service, concurrency: {{ limit: {c}, queue: {q} }}, proc: {{ mean: 10ms }} }}\nedges:\n  - {{ from: gw, to: app }}\n"
    ))
}

/// Share of reported requests that failed with `status`.
fn share(rep: &SimReport, status: u16) -> f64 {
    let n = rep
        .requests
        .iter()
        .filter(|r| matches!(r.result, ChainResult::Err { status: s, .. } if s == status))
        .count();
    let count = |n: usize| f64::from(u32::try_from(n).expect("a few million requests"));
    count(n) / count(rep.requests.len())
}

/// Erlang B blocking probability for `c` servers and offered load `a` (M/M/c/c).
fn erlang_b(c: u32, a: f64) -> f64 {
    let (mut sum, mut term) = (1.0, 1.0);
    for k in 1..=c {
        term *= a / f64::from(k);
        sum += term;
    }
    term / sum
}

/// Blocking probability of M/M/c/K (K places in all, c servers): the share of arrivals that find it full.
fn mmck_blocking(c: u32, k: u32, a: f64) -> f64 {
    let mut p = Vec::new(); // unnormalised state probabilities
    let mut term = 1.0;
    for n in 0..=k {
        if n > 0 {
            term *= a / f64::from(n.min(c));
        }
        p.push(term);
    }
    p[k as usize] / p.iter().sum::<f64>()
}

#[test]
fn a_pool_without_a_queue_matches_erlang_b() {
    let t = bounded(3, 0);
    let (lam, a) = (240.0, 2.4);
    let rep = sim(&t, lam, 400_000.0);
    assert_close(
        share(&rep, 503),
        erlang_b(3, a),
        "Erlang B blocking, c = 3, a = 2.4",
    );
}

#[test]
fn a_bounded_queue_matches_mmck_blocking() {
    let t = bounded(2, 3);
    let rep = sim(&t, 180.0, 600_000.0);
    assert_close(
        share(&rep, 503),
        mmck_blocking(2, 5, 1.8),
        "M/M/2/5 blocking, a = 1.8",
    );
}

#[test]
fn overflow_status_is_the_configured_one() {
    let t = topo(
        "version: 1\nnodes:\n  gw: { type: gateway }\n  app: { type: service, concurrency: { limit: 1, queue: 0, overflow_status: 502 }, proc: { mean: 10ms } }\nedges:\n  - { from: gw, to: app }\n",
    );
    let rep = sim(&t, 150.0, 20_000.0);
    assert!(share(&rep, 502) > 0.2 && share(&rep, 503) == 0.0);
}

#[test]
fn a_rate_limit_under_overload_passes_exactly_its_rps() {
    // Poisson 300/s into 100 rps with a burst of 50: the bucket is empty almost always, so the
    // accepted rate is the refill rate. (A burst of 1 would pass λ/(1+λT) = 75/s instead.)
    let t = topo(
        "version: 1\nnodes:\n  gw: { type: gateway, rate_limit: { rps: 100, burst: 50 } }\n  app: { type: service }\nedges:\n  - { from: gw, to: app }\n",
    );
    let spec = RouteSpec::inferred("GET /x/{id}", t.id("app").unwrap());
    let e = compile(&t, &[spec], EndpointsInput::default()).unwrap();
    let rep = sim_with(&t, &e, 300.0, 120_000.0);
    let ok = rep
        .requests
        .iter()
        .filter(|r| matches!(r.result, ChainResult::Ok))
        .count();
    let span =
        rep.requests.last().unwrap().arrival.as_secs_f64() - rep.requests[0].arrival.as_secs_f64();
    assert_close(
        f64::from(u32::try_from(ok).unwrap()) / span,
        100.0,
        "accepted rate",
    );
}
