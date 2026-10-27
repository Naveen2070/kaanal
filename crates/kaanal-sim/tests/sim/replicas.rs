//! Replicas in the simulator: equal to `execute` at zero load, replica indices included.

use crate::common::*;
use crate::shared::compiled;
use kaanal_core::{ChaosKey, ChaosLayer, ChaosState, Config};
use std::time::Duration;

const KEYS: [&str; 3] = ["GET /users", "GET /users/{id}", "POST /orders"];

fn down() -> ChaosLayer {
    ChaosLayer {
        down: Some(true),
        ..ChaosLayer::default()
    }
}

#[test]
fn zero_load_equals_execute_with_replicas_and_replica_chaos() {
    let seed = Config::from_yaml(REPLICAS).unwrap().seed;
    let (t, e) = compiled(REPLICAS, &APP_ROUTES);
    let (db, app) = (t.id("db").unwrap(), t.id("app").unwrap());
    let mut chaos = ChaosState::default();
    // one db replica down (skipped), one app replica slow
    chaos.set(ChaosKey::Replica(db, 0), down());
    chaos.set(
        ChaosKey::Replica(app, 1),
        ChaosLayer {
            latency_add: Some(Duration::from_millis(20)),
            ..ChaosLayer::default()
        },
    );
    for (label, c) in [("calm", ChaosState::default()), ("replica chaos", chaos)] {
        let (rep, _) = assert_zero_load_equal(&t, &e, &c, label, &KEYS, seed);
        assert!(
            rep.requests.len() > 5_000,
            "{label}: {}",
            rep.requests.len()
        );
        // the plans really used more than one replica
        let used: std::collections::BTreeSet<u16> = rep
            .requests
            .iter()
            .flat_map(|r| r.hops.iter().map(|(h, _)| h.replica))
            .collect();
        assert!(used.len() > 1, "{label}: replicas used {used:?}");
    }
}

#[test]
fn a_downed_replica_never_serves_in_the_simulator() {
    let (t, e) = compiled(REPLICAS, &APP_ROUTES);
    let db = t.id("db").unwrap();
    let mut chaos = ChaosState::default();
    chaos.set(ChaosKey::Replica(db, 1), down());
    let rep = kaanal_sim::run(&t, &e, &chaos, &zero_load_config(&KEYS, 9)).unwrap();
    let on_down = rep
        .requests
        .iter()
        .flat_map(|r| r.hops.iter())
        .filter(|(h, _)| h.node == db && h.replica == 1)
        .count();
    assert_eq!(on_down, 0);
    assert!(
        rep.requests
            .iter()
            .any(|r| r.hops.iter().any(|(h, _)| h.node == db))
    );
}

#[test]
fn an_inline_auth_route_on_a_replicated_gateway_runs_without_a_bad_replica() {
    // Local validation (no gateway-to-auth edge) puts auth's work in the gateway's sequence; it
    // used to inherit the gateway's replica index and index past auth's single replica.
    let yaml = "version: 1\nnodes:\n  gw: { type: gateway, proc: 1ms, replicas: 3, concurrency: 2 }\n  login: { type: auth, proc: 2ms, concurrency: 2 }\n  app: { type: service, proc: 5ms }\nedges:\n  - { from: gw, to: app }\n";
    let (t, e) = compiled(yaml, &[("GET /x", "app")]);
    let cfg = kaanal_sim::SimConfig {
        streams: vec![kaanal_sim::Stream {
            endpoint_key: "POST /login".into(),
            rate_per_s: 50.0,
        }],
        duration: Duration::from_secs(20),
        warmup: Duration::ZERO,
        seed: 3,
    };
    let rep = kaanal_sim::run(&t, &e, &ChaosState::default(), &cfg).unwrap();
    let login = t.id("login").unwrap();
    assert!(rep.requests.len() > 500);
    assert!(
        rep.requests
            .iter()
            .flat_map(|r| r.hops.iter())
            .filter(|(h, _)| h.node == login)
            .all(|(h, _)| h.replica == 0)
    );
    assert!(rep.nodes[login].completed > 500);
}
