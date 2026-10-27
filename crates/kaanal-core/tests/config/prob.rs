//! `prob`: a branch probability on an edge (the default) and on a `call:` step.

use kaanal_core::Config;
use kaanal_core::config::{StepConfig, parse_endpoints};

const BASE: &str = "version: 1\nnodes:\n  gw: { type: gateway }\n  app: { type: service }\n  db: { type: database, flavor: postgres }\n  c: { type: cache, flavor: redis }\nedges:\n";

fn cfg(edges: &str) -> Result<Config, String> {
    Config::from_yaml(&format!("{BASE}{edges}")).map_err(|e| e.to_string())
}

#[test]
fn prob_parses_on_a_step_and_an_edge() {
    let c = cfg("  - { from: app, to: db, prob: 0.5 }\n").unwrap();
    assert_eq!(c.edges[0].prob, Some(0.5));
    let eps = parse_endpoints("\"GET /x\": { chain: [ { call: db, prob: 0.2 } ] }\n").unwrap();
    let step = &eps["GET /x"].chain.as_ref().unwrap()[0];
    let StepConfig::Call(call) = step else {
        panic!("a call")
    };
    assert_eq!(call.prob, Some(0.2));
}

#[test]
fn an_edge_without_prob_has_none() {
    assert_eq!(
        cfg("  - { from: app, to: db }\n").unwrap().edges[0].prob,
        None
    );
}

#[test]
fn prob_must_be_in_0_exclusive_to_1() {
    for bad in ["0", "-0.5", "1.5", ".nan"] {
        let err = cfg(&format!("  - {{ from: app, to: db, prob: {bad} }}\n")).unwrap_err();
        assert!(err.contains("prob"), "{bad}: {err}");
        assert!(
            err.starts_with("8:"),
            "{bad} is positioned on its line: {err}"
        );
    }
    cfg("  - { from: app, to: db, prob: 1 }\n").unwrap();
}

#[test]
fn prob_on_an_edge_to_a_cache_is_rejected() {
    let err = cfg("  - { from: app, to: c, prob: 0.5 }\n").unwrap_err();
    assert!(err.contains("hit ratio"), "{err}");
}

#[test]
fn prob_on_a_gateway_edge_is_rejected() {
    let err = cfg("  - { from: gw, to: app, prob: 0.5 }\n").unwrap_err();
    assert!(err.contains("gateway"), "{err}");
}

#[test]
fn a_cache_step_rejects_prob() {
    let err = parse_endpoints("\"GET /x\": { chain: [ { cache: c, prob: 0.5 } ] }\n").unwrap_err();
    assert!(err.to_string().contains("prob"), "{err}");
}
