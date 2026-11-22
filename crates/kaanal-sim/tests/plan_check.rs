//! The plan check (Step 9c-1): one run at a target load, against the M/M/1 closed form. Fixture
//! follows `sweep.rs`; seed 7.

use kaanal_core::chain::{EndpointsInput, RouteSpec, compile};
use kaanal_core::{Config, EffectiveChaos, Endpoints, Topology};
use kaanal_sim::plan::{Target, check};
use kaanal_sim::sweep::Limit;
use kaanal_sim::{Limits, SimConfig, Stream};
use std::time::Duration;

const MU: f64 = 100.0; // 10 ms mean service at `app`

fn topo(replicas: u16) -> Topology {
    let y = format!(
        "version: 1\nnodes:\n  gw: {{ type: gateway }}\n  app: {{ type: service, concurrency: 1, replicas: {replicas}, proc: {{ mean: 10ms }} }}\nedges:\n  - {{ from: gw, to: app }}\n"
    );
    Topology::build(&Config::from_yaml(&y).unwrap()).unwrap()
}

fn endpoints(t: &Topology) -> Endpoints {
    let spec = RouteSpec::inferred("GET /x/{id}", t.id("app").unwrap());
    compile(t, &[spec], EndpointsInput::default()).unwrap()
}

fn base(rate: f64) -> SimConfig {
    SimConfig {
        streams: vec![Stream {
            endpoint_key: "GET /x/{id}".into(),
            rate_per_s: rate,
            req_bytes: 0,
            resp_bytes: 0,
        }],
        duration: Duration::from_secs(1),
        warmup: Duration::ZERO,
        seed: 7,
    }
}

fn run(t: &Topology, e: &Endpoints, rate: f64, k: f64) -> kaanal_sim::plan::Check {
    check(
        t,
        e,
        &EffectiveChaos::default(),
        &base(rate),
        k,
        &Limit::default(),
        &Limits::default(),
    )
    .unwrap()
}

#[test]
fn a_target_below_capacity_holds() {
    let t = topo(1);
    let c = run(&t, &endpoints(&t), 50.0, 1.0);
    assert!(c.holds, "{:?}", c.point);
}

#[test]
fn a_target_above_capacity_does_not_hold_and_names_the_node() {
    let t = topo(1);
    let c = run(&t, &endpoints(&t), 50.0, 3.0);
    assert!(!c.holds);
    assert_eq!(c.bottleneck.as_deref(), Some("app"));
}

/// M/M/1 saturates at lambda = mu; tolerance 5 percent, the existing theory-test bound.
#[test]
fn capacity_estimate_matches_the_closed_form_within_five_percent() {
    let t = topo(1);
    let c = run(&t, &endpoints(&t), 50.0, 1.0);
    let cap = c.capacity_req_s.unwrap();
    assert!((cap - MU).abs() / MU < 0.05, "{cap}");
}

#[test]
fn a_rate_target_and_the_equal_multiple_give_the_same_check() {
    let t = topo(1);
    let e = endpoints(&t);
    let b = base(50.0);
    let k = Target::PerSecond(75.0).factor(&b).unwrap();
    assert!((k - 1.5).abs() < 1e-12);
    assert_eq!(Target::Multiple(1.5).factor(&b).unwrap(), k);
    assert_eq!(run(&t, &e, 50.0, k), run(&t, &e, 50.0, 1.5));
}

#[test]
fn target_factor_rejects_zero_negative_nan_and_empty_streams() {
    let b = base(50.0);
    for t in [
        Target::Multiple(0.0),
        Target::Multiple(-1.0),
        Target::Multiple(f64::NAN),
        Target::PerSecond(0.0),
        Target::PerSecond(-5.0),
        Target::PerSecond(f64::INFINITY),
    ] {
        assert!(t.factor(&b).is_err(), "{t:?}");
    }
    let empty = SimConfig {
        streams: vec![],
        ..base(50.0)
    };
    assert!(Target::PerSecond(10.0).factor(&empty).is_err());
    assert!(Target::Multiple(2.0).factor(&empty).is_err());
}

#[test]
fn a_candidate_with_more_copies_reuses_the_endpoints_and_chaos_of_the_original() {
    let one = topo(1);
    let ends = endpoints(&one);
    let two = topo(2);
    let before = run(&one, &ends, 50.0, 1.0).point.requests;
    let after = run(&two, &ends, 50.0, 1.0).point.requests;
    assert!(before > 0 && after > 0 && before / 10 < after && after < before * 10);
}
