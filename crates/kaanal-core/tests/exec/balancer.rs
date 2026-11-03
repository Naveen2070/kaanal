//! A load balancer in the walk: it passes each call to ONE of its callees by `spread`, at its own
//! cost, and a callee with every copy down is skipped (RULES 50).

#![expect(
    clippy::many_single_char_names,
    clippy::cast_precision_loss,
    reason = "t, e, c and n are the shorthand of every exec test; the shares are counts far below 2^52"
)]

use crate::common::*;
use kaanal_core::{ChaosKey, ChaosLayer, ChaosState, Endpoints, HopKind, PlanItem, Topology};
use std::time::Duration;

/// nginx -> app -> lb -> (a | b); the route is owned by `app`, so its default body calls the
/// balancer, which does 2 ms of work and then hands the call on.
fn cfg(lb_extra: &str) -> String {
    format!(
        "version: 1\nnodes:\n  nginx: {{ type: gateway, proc: 1ms }}\n  app: {{ type: service, proc: 5ms }}\n  lb: {{ type: load-balancer, proc: 2ms{lb_extra} }}\n  a: {{ type: service, proc: 10ms }}\n  b: {{ type: service, proc: 20ms }}\nedges:\n  - {{ from: nginx, to: app, net: 1ms }}\n  - {{ from: app, to: lb, net: 1ms }}\n  - {{ from: lb, to: a, net: 3ms }}\n  - {{ from: lb, to: b, net: 4ms }}\n"
    )
}

fn world(lb_extra: &str) -> (Topology, Endpoints) {
    build(&cfg(lb_extra))
}

/// The callee request `n` went to: the one of a and b with a `proc` hop.
fn target(t: &Topology, e: &Endpoints, c: &ChaosState, n: u64) -> Option<&'static str> {
    let (a, b) = (t.id("a").unwrap(), t.id("b").unwrap());
    let mut hit = None;
    for item in run(t, e, c, n).plan.items {
        if let PlanItem::Hop(h) = item
            && h.kind == HopKind::Proc
        {
            if h.node == a {
                assert!(hit.is_none(), "one callee only");
                hit = Some("a");
            } else if h.node == b {
                assert!(hit.is_none(), "one callee only");
                hit = Some("b");
            }
        }
    }
    hit
}

#[test]
fn a_balancer_does_its_own_work_then_one_callee_runs() {
    let (t, e) = world("");
    let lb = t.id("lb").unwrap();
    let out = run(&t, &e, &ChaosState::default(), 0);
    let hops: Vec<_> = out
        .plan
        .items
        .iter()
        .filter_map(|i| match i {
            PlanItem::Hop(h) => Some(h),
            _ => None,
        })
        .collect();
    let proc_at = |name: &str| {
        hops.iter()
            .find(|h| h.node == t.id(name).unwrap() && h.kind == HopKind::Proc)
            .map(|h| (h.start, h.dur))
    };
    // nginx 1 + net 1 + app 5 + net 1 = lb starts at 8 ms, works 2 ms
    assert_eq!(proc_at("lb"), Some((ms(8), ms(2))));
    assert!(hops.iter().any(|h| h.node == lb));
    let (a, b) = (proc_at("a"), proc_at("b"));
    assert!(a.is_some() ^ b.is_some(), "exactly one callee: {a:?} {b:?}");
    // the wire to the chosen callee is that edge's net (3 ms to a, 4 ms to b), after lb's work
    let (start, dur) = a.or(b).unwrap();
    let net = if a.is_some() { 3 } else { 4 };
    assert_eq!(start, ms(10 + net), "lb ends at 10 ms, then the wire");
    assert_eq!(dur, if a.is_some() { ms(10) } else { ms(20) });
}

#[test]
fn round_robin_alternates_between_the_callees() {
    let (t, e) = world("");
    let got: Vec<_> = (0..6)
        .map(|n| target(&t, &e, &ChaosState::default(), n).unwrap())
        .collect();
    let (a, b) = (
        got.iter().filter(|x| **x == "a").count(),
        got.iter().filter(|x| **x == "b").count(),
    );
    assert_eq!((a, b), (3, 3), "{got:?}");
    assert!(
        got.windows(2).all(|w| w[0] != w[1]),
        "strictly alternating: {got:?}"
    );
}

