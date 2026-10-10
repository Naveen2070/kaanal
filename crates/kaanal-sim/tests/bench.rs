//! NFR-05 measurement: 1M simulated requests. Not a CI test (depends on machine and
//! profile). Run with `cargo test -p kaanal-sim --release --test bench -- --ignored --nocapture`.

use kaanal_core::chain::{EndpointsInput, RouteSpec, compile};
use kaanal_core::{ChaosState, Config, Topology};
use kaanal_sim::{SimConfig, Stream, run};
use std::time::{Duration, Instant};

#[test]
#[ignore = "NFR-05 measurement; run in release with --ignored --nocapture"]
fn one_million_requests() {
    let t = Topology::build(
        &Config::from_yaml(
            "version: 1\nnodes:\n  gw: { type: gateway, proc: { p50: 1ms, p99: 3ms } }\n  app: { type: service, concurrency: 4, proc: { mean: 2ms } }\n  db: { type: database, concurrency: 2, ops: { read: { mean: 0.5ms }, write: 1ms, scan: 3ms } }\nedges:\n  - { from: gw, to: app, net: 1ms }\n  - { from: app, to: db, net: { p50: 1ms, p99: 2ms } }\n",
        )
        .unwrap(),
    )
    .unwrap();
    let spec = RouteSpec::inferred("GET /users/{id}", t.id("app").unwrap());
    let e = compile(&t, &[spec], EndpointsInput::default()).unwrap();
    let cfg = SimConfig {
        streams: vec![Stream {
            endpoint_key: "GET /users/{id}".into(),
            rate_per_s: 1_000.0,
        }],
        duration: Duration::from_secs(1_000),
        warmup: Duration::ZERO,
        seed: 1,
    };
    let start = Instant::now();
    let rep = run(&t, &e, &ChaosState::default(), &cfg).unwrap();
    let took = start.elapsed();
    println!(
        "NFR-05: {} simulated requests in {took:.2?}",
        rep.requests.len()
    );
    assert!(rep.requests.len() > 990_000, "{}", rep.requests.len());
}
