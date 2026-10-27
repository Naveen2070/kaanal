//! Login and JWKS as stream keys, equal to live at zero load.

use crate::shared::compiled;
use kaanal_core::{ChainRequest, ChainResult, ChaosState, Config, HopKind, NodeChaos, execute};
use kaanal_sim::{SimConfig, Stream, run};
use std::time::Duration;

#[test]
fn login_is_a_stream_key_and_auth_concurrency_queues_logins() {
    // Spec section 6: the built-in route is a compiled endpoint, so it streams with no simulator change.
    let yaml = "version: 1\nnodes:\n  gw: { type: gateway }\n  login: { type: auth, concurrency: 1, proc: { mean: 10ms } }\n  users: { type: database, flavor: postgres }\nedges:\n  - { from: gw, to: login }\n  - { from: login, to: users }\n";
    let (t, e) = compiled(yaml, &[]);
    let cfg = SimConfig {
        streams: vec![Stream {
            endpoint_key: "POST /login".into(),
            rate_per_s: 50.0,
            req_bytes: 0,
            resp_bytes: 0,
        }],
        duration: Duration::from_secs(20),
        warmup: Duration::ZERO,
        seed: 3,
    };
    let rep = run(&t, &e, &ChaosState::default(), &cfg).unwrap();
    let (login, users) = (t.id("login").unwrap(), t.id("users").unwrap());
    assert!(rep.requests.len() > 500, "{}", rep.requests.len());
    assert!(
        rep.requests
            .iter()
            .all(|r| matches!(r.result, ChainResult::Ok))
    );
    // X12: every login reads the user store.
    assert!(rep.requests.iter().all(|r| {
        r.hops
            .iter()
            .any(|(h, _)| h.node == users && h.kind == HopKind::Op(kaanal_core::Op::Read))
    }));
    // One slot at rho = 0.5: logins queue at the auth node.
    let s = &rep.nodes[login];
    assert!(s.max_queue >= 1 && s.mean_wait > Duration::ZERO, "{s:?}");
}

#[test]
fn login_and_jwks_equal_live_at_zero_load() {
    // R7.5: at zero load the simulator replays the live executor exactly. The built-in routes
    // are compiled endpoints, so they must obey the same guard as the spec routes
    // (`zero_load_equals_live_exactly` streams only the `CHAINS` routes).
    let nodes = "  login: { type: auth, proc: { mean: 8ms } }\n  users: { type: database, flavor: postgres }\n  sess: { type: cache, flavor: redis }\n  mail: { type: external-api, proc: 4ms }\n";
    let behind = "  - { from: login, to: users }\n  - { from: login, to: sess }\n  - { from: login, to: mail }\n";
    let configs = [
        (
            "edge with calls: 2",
            format!(
                "version: 1\nnodes:\n  gw: {{ type: gateway }}\n{nodes}edges:\n  - {{ from: gw, to: login, calls: 2 }}\n{behind}"
            ),
        ),
        (
            "no edge",
            format!(
                "version: 1\nnodes:\n  gw: {{ type: gateway }}\n  app: {{ type: service }}\n{nodes}edges:\n  - {{ from: gw, to: app }}\n{behind}"
            ),
        ),
        (
            "auth only",
            format!("version: 1\nnodes:\n{nodes}edges:\n{behind}"),
        ),
    ];
    for (name, yaml) in configs {
        let seed = Config::from_yaml(&yaml).unwrap().seed;
        let (t, e) = compiled(&yaml, &[]);
        let flaky: ChaosState = ["users", "sess", "mail"]
            .iter()
            .filter_map(|n| t.id(n))
            .map(|id| {
                (
                    id,
                    NodeChaos {
                        error_rate: 0.3,
                        ..NodeChaos::default()
                    },
                )
            })
            .collect();
        for chaos in [ChaosState::default(), flaky] {
            let sc = SimConfig {
                streams: ["POST /login", "GET /.well-known/jwks.json"]
                    .iter()
                    .map(|k| Stream {
                        endpoint_key: (*k).to_string(),
                        rate_per_s: 1.0,
                        req_bytes: 0,
                        resp_bytes: 0,
                    })
                    .collect(),
                duration: Duration::from_secs(1_000),
                warmup: Duration::ZERO,
                seed,
            };
            let rep = run(&t, &e, &chaos, &sc).unwrap();
            assert!(rep.requests.len() > 1_500, "{name}: {}", rep.requests.len());
            let mut failed = 0;
            for r in &rep.requests {
                let key = &sc.streams[r.stream].endpoint_key;
                let live = execute(
                    &t,
                    &chaos.compile(),
                    &ChainRequest {
                        endpoint: e.get(key).unwrap(),
                        n: r.n,
                        seed,
                        pin_delay: std::time::Duration::ZERO,
                        req_bytes: 0,
                        resp_bytes: 0,
                    },
                );
                let at = format!("{name} {key} n={}", r.n);
                let hops: Vec<_> = r.hops.iter().map(|(h, _)| h.clone()).collect();
                assert_eq!(hops, live.hops, "{at}");
                assert!(r.hops.iter().all(|(_, w)| w.is_zero()), "{at}: no queueing");
                assert_eq!(r.latency, live.total(), "{at}");
                assert_eq!(r.result, live.result, "{at}");
                failed += usize::from(r.result != ChainResult::Ok);
            }
            // Chaos on auth's dependencies fails logins (never the proc-only JWKS), so the
            // flaky run really exercises the failure path.
            assert_eq!(failed > 0, !chaos.is_empty(), "{name}: {failed} failures");
        }
    }
}