#[test]
fn weighted_follows_the_weights_per_wire() {
    let (t, e) = world(", spread: { weighted: [3, 1] }");
    let n = 4_000u64;
    let a = (0..n)
        .filter(|i| target(&t, &e, &ChaosState::default(), *i) == Some("a"))
        .count();
    let share = a as f64 / n as f64;
    assert!((share - 0.75).abs() < 0.05, "a took {share}");
    let (t, e) = world(", spread: { weighted: [0, 1] }");
    assert!((0..50).all(|i| target(&t, &e, &ChaosState::default(), i) == Some("b")));
}

#[test]
fn random_spreads_and_the_same_request_always_goes_the_same_way() {
    let (t, e) = world(", spread: random");
    let n = 4_000u64;
    let a = (0..n)
        .filter(|i| target(&t, &e, &ChaosState::default(), *i) == Some("a"))
        .count();
    assert!((a as f64 / n as f64 - 0.5).abs() < 0.05, "a took {a}");
    assert_eq!(
        target(&t, &e, &ChaosState::default(), 17),
        target(&t, &e, &ChaosState::default(), 17)
    );
}

fn down(t: &Topology, node: &str) -> ChaosState {
    let mut s = ChaosState::default();
    let l = ChaosLayer {
        down: Some(true),
        ..ChaosLayer::default()
    };
    s.set(ChaosKey::Replica(t.id(node).unwrap(), 0), l);
    s
}

#[test]
fn a_callee_with_no_live_copy_is_skipped_and_none_left_is_a_503() {
    let (t, e) = world("");
    let c = down(&t, "a");
    assert!((0..20).all(|n| target(&t, &e, &c, n) == Some("b")));
    let mut both = down(&t, "a");
    let l = ChaosLayer {
        down: Some(true),
        ..ChaosLayer::default()
    };
    both.set(ChaosKey::Replica(t.id("b").unwrap(), 0), l);
    let out = run(&t, &e, &both, 0);
    let fail = out.plan.items.iter().find_map(|i| match i {
        PlanItem::Fail { node, status, .. } => Some((*node, *status)),
        _ => None,
    });
    assert_eq!(fail, Some((t.id("lb").unwrap(), 503)));
}

#[test]
fn the_balancers_copies_and_its_callee_pick_are_independent() {
    // two copies of the balancer must not lock the callee choice to the copy choice
    let (t, e) = world(", replicas: 2");
    let lb = t.id("lb").unwrap();
    let mut seen = std::collections::BTreeSet::new();
    for n in 0..40 {
        let rep = run(&t, &e, &ChaosState::default(), n)
            .plan
            .items
            .iter()
            .find_map(|i| match i {
                PlanItem::Hop(h) if h.node == lb && h.kind == HopKind::Proc => Some(h.replica),
                _ => None,
            })
            .unwrap();
        seen.insert((rep, target(&t, &e, &ChaosState::default(), n).unwrap()));
    }
    assert_eq!(seen.len(), 4, "every copy x callee pair occurs: {seen:?}");
}

#[test]
fn expected_visits_split_by_the_spread() {
    let (t, e) = world(", spread: { weighted: [3, 1] }");
    let v = kaanal_core::expected_visits(&t, e.get("GET /users/{id}").unwrap());
    let at = |n: &str| v[t.id(n).unwrap()];
    assert!(
        (at("lb") - 1.0).abs() < 1e-9,
        "every request passes the balancer"
    );
    assert!((at("a") - 0.75).abs() < 1e-9 && (at("b") - 0.25).abs() < 1e-9);
    let (t, e) = world("");
    let v = kaanal_core::expected_visits(&t, e.get("GET /users/{id}").unwrap());
    assert!((v[t.id("a").unwrap()] - 0.5).abs() < 1e-9);
}

