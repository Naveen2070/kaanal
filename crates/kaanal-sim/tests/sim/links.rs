//! Links in the simulator: payload sizes make link hops, equal to `execute` at zero load. The
//! links are fast (a transfer is microseconds), because a link is one FIFO server: two requests
//! that overlap on it really queue, which is not zero load.

use crate::common::*;
use crate::shared::compiled;
use kaanal_core::{ChaosState, Config, HopKind};

/// Two bandwidth edges and a `per_kb` node, behind a gateway.
const LINKS: &str = "version: 1\nseed: 31\nnodes:\n  gw: { type: gateway, proc: { mean: 1ms } }\n  app: { type: service, proc: { mean: 2ms }, per_kb: 100us }\n  db: { type: database, flavor: postgres }\nedges:\n  - { from: gw, to: app, net: 1ms, bandwidth: 1000000000 }\n  - { from: app, to: db, net: { mean: 1ms }, bandwidth: 500000000 }\n";

#[test]
fn link_chain_matches_execute() {
    let seed = Config::from_yaml(LINKS).unwrap().seed;
    let (t, e) = compiled(LINKS, &APP_ROUTES);
    let mut sc = zero_load_config(&["GET /users/{id}", "POST /orders"], seed);
    for (s, (req, resp)) in sc.streams.iter_mut().zip([(2_000, 10_000), (50_000, 0)]) {
        s.req_bytes = req;
        s.resp_bytes = resp;
    }
    // short, so two transfers meeting on a link (which queue, correctly) is out of the picture
    sc.duration = std::time::Duration::from_secs(300);
    let rep = kaanal_sim::run(&t, &e, &ChaosState::default(), &sc).unwrap();
    compare_with_execute(&t, &e, &ChaosState::default().compile(), &rep, &sc, "links");
    let kinds = |resp: bool| {
        rep.requests
            .iter()
            .flat_map(|r| r.hops.iter())
            .filter(|(h, _)| matches!(h.kind, HopKind::Link { resp: x, .. } if x == resp))
            .count()
    };
    assert!(kinds(false) > 100, "request transfers were planned");
    assert!(kinds(true) > 100, "response transfers were planned");
}
