//! `detect` in a simulated run (step 7d, RULES 91): a copy of the callee goes down 10 s in, the balancer
//! notices 5 s later, and in between a third of the calls (round-robin over three copies) fail fast. The
//! oracle is the window arithmetic: failed share = (1 / copies) x detect / duration.

use kaanal_core::chain::{EndpointsInput, RouteSpec, compile};
use kaanal_core::{ChainResult, ChaosKey, ChaosLayer, ChaosState, Config, Topology};
use kaanal_sim::{SimConfig, Stream, run};
use std::time::Duration;

/// The share of requests that failed, for edge fields `edge` and a copy down from 10 s.
fn failed_share(edge: &str) -> f64 {
    let y = format!(
        "version: 1\nnodes:\n  gw: {{ type: gateway }}\n  app: {{ type: service, replicas: 3, proc: 1ms }}\nedges:\n  - {{ from: gw, to: app{edge} }}\n"
    );
    let t = Topology::build(&Config::from_yaml(&y).unwrap()).unwrap();
    let spec = RouteSpec::inferred("GET /x/{id}", t.id("app").unwrap());
    let e = compile(&t, &[spec], EndpointsInput::default()).unwrap();
    let mut chaos = ChaosState::default();
    chaos.set(
        ChaosKey::Replica(t.id("app").unwrap(), 1),
        ChaosLayer {
            down: Some(true),
            down_from: Some(Duration::from_secs(10)),
            ..ChaosLayer::default()
        },
    );
    let cfg = SimConfig {
        streams: vec![Stream {
            endpoint_key: "GET /x/{id}".into(),
            rate_per_s: 100.0,
            req_bytes: 0,
            resp_bytes: 0,
        }],
        duration: Duration::from_secs(30),
        warmup: Duration::ZERO,
        seed: 7,
    };
    let rep = run(&t, &e, &chaos, &cfg).unwrap();
    let bad = rep
        .requests
        .iter()
        .filter(|r| !matches!(r.result, ChainResult::Ok))
        .count();
    #[allow(clippy::cast_precision_loss)]
    let share = bad as f64 / rep.requests.len() as f64;
    share
}

#[test]
fn the_failed_share_is_the_detect_window_over_the_run_over_the_copies() {
    // 5 s of 30 s, one copy of three
    let want = 5.0 / 30.0 / 3.0;
    let got = failed_share(", detect: 5s");
    assert!(
        (got - want).abs() / want < 0.15,
        "failed share {got:.4}, window arithmetic {want:.4}"
    );
}

#[test]
fn without_detect_the_copy_is_skipped_the_moment_it_goes_down() {
    assert!(failed_share("") < 0.001);
}

#[test]
fn a_longer_detect_costs_proportionally_more() {
    let (short, long) = (failed_share(", detect: 2s"), failed_share(", detect: 8s"));
    let ratio = long / short;
    assert!(
        (3.0..5.0).contains(&ratio),
        "8 s / 2 s should be about 4x: {ratio:.2}"
    );
}
