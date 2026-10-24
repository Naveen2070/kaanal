//! Spans and timeouts: hang, reset, malformed, the deadline and gateway timeout.

use crate::common::*;
use kaanal_core::{
    ChainResult, ChaosState, Endpoints, ErrorKind, FailKind, HANG_CAP, HopKind, NodeChaos,
    PlanItem, Topology,
};
use std::time::Duration;

pub(crate) fn timed(extra_nodes: &str) -> (Topology, Endpoints) {
    build(&BASE.replace(
        "nginx: { type: gateway,",
        &format!("nginx: {{ type: gateway,{extra_nodes}"),
    ))
}

#[test]
fn a_node_without_timeout_makes_no_spans() {
    let (t, e) = topo();
    let out = run(&t, &e, &ChaosState::default(), 0);
    assert!(
        out.plan
            .items
            .iter()
            .all(|i| !matches!(i, PlanItem::Span { .. }))
    );
}

#[test]
fn timeout_on_the_callee_wraps_edge_hop_and_body_in_a_span() {
    let (t, e) = build(&BASE.replace(
        "db:    { type: database,",
        "db:    { type: database, timeout: 50ms,",
    ));
    let out = run(&t, &e, &ChaosState::default(), 0);
    let i = out
        .plan
        .items
        .iter()
        .position(|i| matches!(i, PlanItem::Span { .. }))
        .unwrap();
    let PlanItem::Span {
        node,
        timeout,
        size,
        ..
    } = &out.plan.items[i]
    else {
        unreachable!()
    };
    assert_eq!(
        (*node, *timeout),
        (t.id("db").unwrap(), Duration::from_millis(50))
    );
    // the net hop to db and the db op are inside
    assert_eq!(*size, 2);
}

#[test]
fn execute_reports_a_zero_load_timeout_at_the_deadline() {
    // db read takes 4ms p50 / 20ms p99; a 1ms deadline on db always passes before the op ends
    let (t, e) = build(&BASE.replace(
        "db:    { type: database,",
        "db:    { type: database, timeout: 1ms,",
    ));
    let out = run(&t, &e, &ChaosState::default(), 0);
    let ChainResult::Err {
        node, status, kind, ..
    } = &out.result
    else {
        panic!("{:?}", out.result)
    };
    assert_eq!(
        (*node, *status, *kind),
        (t.id("db").unwrap(), 504, FailKind::Timeout)
    );
    let span_start = out
        .plan
        .items
        .iter()
        .find_map(|i| match i {
            PlanItem::Span { at, .. } => Some(*at),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        out.total(),
        span_start + Duration::from_millis(1),
        "answered at the deadline"
    );
    assert!(
        out.hops.iter().all(|h| h.start + h.dur <= out.total()),
        "hops are cut at the deadline"
    );
}

#[test]
fn gateway_timeout_covers_the_whole_chain() {
    let (t, e) = timed(" timeout: 2ms,");
    let out = run(&t, &e, &ChaosState::default(), 0);
    let ChainResult::Err { node, kind, .. } = &out.result else {
        panic!("{:?}", out.result)
    };
    assert_eq!((*node, *kind), (t.id("nginx").unwrap(), FailKind::Timeout));
}

#[test]
fn a_planned_fail_earlier_than_the_deadline_wins() {
    // gateway down fails at offset 0, before its own 1 ms deadline; the planned Fail decides
    let (t, e) = build(&BASE.replace(
        "nginx: { type: gateway,",
        "nginx: { type: gateway, timeout: 1ms,",
    ));
    let gw_down = chaos_on(
        &t,
        "nginx",
        NodeChaos {
            down: true,
            ..NodeChaos::default()
        },
    );
    let out = run(&t, &e, &gw_down, 0);
    let ChainResult::Err { kind, status, .. } = &out.result else {
        panic!()
    };
    assert_eq!(
        (*kind, *status),
        (FailKind::Status, 503),
        "fail at offset 0 is earlier than 1ms"
    );
}

#[test]
fn reset_and_malformed_are_planned_fails_with_their_kind() {
    let (t, e) = topo();
    for (k, want) in [
        (ErrorKind::Reset, FailKind::Reset),
        (ErrorKind::Malformed, FailKind::Malformed),
    ] {
        let c = chaos_on(
            &t,
            "db",
            NodeChaos {
                error_rate: 1.0,
                error_kind: k,
                ..NodeChaos::default()
            },
        );
        let out = run(&t, &e, &c, 0);
        let ChainResult::Err {
            node, kind, status, ..
        } = &out.result
        else {
            panic!()
        };
        assert_eq!((*node, *kind, *status), (t.id("db").unwrap(), want, 502));
    }
}

#[test]
fn hang_is_a_capped_hop_then_a_timeout_fail() {
    let (t, e) = topo();
    let c = chaos_on(
        &t,
        "db",
        NodeChaos {
            error_rate: 1.0,
            error_kind: ErrorKind::Hang,
            ..NodeChaos::default()
        },
    );
    let out = run(&t, &e, &c, 0);
    let ChainResult::Err {
        kind, status, node, ..
    } = &out.result
    else {
        panic!()
    };
    assert_eq!(
        (*kind, *status, *node),
        (FailKind::Timeout, 504, t.id("db").unwrap())
    );
    let hang = out
        .hops
        .iter()
        .find(|h| h.node == t.id("db").unwrap() && matches!(h.kind, HopKind::Op(_)))
        .unwrap();
    assert_eq!(hang.dur, HANG_CAP, "no node timeout: the 60 s default cap");
}

#[test]
fn hang_uses_the_node_timeout_when_set() {
    let (t, e) = build(&BASE.replace(
        "db:    { type: database,",
        "db:    { type: database, timeout: 3s,",
    ));
    let c = chaos_on(
        &t,
        "db",
        NodeChaos {
            error_rate: 1.0,
            error_kind: ErrorKind::Hang,
            ..NodeChaos::default()
        },
    );
    let out = run(&t, &e, &c, 0);
    // The plan keeps the full cap, but db's span opened at the edge hop (Z6 consequence), so its
    // deadline precedes the hang's own Fail and `out.hops` is clipped there.
    let hang = out.plan.items.iter().find_map(|i| match i {
        PlanItem::Hop(h) if matches!(h.kind, HopKind::Op(_)) => Some(h),
        _ => None,
    });
    assert_eq!(hang.unwrap().dur, Duration::from_secs(3));
    let db = t.id("db").unwrap();
    let span_at = out.plan.items.iter().find_map(|i| match i {
        PlanItem::Span { node, at, .. } if *node == db => Some(*at),
        _ => None,
    });
    assert_eq!(out.total(), span_at.unwrap() + Duration::from_secs(3));
    assert!(matches!(out.result, ChainResult::Err { node, .. } if node == db));
    assert!(matches!(
        out.result,
        ChainResult::Err {
            kind: FailKind::Timeout,
            status: 504,
            ..
        }
    ));
}
