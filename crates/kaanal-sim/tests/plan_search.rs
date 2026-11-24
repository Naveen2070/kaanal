//! The cheapest search (Step 9c-2) on the real engine, against Erlang C. M/M/c fixtures
//! (exponential service, Poisson arrivals) as `plan_walk.rs`, seed 7.
//!
//! Oracle: Erlang C (A. K. Erlang, 1917; Kleinrock, *Queueing Systems* vol. 1, 1975, sec. 3.5;
//! Gross and Harris, *Fundamentals of Queueing Theory*, M/M/c): an arrival waits with probability
//! C(c, a), a = lambda / mu, and the wait is past t with probability C(c, a) e^{-(c mu - lambda) t}.
//! The design holds when it is stable (c mu > lambda) and that tail at the 900 ms margin is under
//! 1 percent; c* is the fewest copies that do. Capacity is c mu, checked within 5 percent.

use std::collections::BTreeMap;
use std::time::Duration;

use kaanal_core::chain::{EndpointsInput, RouteSpec, compile};
use kaanal_core::resize::MoveKind;
use kaanal_core::{Config, EffectiveChaos, Endpoints, Topology};
use kaanal_sim::plan::{
    PlanVerdict, Request, SearchOptions, Searched, StopReason, Target, search, walk,
};
use kaanal_sim::{Limits, SimConfig, Stream};

const MU: f64 = 100.0; // 10 ms mean service per slot
const LAMBDA: f64 = 250.0;

/// A gateway and `app` (its YAML line) in front of `x` (its YAML line); the one route ends at `x`.
fn design(app: &str, x: &str) -> Config {
    Config::from_yaml(&format!(
        "version: 1\nnodes:\n  gw: {{ type: gateway }}\n{app}{x}edges:\n  - {{ from: gw, to: app }}\n  - {{ from: app, to: x }}\n"
    ))
    .unwrap()
}

const APP: &str = "  app: { type: service }\n";

/// An M/M/c service, priced, copies only (`cpu` next to the instance): c copies cost c units.
fn priced_x(copies: u16) -> String {
    format!(
        "  x: {{ type: service, concurrency: 1, proc: {{ mean: 10ms }}, instance: aws/m5.large, cpu: {{ cores: 2 }}, replicas: {copies} }}\n"
    )
}

fn endpoints(cfg: &Config) -> Endpoints {
    let t = Topology::build(cfg).unwrap();
    let spec = RouteSpec::inferred("GET /x/{id}", t.id("app").unwrap());
    compile(&t, &[spec], EndpointsInput::default()).unwrap()
}

/// 100 req/s configured, the target a multiple of it: `LAMBDA` req/s.
fn base() -> SimConfig {
    SimConfig {
        streams: vec![Stream {
            endpoint_key: "GET /x/{id}".into(),
            rate_per_s: 100.0,
            req_bytes: 0,
            resp_bytes: 0,
        }],
        duration: Duration::from_secs(1),
        warmup: Duration::ZERO,
        seed: 7,
    }
}

fn req() -> Request {
    Request {
        target: Target::Multiple(LAMBDA / 100.0),
        ..Request::default()
    }
}

fn run(cfg: &Config) -> Searched {
    search(
        cfg,
        &endpoints(cfg),
        &EffectiveChaos::default(),
        &base(),
        &req(),
        &SearchOptions::default(),
        &Limits::default(),
        &mut || Duration::ZERO,
        &mut |_| {},
    )
    .unwrap()
}

fn unit_price() -> u64 {
    let c = kaanal_core::cost::estimate(&design(APP, &priced_x(1)), &BTreeMap::new());
    c.nodes
        .iter()
        .find(|n| n.name == "x")
        .unwrap()
        .unit_micro_usd_hr
        .unwrap()
}

/// Erlang C: probability an arrival waits in M/M/c (offered load `a = lambda / mu`).
fn erlang_c(c: u32, a: f64) -> f64 {
    let (mut sum, mut term) = (0.0, 1.0); // term = a^k / k!
    for k in 0..c {
        if k > 0 {
            term *= a / f64::from(k);
        }
        sum += term;
    }
    let tail = term * a / f64::from(c) / (1.0 - a / f64::from(c));
    tail / (sum + tail)
}

