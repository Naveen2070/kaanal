//! Timeout spans: zombies, deadlines, parallel and nested spans, ties and hand-over instants.

use crate::common::*;
use kaanal_core::replay::{Engine, FailCause, Finished};
use kaanal_core::{ChainResult, FailKind, HopKind, NodeId, Op, PlanItem};
use std::time::Duration;

#[test]
fn zombie_hop_keeps_the_slot_and_waiters_time_out_in_the_queue() {
    // spec section 1 criterion 2
    let t = topo(POOL1);
    let db = t.id("db").unwrap();
    let mk = || plan(vec![span(db, 500, 1), hop(db, HopKind::Op(Op::Read), 400)]);
    let mut e = Engine::new(&t);
    for _ in 0..3 {
        e.submit(0, mk(), 0).unwrap();
    }
    e.submit(600 * MS, mk(), 0).unwrap();
    let f = run_all(&mut e);
    assert_eq!(f.len(), 4);
    assert_eq!(fin(&f, 0).result, ChainResult::Ok);
    assert_eq!(fin(&f, 0).done, 400 * MS);
    // request 1: got the slot at 400, times out while running at 500, hop is a zombie until 800
    let r1 = fin(&f, 1);
    assert_eq!(r1.done, 500 * MS);
    assert!(matches!(
        r1.result,
        ChainResult::Err {
            kind: FailKind::Timeout,
            status: 504,
            ..
        }
    ));
    assert_eq!(
        r1.hops[0].hop.dur,
        Duration::from_millis(100),
        "client view is cut at the deadline"
    );
    // request 2: leaves the queue at 500
    let r2 = fin(&f, 2);
    assert_eq!(r2.done, 500 * MS);
    assert_eq!(r2.hops.len(), 0);
    // request 3 arrives at 600, waits for the zombie until 800, deadline at 1100 hits it mid-run
    let r3 = fin(&f, 3);
    assert_eq!(r3.done, 1100 * MS);
    assert!(matches!(
        r3.result,
        ChainResult::Err {
            kind: FailKind::Timeout,
            ..
        }
    ));
    assert_eq!(r3.hops[0].wait, Duration::from_millis(200));
    let c = &e.node_counters()[db];
    assert_eq!(c.timed_out, 3);
    // r0, r1 and r3 started a hop; r2 left the queue unserved
    assert_eq!(c.completed, 3);
    // waits: r0 0, r1 400, r3 200
    assert_eq!(c.wait, u128::from(600 * MS));
}

#[test]
fn finishing_exactly_at_the_deadline_is_ok() {
    let t = topo(POOL1);
    let db = t.id("db").unwrap();
    let mut e = Engine::new(&t);
    e.submit(
        0,
        plan(vec![span(db, 400, 1), hop(db, HopKind::Op(Op::Read), 400)]),
        0,
    )
    .unwrap();
    let f = run_all(&mut e);
    assert_eq!(f[0].result, ChainResult::Ok);
    assert_eq!(f[0].done, 400 * MS);
}

#[test]
fn a_planned_fail_at_the_deadline_instant_beats_the_deadline() {
    let t = topo(POOL1);
    let db = t.id("db").unwrap();
    let mut e = Engine::new(&t);
    e.submit(
        0,
        plan(vec![
            span(db, 400, 2),
            hop(db, HopKind::Op(Op::Read), 400),
            PlanItem::Fail {
                node: db,
                status: 500,
                kind: FailKind::Status,
                at: Duration::from_millis(400),
            },
        ]),
        0,
    )
    .unwrap();
    let f = run_all(&mut e);
    assert!(
        matches!(
            f[0].result,
            ChainResult::Err {
                status: 500,
                kind: FailKind::Status,
                ..
            }
        ),
        "{:?}",
        f[0].result
    );
}

