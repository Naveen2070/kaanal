//! `least-conn` and `p2c` end to end: the engine sends each call to the less loaded copy, so a slow
//! copy gets less traffic than under round robin and the tail improves.

#![expect(
    clippy::cast_precision_loss,
    reason = "visit counts are far below 2^52"
)]

use crate::common::*;
use kaanal_core::{ChaosKey, ChaosLayer, ChaosState};
use kaanal_sim::{SimConfig, run, summarize};
use std::time::Duration;

/// gw -> app, three copies with two slots each; copy 0 is 40 ms slower than its siblings.
fn system(balance: &str) -> String {
    format!(
        "version: 1\nnodes:\n  gw: {{ type: gateway }}\n  app: {{ type: service, proc: 10ms, concurrency: 2, replicas: 3, balance: {balance} }}\nedges:\n  - {{ from: gw, to: app }}\n"
    )
}

fn slow_first_copy(t: &kaanal_core::Topology) -> ChaosState {
    let mut s = ChaosState::default();
    s.set(
        ChaosKey::Replica(t.id("app").unwrap(), 0),
        ChaosLayer {
            latency_add: Some(Duration::from_millis(40)),
            ..ChaosLayer::default()
        },
    );
    s
}

/// Visits per copy and the p99 (ms) of a 100/s run.
fn measure(balance: &str) -> (Vec<u64>, f64) {
    let (t, e) = topo(&system(balance));
    let cfg = SimConfig {
        warmup: Duration::from_secs(5),
        ..one_stream(100.0, 120)
    };
    let rep = run(&t, &e, &slow_first_copy(&t), &cfg).unwrap();
    let s = summarize(&t, &e, &cfg, &rep);
    let visits = s.nodes[t.id("app").unwrap()]
        .replicas
        .iter()
        .map(|r| r.visits)
        .collect();
    (visits, s.latency.p99_ms)
}

#[test]
fn least_conn_sends_a_slow_copy_less_than_round_robin_and_the_tail_improves() {
    let (rr, rr_p99) = measure("round-robin");
    let (lc, lc_p99) = measure("least-conn");
    let share = |v: &[u64]| v[0] as f64 / v.iter().sum::<u64>() as f64;
    assert!(
        (share(&rr) - 1.0 / 3.0).abs() < 0.02,
        "round robin is even: {rr:?}"
    );
    assert!(
        share(&lc) < 0.26,
        "least-conn gives the slow copy less: {lc:?}"
    );
    assert!(lc_p99 < rr_p99, "p99 {lc_p99} ms against {rr_p99} ms");
}

#[test]
fn p2c_also_shifts_load_off_the_slow_copy() {
    let (p2, _) = measure("p2c");
    let share = p2[0] as f64 / p2.iter().sum::<u64>() as f64;
    assert!(share < 0.31, "{p2:?}");
}

#[test]
fn a_run_with_least_conn_is_repeatable() {
    assert_eq!(measure("least-conn").0, measure("least-conn").0);
}

/// The Studio starter "A balanced tier" (ui/src/canvas/starters.ts), word for word.
const STARTER: &str = "version: 1
seed: 7
nodes:
  gateway: { type: gateway, concurrency: 256, proc: { p50: 1ms, p99: 3ms } }
  balancer: { type: load-balancer, concurrency: 512, proc: { p50: 200us, p99: 600us } }
  api: { type: service, concurrency: 8, replicas: 3, balance: p2c, proc: { p50: 12ms, p99: 40ms } }
  db: { type: database, flavor: postgres, pool: { size: 20 } }
edges:
  - { from: gateway, to: balancer, net: { p50: 1ms, p99: 3ms } }
  - { from: balancer, to: api, net: { p50: 1ms, p99: 3ms } }
  - { from: api, to: db, net: { p50: 1ms, p99: 3ms } }
endpoints:
  \"GET /items\":
    op: read
    override: { body: { items: [] } }
    chain: [balancer]
";

#[test]
fn the_balanced_tier_starter_runs_through_the_balancer_and_spreads_over_the_copies() {
    let cfg = kaanal_core::Config::from_yaml(STARTER).unwrap();
    let t = kaanal_core::Topology::build(&cfg).unwrap();
    let e = kaanal_core::compile(&t, &[], kaanal_core::EndpointsInput::inline(&cfg)).unwrap();
    let sim = SimConfig {
        streams: vec![kaanal_sim::Stream {
            endpoint_key: "GET /items".into(),
            rate_per_s: 150.0,
            req_bytes: 0,
            resp_bytes: 0,
        }],
        duration: Duration::from_secs(30),
        warmup: Duration::from_secs(3),
        seed: 7,
    };
    let rep = run(&t, &e, &ChaosState::default(), &sim).unwrap();
    let s = summarize(&t, &e, &sim, &rep);
    assert_eq!(s.failed, 0, "{:?}", s.errors);
    let visits = |n: &str| s.nodes[t.id(n).unwrap()].visits_per_s;
    assert!(
        (visits("balancer") - 150.0).abs() < 15.0,
        "{}",
        visits("balancer")
    );
    let copies = &s.nodes[t.id("api").unwrap()].replicas;
    assert_eq!(copies.len(), 3);
    assert!(
        copies.iter().all(|c| c.visits > 0),
        "every copy serves some"
    );
}
