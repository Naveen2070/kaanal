//! `expected_visits` against the simulator: each node's completed hops per request match.

use crate::shared::compiled;
use kaanal_core::{ChaosState, expected_visits};
use kaanal_sim::{SimConfig, Stream, run};
use std::time::Duration;

/// Every visit is exactly one hop here (a gateway and a service charge one `proc` each, the cache
/// one lookup, the database one op), so `completed` counts visits.
const WORLD: &str = "version: 1\nseed: 5\nnodes:\n  gw: { type: gateway, proc: 1ms }\n  app: { type: service, proc: 2ms }\n  db: { type: database, flavor: postgres }\n  pay: { type: external-api, proc: 3ms }\n  c: { type: cache, hit_ratio: 0.4, hit: 1ms, miss: 1ms }\nedges:\n  - { from: gw, to: app }\n  - { from: app, to: db }\n  - { from: app, to: pay, prob: 0.25 }\n  - { from: app, to: c }\nendpoints:\n  \"GET /x/{id}\":\n    chain: [ { cache: c }, { call: db, repeat: 2, prob: 0.5 }, { parallel: [ { call: pay, prob: 0.8 }, db ] }, pay ]\n";

#[test]
fn visits_match_sim_completed_counts() {
    let (t, e) = compiled(WORLD, &[("GET /x/{id}", "app")]);
    let ep = e.get("GET /x/{id}").unwrap();
    let want = expected_visits(&t, ep);
    let cfg = SimConfig {
        streams: vec![Stream {
            endpoint_key: "GET /x/{id}".into(),
            rate_per_s: 1000.0,
            req_bytes: 0,
            resp_bytes: 0,
        }],
        duration: Duration::from_secs(200),
        warmup: Duration::ZERO,
        seed: 7,
    };
    let rep = run(&t, &e, &ChaosState::default(), &cfg).unwrap();
    let arrivals = f64::from(u32::try_from(rep.requests.len()).unwrap());
    assert!(arrivals > 150_000.0, "{arrivals}");
    for (i, n) in t.nodes.iter().enumerate() {
        let got = f64::from(u32::try_from(rep.nodes[i].completed).unwrap()) / arrivals;
        let rel = (got - want[i]).abs() / want[i];
        assert!(
            rel < 0.05,
            "{}: sim {got:.4}, expected {:.4}",
            n.name,
            want[i]
        );
    }
}