#[test]
fn span_ending_in_a_parallel_block_stays_open() {
    // review B3: the fork moves the parent token to the block end at once; the span must not close there
    let t = topo(POOL1);
    let gw = t.id("gw").unwrap();
    let items = vec![
        span(gw, 300, 5),
        PlanItem::Fork {
            branches: 2,
            size: 4,
        },
        PlanItem::Branch { size: 1 },
        hop(gw, HopKind::Proc, 500),
        PlanItem::Branch { size: 1 },
        hop(gw, HopKind::Proc, 500),
    ];
    let mut e = Engine::new(&t);
    e.submit(0, plan(items), 0).unwrap();
    let f = run_all(&mut e);
    assert_eq!(f[0].done, 300 * MS);
    assert!(matches!(
        f[0].result,
        ChainResult::Err {
            kind: FailKind::Timeout,
            ..
        }
    ));
}

#[test]
fn stale_deadline_never_hits_a_recycled_request_slot() {
    // review B2: A finishes at 100 ms, its table slot is reused by B at 150 ms,
    // A's deadline (300 ms) pops while B runs and must be ignored.
    let t = topo(POOL1);
    let gw = t.id("gw").unwrap();
    let mut e = Engine::new(&t);
    e.submit(
        0,
        plan(vec![span(gw, 300, 1), hop(gw, HopKind::Proc, 100)]),
        0,
    )
    .unwrap();
    e.advance_to(150 * MS).unwrap();
    // B has its own span (a different deadline seq), so a stale deadline cannot match it by accident
    e.submit(
        150 * MS,
        plan(vec![span(gw, 5000, 1), hop(gw, HopKind::Proc, 1000)]),
        1,
    )
    .unwrap();
    let f = run_all(&mut e);
    let b = f.iter().find(|x| x.tag == 1).unwrap();
    assert_eq!(b.result, ChainResult::Ok);
    assert_eq!(b.done, 1150 * MS);
    assert_eq!(e.table_len(), 1, "B reused A's slot");
}

#[test]
fn closed_span_ignores_its_deadline() {
    let t = topo(POOL1);
    let gw = t.id("gw").unwrap();
    let mut e = Engine::new(&t);
    e.submit(
        0,
        plan(vec![
            span(gw, 300, 1),
            hop(gw, HopKind::Proc, 100),
            hop(gw, HopKind::Proc, 500),
        ]),
        0,
    )
    .unwrap();
    let f = run_all(&mut e);
    assert_eq!(
        f[0].result,
        ChainResult::Ok,
        "the span ended at 100 ms; the later hop is outside it"
    );
    assert_eq!(f[0].done, 600 * MS);
}

#[test]
fn nested_spans_outer_wins_at_equal_deadlines() {
    let t = topo(POOL1);
    let (gw, db) = (t.id("gw").unwrap(), t.id("db").unwrap());
    let mut e = Engine::new(&t);
    e.submit(
        0,
        plan(vec![
            span(gw, 200, 2),
            span(db, 200, 1),
            hop(db, HopKind::Op(Op::Read), 400),
        ]),
        0,
    )
    .unwrap();
    let f = run_all(&mut e);
    let ChainResult::Err { node, .. } = f[0].result else {
        panic!()
    };
    assert_eq!(
        node, gw,
        "equal deadlines: the open span with the lowest plan index (the outer one) fires (Z14)"
    );
}

#[test]
fn deadline_inside_a_parallel_branch_cancels_the_siblings() {
    let t = topo(POOL1);
    let gw = t.id("gw").unwrap();
    let items = vec![
        PlanItem::Fork {
            branches: 2,
            size: 5,
        },
        PlanItem::Branch { size: 2 },
        span(gw, 100, 1),
        hop(gw, HopKind::Proc, 500),
        PlanItem::Branch { size: 1 },
        hop(gw, HopKind::Proc, 500),
    ];
    let mut e = Engine::new(&t);
    e.submit(0, plan(items), 0).unwrap();
    let f = run_all(&mut e);
    assert_eq!(f[0].done, 100 * MS);
    assert_eq!(f[0].cause, Some(FailCause::Deadline));
    assert!(
        matches!(
            f[0].result,
            ChainResult::Err { node, kind: FailKind::Timeout, status: 504, .. } if node == gw
        ),
        "{:?}",
        f[0].result
    );
    assert_eq!(f[0].hops.len(), 2);
    assert!(
        f[0].hops
            .iter()
            .all(|h| h.hop.dur <= Duration::from_millis(100)),
        "sibling cut at the failure"
    );
}

