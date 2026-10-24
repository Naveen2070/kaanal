//! Streams named by compiled endpoints, and the hop budget.

use crate::shared::compiled;
use kaanal_core::ChaosState;
use kaanal_sim::{SimConfig, Stream, run};
use std::time::Duration;

#[test]
fn streams_name_compiled_endpoints() {
    // D15: any compiled endpoint may be a stream, a static one included.
    let (t, e) = compiled(
        "version: 1\nnodes:\n  gw: { type: gateway, proc: 1ms }\n  shop: { type: monolith, proc: 5ms }\nedges:\n  - { from: gw, to: shop }\nendpoints:\n  \"GET /health\": { override: { body: 1 } }\n",
        &[("GET /shop/{id}", "shop")],
    );
    let stream = |key: &str| SimConfig {
        streams: vec![Stream {
            endpoint_key: key.into(),
            rate_per_s: 10.0,
        }],
        duration: Duration::from_secs(10),
        warmup: Duration::ZERO,
        seed: 1,
    };
    let none = ChaosState::default();
    let rep = run(&t, &e, &none, &stream("GET /shop/{id}")).unwrap();
    assert_ne!(rep.requests.len(), 0);
    assert!(
        rep.requests
            .iter()
            .all(|r| r.latency == Duration::from_millis(6)),
        "gw 1 ms + shop 5 ms"
    );
    let rep = run(&t, &e, &none, &stream("GET /health")).unwrap();
    assert!(
        rep.requests
            .iter()
            .all(|r| r.hops.len() == 1 && r.latency == Duration::from_millis(1))
    );
    let err = run(&t, &e, &none, &stream("GET /nope")).unwrap_err();
    assert_eq!(err, "stream 0: unknown endpoint 'GET /nope'");
}

#[test]
fn hop_budget_counts_chain_length() {
    // R6.13: app.proc + 100 × (net + svc.proc + 100 × (net + db op)) = 20 201 hops per request.
    let (t, e) = compiled(
        "version: 1\nnodes:\n  app: { type: service }\n  svc: { type: service }\n  db: { type: database, flavor: postgres }\nedges:\n  - { from: app, to: svc }\n  - { from: svc, to: db }\nendpoints:\n  \"GET /x\": { chain: [ { call: svc, repeat: 100, chain: [ { call: db, repeat: 100 } ] } ] }\n",
        &[("GET /x", "app")],
    );
    assert_eq!(e.get("GET /x").unwrap().chain.hops, 20_201);
    let cfg = SimConfig {
        streams: vec![Stream {
            endpoint_key: "GET /x".into(),
            rate_per_s: 1.0,
        }],
        duration: Duration::from_secs(1_000),
        warmup: Duration::ZERO,
        seed: 1,
    };
    let err = run(&t, &e, &ChaosState::default(), &cfg).unwrap_err();
    assert_eq!(
        err,
        "workload too large: about 1000 arrivals with up to 20201000 hops in total (max 20000000); lower the rate, the duration or the chain length"
    );
}

#[test]
fn hop_budget_sums_streams_and_skips_warmup() {
    // The record budget counts reported arrivals; a second cap counts all simulated hops.
    // Two 20 201-hop endpoints. Each stream alone is under 20 M records at 0.6 req/s, the sum
    // is not; and a warmup shrinks the count to the reported arrivals only.
    let chain = "chain: [ { call: svc, repeat: 100, chain: [ { call: db, repeat: 100 } ] } ]";
    let (t, e) = compiled(
        &format!(
            "version: 1\nnodes:\n  app: {{ type: service }}\n  svc: {{ type: service }}\n  db: {{ type: database, flavor: postgres }}\nedges:\n  - {{ from: app, to: svc }}\n  - {{ from: svc, to: db }}\nendpoints:\n  \"GET /x\": {{ {chain} }}\n  \"GET /y\": {{ {chain} }}\n"
        ),
        &[("GET /x", "app"), ("GET /y", "app")],
    );
    let stream = |key: &str, rate_per_s| Stream {
        endpoint_key: key.into(),
        rate_per_s,
    };
    let cfg = |streams, warmup| SimConfig {
        streams,
        duration: Duration::from_secs(1_000),
        warmup: Duration::from_secs(warmup),
        seed: 1,
    };
    let none = ChaosState::default();
    let err = run(
        &t,
        &e,
        &none,
        &cfg(vec![stream("GET /x", 0.6), stream("GET /y", 0.6)], 0),
    )
    .unwrap_err();
    assert!(
        err.contains("about 1200 arrivals with up to 24241200 hops"),
        "{err}"
    );
    let err = run(&t, &e, &none, &cfg(vec![stream("GET /x", 1.2)], 100)).unwrap_err();
    assert!(
        err.contains("about 1080 arrivals with up to 21817080 hops"),
        "only post-warmup arrivals count: {err}"
    );
    // A long warmup keeps the records small but not the CPU time: warmup arrivals count
    // against the total-work cap (30 reported arrivals, 3 000 simulated).
    let err = run(&t, &e, &none, &cfg(vec![stream("GET /x", 3.0)], 990)).unwrap_err();
    assert!(
        err.starts_with("simulated work too large: about 60603000 hops in total"),
        "{err}"
    );
}
