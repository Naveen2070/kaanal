//! Deadline ties, earliest deadline, parallel branches and repeats.

use crate::common::*;
use kaanal_core::{ChainResult, ChaosState, Endpoints, FailKind, NodeChaos, PlanItem, Topology};
use std::time::Duration;

/// Fixed timings: nginx proc 1 ms, edge nets 1 ms, app proc 5 ms, every db op 10 ms.
pub(crate) fn custom(app: &str, db: &str, db2: &str, chain: &str) -> (Topology, Endpoints) {
    build(&format!(
        "version: 1\nnodes:\n  nginx: {{ type: gateway, proc: 1ms }}\n  app: {{ type: service, spec: openapi.yaml, proc: 5ms{app} }}\n  db: {{ type: database{db}, ops: {{ read: 10ms, write: 10ms, scan: 10ms }} }}\n  db2: {{ type: database{db2}, ops: {{ read: 10ms, write: 10ms, scan: 10ms }} }}\nedges:\n  - {{ from: nginx, to: app, net: 1ms }}\n  - {{ from: app, to: db, net: 1ms }}\n  - {{ from: app, to: db2, net: 1ms }}\nendpoints:\n  \"GET /users/{{id}}\":\n    chain: {chain}\n"
    ))
}

#[test]
fn a_planned_fail_at_the_deadline_instant_wins_the_tie() {
    // span(app) opens at 1 ms, due 8 ms; db is down and fails at its op start, also 8 ms,
    // while db2's op runs on to 18 ms, so the span is violated.
    let (t, e) = custom(", timeout: 7ms", "", "", "[ { parallel: [ db, db2 ] } ]");
    let down = chaos_on(
        &t,
        "db",
        NodeChaos {
            down: true,
            ..NodeChaos::default()
        },
    );
    let out = run(&t, &e, &down, 0);
    let ChainResult::Err {
        node, status, kind, ..
    } = &out.result
    else {
        panic!("{:?}", out.result)
    };
    assert_eq!(
        (*node, *status, *kind),
        (t.id("db").unwrap(), 503, FailKind::Status)
    );
    assert_eq!(out.total(), ms(8));
}

#[test]
fn the_earliest_deadline_wins_not_the_first_span_in_plan_order() {
    // outer app span: opens 1 ms, due 16 ms, body ends 18 ms (violated).
    // inner db span: opens 7 ms, due 9 ms, body ends 18 ms (violated, and due earlier).
    let (t, e) = custom(", timeout: 15ms", ", timeout: 2ms", "", "[ db ]");
    let out = run(&t, &e, &ChaosState::default(), 0);
    let ChainResult::Err { node, kind, .. } = &out.result else {
        panic!("{:?}", out.result)
    };
    assert_eq!((*node, *kind), (t.id("db").unwrap(), FailKind::Timeout));
    assert_eq!(out.total(), ms(9));
}

#[test]
fn a_span_in_a_parallel_branch_cuts_every_hop_at_its_deadline() {
    // db2 span: opens 7 ms, due 9 ms; both branches' ops would run to 18 ms.
    let (t, e) = custom("", "", ", timeout: 2ms", "[ { parallel: [ db, db2 ] } ]");
    let out = run(&t, &e, &ChaosState::default(), 0);
    let ChainResult::Err { node, kind, .. } = &out.result else {
        panic!("{:?}", out.result)
    };
    assert_eq!((*node, *kind), (t.id("db2").unwrap(), FailKind::Timeout));
    assert_eq!(out.total(), ms(9));
    assert!(
        out.hops.iter().all(|h| h.start + h.dur <= ms(9)),
        "{:?}",
        out.hops
    );
}

#[test]
fn repeat_makes_one_span_per_repetition() {
    let (t, e) = custom("", ", timeout: 50ms", "", "[ { call: db, repeat: 2 } ]");
    let out = run(&t, &e, &ChaosState::default(), 0);
    assert!(matches!(out.result, ChainResult::Ok));
    let ats: Vec<Duration> = out
        .plan
        .items
        .iter()
        .filter_map(|i| match i {
            PlanItem::Span { at, size: 2, .. } => Some(*at),
            _ => None,
        })
        .collect();
    assert_eq!(ats.len(), 2);
    assert!(ats[0] < ats[1], "{ats:?}");
}