#[test]
fn a_chain_may_call_a_balancer_by_name() {
    // the owner's own chain: app does its work, then calls lb (which has the one pick)
    let src = cfg("").replace(
        "edges:",
        "endpoints:
  \"GET /users/{id}\": { chain: [ lb, lb ] }
edges:",
    );
    let (t, e) = build(&src);
    let out = run(&t, &e, &ChaosState::default(), 0);
    let nodes: Vec<_> = out
        .plan
        .items
        .iter()
        .filter_map(|i| match i {
            PlanItem::Hop(h) if h.kind == HopKind::Proc => Some(t.nodes[h.node].name.as_str()),
            _ => None,
        })
        .collect();
    // nginx, app, then two passes through the balancer, each ending in one callee
    assert_eq!(nodes.len(), 6, "{nodes:?}");
    assert_eq!((nodes[2], nodes[4]), ("lb", "lb"), "{nodes:?}");
    assert!(
        matches!(nodes[3], "a" | "b") && matches!(nodes[5], "a" | "b"),
        "{nodes:?}"
    );
    assert_ne!(
        nodes[3], nodes[5],
        "round-robin: the second pass takes the other callee"
    );
}

/// nginx -> lb -> app (-> db): the route's owner has no wire from the gateway, only from lb.
const BEHIND: &str = "version: 1\nnodes:\n  nginx: { type: gateway, proc: 1ms }\n  lb: { type: load-balancer, proc: 2ms, concurrency: 8 }\n  app: { type: service, proc: 5ms }\nedges:\n  - { from: nginx, to: lb, net: 1ms }\n  - { from: lb, to: app, net: 3ms }\n";

fn procs(t: &Topology, out: &kaanal_core::ChainOutcome) -> Vec<(String, Duration, Duration)> {
    out.plan
        .items
        .iter()
        .filter_map(|i| match i {
            PlanItem::Hop(h) if h.kind == HopKind::Proc => {
                Some((t.nodes[h.node].name.clone(), h.start, h.dur))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn a_route_whose_owner_is_behind_a_balancer_crosses_it() {
    let (t, e) = build(BEHIND);
    let got = procs(&t, &run(&t, &e, &ChaosState::default(), 0));
    // nginx 1 ms, net 1, lb 2 ms (from 2), net 3, app 5 ms (from 7)
    assert_eq!(
        got,
        [
            ("nginx".to_string(), ms(0), ms(1)),
            ("lb".to_string(), ms(2), ms(2)),
            ("app".to_string(), ms(7), ms(5)),
        ]
    );
}

#[test]
fn the_balancer_on_a_route_has_its_own_capacity_and_two_in_a_row_both_count() {
    let two = BEHIND
        .replace(
            "  app: { type: service, proc: 5ms }",
            "  lb2: { type: load-balancer, proc: 1ms }\n  app: { type: service, proc: 5ms }",
        )
        .replace(
            "  - { from: lb, to: app, net: 3ms }",
            "  - { from: lb, to: lb2, net: 1ms }\n  - { from: lb2, to: app, net: 1ms }",
        );
    let (t, e) = build(&two);
    let got: Vec<_> = procs(&t, &run(&t, &e, &ChaosState::default(), 0))
        .into_iter()
        .map(|(n, ..)| n)
        .collect();
    assert_eq!(got, ["nginx", "lb", "lb2", "app"]);
    // the load shows on every balancer in the way
    let v = kaanal_core::expected_visits(&t, e.get("GET /users/{id}").unwrap());
    for n in ["lb", "lb2", "app"] {
        assert!((v[t.id(n).unwrap()] - 1.0).abs() < 1e-9, "{n}");
    }
}

#[test]
fn a_direct_wire_to_the_owner_is_used_when_there_is_one() {
    let direct = BEHIND.replace(
        "edges:\n",
        "edges:\n  - { from: nginx, to: app, net: 1ms }\n",
    );
    let (t, e) = build(&direct);
    let got: Vec<_> = procs(&t, &run(&t, &e, &ChaosState::default(), 0))
        .into_iter()
        .map(|(n, ..)| n)
        .collect();
    assert_eq!(got, ["nginx", "app"], "the balancer is not on this route");
}