/// The fewest copies that are stable and keep P(wait > 0.9 s) under 1 percent.
fn c_star() -> u16 {
    let holds = |c: u32| {
        let c_mu = f64::from(c) * MU;
        c_mu > LAMBDA && erlang_c(c, LAMBDA / MU) * (-(c_mu - LAMBDA) * 0.9).exp() < 0.01
    };
    u16::try_from((1..=64u32).find(|&c| holds(c)).unwrap()).unwrap()
}

/// Review focus 1: 8 copies at 250 req/s (utilisation 0.31) hold with copies to spare; the search
/// takes them off down to the Erlang C minimum c* (3 here: 2 copies are unstable).
#[test]
fn a_holding_design_with_spare_copies_is_shrunk_to_the_erlang_c_minimum() {
    let s = run(&design(APP, &priced_x(8)));
    let c = c_star();
    assert_eq!(c, 3);
    assert!(s.info.design_in_force_holds);
    assert_eq!(s.plan.verdict, PlanVerdict::Holds);
    assert_eq!(s.plan.config.nodes["x"].replicas(), c);
    assert!(
        matches!(&s.plan.steps[..], [st] if st.mv.node == "x" && st.mv.kind == MoveKind::Copies { from: 8, to: c }),
        "{:?}",
        s.plan.steps
    );
    let u = unit_price();
    assert_eq!(s.plan.price_before, Some(8 * u));
    assert_eq!(s.plan.price_after, Some(u64::from(c) * u));
    let cap = s.plan.end.capacity_req_s.unwrap();
    let want = f64::from(c) * MU;
    assert!((cap - want).abs() / want < 0.05, "{cap} vs {want}");
    assert_eq!(s.info.stop, StopReason::Exhausted);
    assert!(s.info.runs <= SearchOptions::default().run_cap);
}

#[test]
fn a_design_that_is_already_the_cheapest_says_nothing_cheaper_holds() {
    let s = run(&design(APP, &priced_x(3)));
    assert_eq!(s.plan.verdict, PlanVerdict::AlreadyHolds);
    assert_eq!(s.plan.steps, []);
    assert!(s.info.design_in_force_holds);
    assert_eq!(s.info.stop, StopReason::Exhausted);
    // the check, then 2 copies (unstable, fails) bisects the two candidates
    assert_eq!(s.info.runs, 2);
    assert_eq!(s.plan.price_after, s.plan.price_before);
}

/// `x` at one copy does not hold (the walk adds copies to c*); `app` has 8 copies of a 2 ms
/// service (mu 500 each), so one copy is at utilisation 0.5 and the search takes the other 7 off.
#[test]
fn a_design_that_does_not_hold_is_walked_then_shrunk() {
    let app = "  app: { type: service, concurrency: 1, proc: { mean: 2ms }, instance: aws/m5.large, cpu: { cores: 2 }, replicas: 8 }\n";
    let cfg = design(app, &priced_x(1));
    let walked = walk(
        &cfg,
        &endpoints(&cfg),
        &EffectiveChaos::default(),
        &base(),
        &req(),
        &Limits::default(),
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(walked.verdict, PlanVerdict::Holds);
    let s = run(&cfg);
    assert!(!s.info.design_in_force_holds);
    assert_eq!(s.plan.verdict, PlanVerdict::Holds);
    // the walk's steps first, unchanged, then the shrink of app
    assert_eq!(s.plan.steps[..walked.steps.len()], walked.steps[..]);
    let rest = &s.plan.steps[walked.steps.len()..];
    assert!(
        matches!(rest, [st] if st.mv.node == "app" && st.mv.kind == MoveKind::Copies { from: 8, to: 1 }),
        "{rest:?}"
    );
    assert_eq!(
        s.plan.config.nodes["x"].replicas(),
        walked.config.nodes["x"].replicas()
    );
    assert!(s.plan.price_after < walked.price_after);
}