pub(crate) const THREE: &str =
    "version: 1\nnodes:\n  a: { type: service }\n  b: { type: service }\n  c: { type: service }\n";

pub(crate) fn timeout_node(f: &Finished) -> NodeId {
    match f.result {
        ChainResult::Err {
            node,
            kind: FailKind::Timeout,
            ..
        } => node,
        ref r => panic!("expected a timeout, got {r:?}"),
    }
}

#[test]
fn equal_deadlines_in_two_branches_fail_at_the_lower_plan_index() {
    // Z14 / `execute`: the lower plan index wins, however the spans were opened in time.
    let t = topo(THREE);
    let [na, nb, nc] = ["a", "b", "c"].map(|n| t.id(n).unwrap());
    // b (index 3) opens at 50 ms, c (index 6) at 0: c is stamped first, both fall at 100 ms
    let c_first = vec![
        PlanItem::Fork {
            branches: 2,
            size: 7,
        },
        PlanItem::Branch { size: 3 },
        hop(na, HopKind::Proc, 50),
        span(nb, 50, 1),
        hop(nb, HopKind::Proc, 200),
        PlanItem::Branch { size: 2 },
        span(nc, 100, 1),
        hop(nc, HopKind::Proc, 200),
    ];
    // b (index 2) opens at 0 and is stamped first; c (index 6) opens at 50
    let b_first = vec![
        PlanItem::Fork {
            branches: 2,
            size: 7,
        },
        PlanItem::Branch { size: 2 },
        span(nb, 100, 1),
        hop(nb, HopKind::Proc, 200),
        PlanItem::Branch { size: 3 },
        hop(na, HopKind::Proc, 50),
        span(nc, 50, 1),
        hop(nc, HopKind::Proc, 200),
    ];
    for items in [c_first, b_first] {
        let mut e = Engine::new(&t);
        e.submit(0, plan(items), 0).unwrap();
        let f = run_all(&mut e);
        assert_eq!(f[0].done, 100 * MS);
        assert_eq!(timeout_node(&f[0]), nb);
        assert_eq!(e.node_counters()[nb].timed_out, 1);
        assert_eq!(e.node_counters()[nc].timed_out, 0);
    }
}

#[test]
fn a_deadline_at_the_hand_over_instant_leaves_a_zombie_that_the_client_never_sees() {
    // r1 waits exactly its 400 ms: r0's hop-done (a normal event) runs first and hands r1 the
    // slot, then the deadline fires. The hop started at the failure instant, so it is absent.
    let t = topo(POOL1);
    let db = t.id("db").unwrap();
    let read = || hop(db, HopKind::Op(Op::Read), 400);
    let mut e = Engine::new(&t);
    e.submit(0, plan(vec![span(db, 2000, 1), read()]), 0)
        .unwrap();
    e.submit(0, plan(vec![span(db, 400, 1), read()]), 1)
        .unwrap();
    e.submit(0, plan(vec![span(db, 2000, 1), read()]), 2)
        .unwrap();
    let f = run_all(&mut e);
    let r1 = fin(&f, 1);
    assert_eq!(timeout_node(r1), db);
    assert_eq!(r1.done, 400 * MS);
    assert_eq!(r1.hops.len(), 0, "started at the failure instant: absent");
    // the zombie held the slot until 800, so r2 starts then
    let r2 = fin(&f, 2);
    assert_eq!(r2.result, ChainResult::Ok);
    assert_eq!(r2.done, 1200 * MS);
    assert_eq!(r2.hops[0].wait, Duration::from_millis(800));
    assert_eq!(e.node_counters()[db].completed, 3);
    assert_eq!(e.node_counters()[db].timed_out, 1);
}

