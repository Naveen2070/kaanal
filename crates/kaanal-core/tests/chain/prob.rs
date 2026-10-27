//! `prob` through the compiler: the edge default, the step override and the step's own range.

use crate::common::*;
use kaanal_core::chain::Step;

/// `BASE` with the `app -> db` edge given a `prob` (the edge is the last-but-three edge line).
fn base_with_edge_prob(p: &str) -> String {
    BASE.replace(
        "  - { from: app, to: db }",
        &format!("  - {{ from: app, to: db, prob: {p} }}"),
    )
}

/// The `db` call inside `app`'s default body of `GET /users/{id}`.
fn app_db_prob(t: &kaanal_core::Topology, eps: &kaanal_core::Endpoints) -> f64 {
    let get = eps.get("GET /users/{id}").unwrap();
    let Step::Call(app) = &get.chain.steps[2] else {
        panic!("entry call")
    };
    let db = t.id("db").unwrap();
    app.body
        .steps
        .iter()
        .find_map(|s| match s {
            Step::Call(c) if c.node == db => Some(c.prob),
            _ => None,
        })
        .expect("app calls db")
}

#[test]
fn an_edge_prob_is_the_default_call_prob() {
    let (t, r) = build(&base_with_edge_prob("0.25"), &ROUTES);
    assert_eq!(app_db_prob(&t, &r.unwrap()), 0.25);
}

#[test]
fn no_prob_means_one() {
    let (t, eps) = ok("");
    assert_eq!(app_db_prob(&t, &eps), 1.0);
}

#[test]
fn a_step_prob_overrides_the_edge() {
    let src = format!(
        "{}  \"GET /users/{{id}}\": {{ chain: [ {{ call: db, prob: 0.5 }} ] }}\n",
        base_with_edge_prob("0.25")
    );
    let (t, r) = build(&src, &ROUTES);
    let eps = r.unwrap();
    let get = eps.get("GET /users/{id}").unwrap();
    let db = t.id("db").unwrap();
    let probs: Vec<f64> = get
        .body
        .steps
        .iter()
        .filter_map(|s| match s {
            Step::Call(c) if c.node == db => Some(c.prob),
            _ => None,
        })
        .collect();
    assert_eq!(probs, [0.5]);
}

#[test]
fn a_step_prob_out_of_range_is_a_positioned_error() {
    for bad in ["0", "1.2", "-1"] {
        let src = format!(
            "{BASE}  \"GET /users/{{id}}\": {{ chain: [ {{ call: db, prob: {bad} }} ] }}\n"
        );
        let e = err(&src).to_string();
        assert!(e.starts_with("23:"), "{bad}: {e}");
        assert!(e.contains("prob must be above 0"), "{bad}: {e}");
    }
}
