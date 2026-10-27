//! `prob`: a branch taken by a share of requests, drawn once per step.

use crate::common::*;
use kaanal_core::{ChaosState, Hop, HopKind, PlanItem};

/// `BASE`-like config whose `GET /users/{id}` runs `chain`; `db` has a 1 s timeout (so a taken
/// call opens a span) and the app-to-db edge has `edge` extra fields.
fn cfg(chain: &str, edge: &str) -> String {
    format!(
        "version: 1\nnodes:\n  nginx: {{ type: gateway, proc: 1ms }}\n  app: {{ type: service, proc: {{ p50: 5ms, p99: 20ms }} }}\n  db: {{ type: database, timeout: 1s, ops: {{ read: {{ p50: 4ms, p99: 20ms }}, write: 8ms, scan: 30ms }} }}\nedges:\n  - {{ from: nginx, to: app, net: 1ms }}\n  - {{ from: app, to: db, net: {{ p50: 1ms, p99: 2ms }}{edge} }}\nendpoints:\n  \"GET /users/{{id}}\": {{ chain: {chain} }}\n"
    )
}

/// The plan of request `n` as every hop, in order.
fn hops(t: &kaanal_core::Topology, e: &kaanal_core::Endpoints, n: u64) -> Vec<Hop> {
    run(t, e, &ChaosState::default(), n)
        .plan
        .items
        .into_iter()
        .filter_map(|i| match i {
            PlanItem::Hop(h) => Some(h),
            _ => None,
        })
        .collect()
}

fn db_net(h: &[Hop], t: &kaanal_core::Topology) -> usize {
    let db = t.id("db").unwrap();
    h.iter()
        .filter(|h| h.node == db && h.kind == HopKind::Net)
        .count()
}

#[test]
fn prob_one_changes_no_plan() {
    let (t0, e0) = build(&cfg("[ db, db ]", ""));
    let (t1, e1) = build(&cfg("[ { call: db, prob: 1 }, db ]", ", prob: 1"));
    for n in 0..200 {
        assert_eq!(hops(&t0, &e0, n), hops(&t1, &e1, n), "n = {n}");
    }
}

#[test]
fn prob_half_takes_about_half() {
    let (t, e) = build(&cfg("[ { call: db, prob: 0.5 } ]", ""));
    let taken = (0..4000)
        .filter(|n| db_net(&hops(&t, &e, *n), &t) == 1)
        .count();
    assert!((1800..=2200).contains(&taken), "{taken} of 4000");
}

#[test]
fn an_edge_prob_is_the_default_and_a_step_prob_wins() {
    let (t, e) = build(&cfg("[ db ]", ", prob: 0.25"));
    let edge_only = (0..4000)
        .filter(|n| db_net(&hops(&t, &e, *n), &t) == 1)
        .count();
    assert!((800..=1200).contains(&edge_only), "{edge_only} of 4000");
    let (t, e) = build(&cfg("[ { call: db, prob: 1 } ]", ", prob: 0.25"));
    assert!(
        (0..200).all(|n| db_net(&hops(&t, &e, n), &t) == 1),
        "the step says 1"
    );
}

#[test]
fn one_draw_covers_all_repeats() {
    let (t, e) = build(&cfg("[ { call: db, repeat: 3, prob: 0.5 } ]", ""));
    let counts: Vec<usize> = (0..400).map(|n| db_net(&hops(&t, &e, n), &t)).collect();
    assert!(counts.iter().all(|c| *c == 0 || *c == 3), "{counts:?}");
    assert!(counts.contains(&0) && counts.contains(&3));
}

#[test]
fn a_skipped_call_leaves_nothing_and_shifts_no_other_draw() {
    let (t, skip) = build(&cfg("[ { call: db, prob: 0.5 }, db ]", ""));
    let (_, full) = build(&cfg("[ { call: db, prob: 1 }, db ]", ""));
    let mut skipped = 0;
    for n in 0..400 {
        let (a, b) = (hops(&t, &skip, n), hops(&t, &full, n));
        if db_net(&a, &t) == 2 {
            assert_eq!(a, b, "taken: the same plan as prob 1, n = {n}");
            continue;
        }
        skipped += 1;
        let out = run(&t, &skip, &ChaosState::default(), n);
        let spans = out
            .plan
            .items
            .iter()
            .filter(|i| matches!(i, PlanItem::Span { .. }))
            .count();
        assert_eq!(spans, 1, "only the second call's db span, n = {n}");
        // the surviving call is the second one: its draws are unmoved (same durations)
        let tail = |h: &[Hop]| h[h.len() - 2..].iter().map(|h| h.dur).collect::<Vec<_>>();
        assert_eq!(tail(&a), tail(&b), "n = {n}");
    }
    assert!(skipped > 100, "{skipped}");
}

#[test]
fn a_prob_call_in_a_parallel_branch_keeps_the_plan_consistent() {
    let (t, e) = build(&cfg(
        "[ { parallel: [ { call: db, prob: 0.5 }, db ] } ]",
        "",
    ));
    for n in 0..300 {
        let out = run(&t, &e, &ChaosState::default(), n);
        let items = &out.plan.items;
        let at = items
            .iter()
            .position(|i| matches!(i, PlanItem::Fork { .. }))
            .unwrap();
        let PlanItem::Fork { size, .. } = items[at] else {
            unreachable!()
        };
        assert_eq!(at + 1 + size, items.len(), "fork covers the rest, n = {n}");
        let mut b = at + 1;
        while b < items.len() {
            let PlanItem::Branch { size } = items[b] else {
                panic!("a branch marker at {b}")
            };
            b += 1 + size;
        }
        assert_eq!(b, items.len());
        assert!(out.total() > std::time::Duration::ZERO);
    }
    // the probabilistic branch is really skipped for some requests and taken for others
    let db_calls: Vec<usize> = (0..300).map(|n| db_net(&hops(&t, &e, n), &t)).collect();
    assert!(
        db_calls.contains(&1) && db_calls.contains(&2),
        "{db_calls:?}"
    );
}