#[test]
fn a_deadline_over_a_delay_hop_cuts_it() {
    let t = topo(POOL1);
    let gw = t.id("gw").unwrap();
    let mut e = Engine::new(&t);
    e.submit(
        0,
        plan(vec![span(gw, 100, 1), hop(gw, HopKind::Delay, 500)]),
        0,
    )
    .unwrap();
    let f = run_all(&mut e);
    assert_eq!(timeout_node(&f[0]), gw);
    assert_eq!(f[0].done, 100 * MS);
    assert_eq!(f[0].hops[0].hop.dur, Duration::from_millis(100));
    assert_eq!(e.node_counters()[gw].timed_out, 1);
}

#[test]
fn a_deadline_with_one_sibling_queued_and_another_running() {
    let t = topo(POOL1);
    let (gw, db) = (t.id("gw").unwrap(), t.id("db").unwrap());
    let mut e = Engine::new(&t);
    // a blocker holds the only db slot until 400 ms
    e.submit(0, plan(vec![hop(db, HopKind::Op(Op::Read), 400)]), 0)
        .unwrap();
    let items = vec![
        span(gw, 200, 5),
        PlanItem::Fork {
            branches: 2,
            size: 4,
        },
        PlanItem::Branch { size: 1 },
        hop(gw, HopKind::Proc, 500),
        PlanItem::Branch { size: 1 },
        hop(db, HopKind::Op(Op::Read), 400),
    ];
    e.submit(0, plan(items), 1).unwrap();
    let f = run_all(&mut e);
    let r = fin(&f, 1);
    assert_eq!(timeout_node(r), gw);
    assert_eq!(r.done, 200 * MS);
    assert_eq!(r.hops.len(), 1, "the queued sibling never started");
    assert_eq!(r.hops[0].hop.dur, Duration::from_millis(200));
    // the cancelled waiter did not take the slot when the blocker released it
    assert_eq!(e.node_counters()[db].completed, 1);
    assert_eq!(fin(&f, 0).result, ChainResult::Ok);
}

#[test]
fn timed_out_is_not_counted_for_a_pre_warmup_arrival() {
    let t = topo(POOL1);
    let db = t.id("db").unwrap();
    let mut e = Engine::new(&t);
    e.set_window(100 * MS, u64::MAX);
    let mk = || plan(vec![span(db, 50, 1), hop(db, HopKind::Op(Op::Read), 400)]);
    e.submit(0, mk(), 0).unwrap();
    e.submit(500 * MS, mk(), 1).unwrap();
    let f = run_all(&mut e);
    assert_eq!(f.len(), 1, "the pre-warmup request is not reported");
    assert_eq!(f[0].tag, 1);
    assert_eq!(timeout_node(&f[0]), db);
    assert_eq!(e.node_counters()[db].timed_out, 1);
}

#[test]
fn delay_hop_takes_no_slot_and_no_token() {
    let t = topo(
        "version: 1\nnodes:\n  gw: { type: gateway, concurrency: 1, rate_limit: { rps: 1, burst: 1 } }\n",
    );
    let gw = t.id("gw").unwrap();
    let mut e = Engine::new(&t);
    // two requests at once: each is one 50 ms delay (holds nothing)
    let mk = || plan(vec![hop(gw, HopKind::Delay, 50)]);
    e.submit(0, mk(), 0).unwrap();
    e.submit(0, mk(), 1).unwrap();
    let f = run_all(&mut e);
    assert!(
        f.iter().all(|x| x.result == ChainResult::Ok),
        "no token needed, no slot, no 429"
    );
    assert!(f.iter().all(|x| x.done == 50 * MS));
}
