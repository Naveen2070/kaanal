//! Rate limits and queue overflow: tokens, visits, planned and engine failures.

use crate::common::*;
use kaanal_core::replay::{Engine, FailCause};
use kaanal_core::{ChainResult, FailKind, HopKind, PlanItem};
use std::time::Duration;

#[test]
fn an_empty_bucket_fails_the_request_with_429_and_retry_after_and_takes_no_slot() {
    let t = topo(LIMITED);
    let gw = t.id("gw").unwrap();
    let mut e = Engine::new(&t);
    e.submit(0, plan(vec![hop(gw, HopKind::Proc, 10)]), 1)
        .unwrap();
    e.advance_to(ms(1)).unwrap();
    e.submit(ms(1), plan(vec![hop(gw, HopKind::Proc, 10)]), 2)
        .unwrap();
    let mut done = run_all(&mut e);
    done.sort_by_key(|f| f.tag);
    assert_eq!(done[0].result, ChainResult::Ok);
    assert_eq!(
        err_of(&done[1]),
        (gw, 429, Some(Duration::from_millis(999)))
    );
    assert_eq!(done[1].done, ms(1), "refused at once");
    assert!(done[1].hops.is_empty(), "the refused hop never started");
    let c = &e.node_counters()[gw];
    assert_eq!(
        (c.limited, c.completed, c.max_queue),
        (1, 1, 0),
        "it did not queue for the slot either"
    );
}

#[test]
fn a_full_queue_fails_with_the_gate_status() {
    let t = topo(
        "version: 1\nnodes:\n  app: { type: service }\n  db: { type: database, flavor: postgres, pool: { size: 1, queue: 1, overflow_status: 504 } }\nedges:\n  - { from: app, to: db }\n",
    );
    let db = t.id("db").unwrap();
    let read = HopKind::Op(kaanal_core::Op::Read);
    let mut e = Engine::new(&t);
    for tag in 0..3 {
        e.submit(0, plan(vec![hop(db, read, 10)]), tag).unwrap();
    }
    let mut done = run_all(&mut e);
    done.sort_by_key(|f| f.tag);
    assert_eq!(done[0].result, ChainResult::Ok);
    assert_eq!(done[1].result, ChainResult::Ok);
    assert_eq!(done[1].hops[0].wait, Duration::from_millis(10));
    assert_eq!(err_of(&done[2]), (db, 504, None));
    assert_eq!(done[2].done, 0);
    assert_eq!(e.node_counters()[db].overflowed, 1);
    assert_eq!(e.node_counters()[db].max_queue, 1);
}

#[test]
fn release_instant_arrival_does_not_barge_with_a_bounded_queue() {
    let t = topo(
        "version: 1\nnodes:\n  app: { type: service }\n  db: { type: database, flavor: postgres, pool: { size: 1, queue: 1 } }\nedges:\n  - { from: app, to: db }\n",
    );
    let db = t.id("db").unwrap();
    let read = HopKind::Op(kaanal_core::Op::Read);
    let mut e = Engine::new(&t);
    e.submit(0, plan(vec![hop(db, read, 10)]), 1).unwrap();
    e.submit(0, plan(vec![hop(db, read, 10)]), 2).unwrap(); // waits
    e.submit(ms(10), plan(vec![hop(db, read, 10)]), 3).unwrap(); // before the release event: queue is full
    let mut done = run_all(&mut e);
    done.sort_by_key(|f| f.tag);
    assert_eq!(
        err_of(&done[2]).1,
        503,
        "the queue still held tag 2 when tag 3 arrived"
    );
    assert_eq!(done[1].hops[0].wait, Duration::from_millis(10));
}

#[test]
fn a_visit_takes_one_token_however_many_own_hops_it_has() {
    let t = topo(
        "version: 1\nnodes:\n  m: { type: monolith, flavor: postgres, rate_limit: { rps: 1, burst: 1 } }\n",
    );
    let m = t.id("m").unwrap();
    let mut e = Engine::new(&t);
    // proc then the inline op: one visit, one token.
    e.submit(
        0,
        plan(vec![
            hop(m, HopKind::Proc, 1),
            hop(m, HopKind::Op(kaanal_core::Op::Read), 1),
        ]),
        1,
    )
    .unwrap();
    let done = run_all(&mut e);
    assert_eq!(done[0].result, ChainResult::Ok);
    assert_eq!(e.node_counters()[m].limited, 0);
}

#[test]
fn each_call_of_a_node_is_its_own_visit() {
    let t = topo(
        "version: 1\nnodes:\n  app: { type: service }\n  s: { type: external-api, rate_limit: { rps: 1, burst: 2 } }\nedges:\n  - { from: app, to: s }\n",
    );
    let s = t.id("s").unwrap();
    let call = || [hop(s, HopKind::Net, 1), hop(s, HopKind::Proc, 1)];
    let mut e = Engine::new(&t);
    // repeat: 3 → net, proc, net, proc, net, proc: three visits, a burst of 2 refuses the third.
    let items: Vec<PlanItem> = (0..3).flat_map(|_| call()).collect();
    e.submit(0, plan(items), 1).unwrap();
    let done = run_all(&mut e);
    let (node, status, _) = err_of(&done[0]);
    assert_eq!((node, status), (s, 429));
    assert_eq!(e.node_counters()[s].limited, 1);
    assert_eq!(
        done[0].hops.len(),
        5,
        "two full visits and the third visit's net hop"
    );
}

#[test]
fn parallel_branches_each_take_a_token() {
    let t = topo(
        "version: 1\nnodes:\n  app: { type: service }\n  s: { type: external-api, rate_limit: { rps: 1, burst: 1 } }\nedges:\n  - { from: app, to: s }\n",
    );
    let s = t.id("s").unwrap();
    let p = plan(vec![
        PlanItem::Fork {
            branches: 2,
            size: 6,
        },
        PlanItem::Branch { size: 2 },
        hop(s, HopKind::Net, 1),
        hop(s, HopKind::Proc, 5),
        PlanItem::Branch { size: 2 },
        hop(s, HopKind::Net, 1),
        hop(s, HopKind::Proc, 5),
    ]);
    let mut e = Engine::new(&t);
    e.submit(0, p, 1).unwrap();
    let done = run_all(&mut e);
    assert_eq!(
        err_of(&done[0]).1,
        429,
        "the second branch's visit finds the bucket empty"
    );
}

#[test]
fn a_chaos_failure_takes_no_token() {
    let t = topo("version: 1\nnodes:\n  gw: { type: gateway, rate_limit: { rps: 1, burst: 1 } }\n");
    let gw = t.id("gw").unwrap();
    let mut e = Engine::new(&t);
    e.submit(
        0,
        plan(vec![PlanItem::Fail {
            node: gw,
            status: 503,
            kind: FailKind::Status,
            at: Duration::ZERO,
        }]),
        1,
    )
    .unwrap();
    e.submit(0, plan(vec![hop(gw, HopKind::Proc, 1)]), 2)
        .unwrap();
    let mut done = run_all(&mut e);
    done.sort_by_key(|f| f.tag);
    assert_eq!(err_of(&done[0]), (gw, 503, None));
    assert_eq!(
        done[1].result,
        ChainResult::Ok,
        "the planned failure spent no token"
    );
}

#[test]
fn an_engine_failure_cancels_queued_siblings_like_a_planned_one() {
    let t = topo(
        "version: 1\nnodes:\n  app: { type: service }\n  s: { type: external-api, concurrency: 1 }\n  lim: { type: external-api, rate_limit: { rps: 1, burst: 1 } }\n",
    );
    let (s, lim) = (t.id("s").unwrap(), t.id("lim").unwrap());
    let mut e = Engine::new(&t);
    e.submit(0, plan(vec![hop(s, HopKind::Proc, 10)]), 1)
        .unwrap(); // holds `s`
    e.submit(0, plan(vec![hop(lim, HopKind::Proc, 1)]), 2)
        .unwrap(); // empties the bucket
    let fork = plan(vec![
        PlanItem::Fork {
            branches: 2,
            size: 4,
        },
        PlanItem::Branch { size: 1 },
        hop(s, HopKind::Proc, 10), // queues behind tag 1
        PlanItem::Branch { size: 1 },
        hop(lim, HopKind::Proc, 1), // refused
    ]);
    e.submit(0, fork, 3).unwrap();
    let mut done = run_all(&mut e);
    done.sort_by_key(|f| f.tag);
    assert_eq!(err_of(&done[2]).1, 429);
    assert_eq!(done[2].hops, []);
    assert_eq!(
        e.node_counters()[s].completed,
        1,
        "the queued sibling never ran"
    );
}

#[test]
fn a_planned_failure_that_replaces_a_refusal_takes_its_count_back() {
    let t = topo(
        "version: 1\nnodes:\n  app: { type: service }\n  free: { type: external-api }\n  g1: { type: external-api }\n  b3: { type: external-api, rate_limit: { rps: 1, burst: 1 } }\nedges:\n  - { from: app, to: free }\n  - { from: app, to: g1 }\n  - { from: app, to: b3 }\n",
    );
    let (app, free, g1, b3) = (
        t.id("app").unwrap(),
        t.id("free").unwrap(),
        t.id("g1").unwrap(),
        t.id("b3").unwrap(),
    );
    let mut e = Engine::new(&t);
    e.submit(0, plan(vec![hop(b3, HopKind::Proc, 1)]), 1)
        .unwrap(); // spends b3's only token
    // Branch 0 reaches its planned Fail at 3 ms (low plan index); branch 1 reaches b3 at 3 ms and is
    // refused first (its Net hop was scheduled earlier). D17: the lower index still wins.
    let p = plan(vec![
        hop(app, HopKind::Proc, 0),
        PlanItem::Fork {
            branches: 2,
            size: 8,
        },
        PlanItem::Branch { size: 4 },
        hop(free, HopKind::Net, 1),
        hop(free, HopKind::Proc, 1),
        hop(g1, HopKind::Net, 1),
        PlanItem::Fail {
            node: g1,
            status: 500,
            kind: FailKind::Status,
            at: Duration::from_millis(3),
        },
        PlanItem::Branch { size: 2 },
        hop(b3, HopKind::Net, 3),
        hop(b3, HopKind::Proc, 1),
    ]);
    e.submit(0, p, 2).unwrap();
    let mut done = run_all(&mut e);
    done.sort_by_key(|f| f.tag);
    assert_eq!(err_of(&done[1]), (g1, 500, None));
    assert_eq!(
        done[1].cause,
        Some(FailCause::Chaos),
        "the replaced rate-limit refusal leaves no RateLimit cause"
    );
    assert_eq!(done[1].done, ms(3));
    assert_eq!(
        e.node_counters()[b3].limited,
        0,
        "the refusal did not decide the answer, so it is not counted"
    );
}

#[test]
fn an_own_hop_after_a_join_starts_a_new_visit() {
    let t = topo(
        "version: 1\nnodes:\n  app: { type: service }\n  s: { type: external-api, rate_limit: { rps: 1, burst: 1 } }\n  x: { type: external-api }\nedges:\n  - { from: app, to: s }\n  - { from: app, to: x }\n",
    );
    let (lim, other) = (t.id("s").unwrap(), t.id("x").unwrap());
    let mut e = Engine::new(&t);
    // s's visit, then a parallel block whose branch calls x over an edge, then s again: a new visit.
    let p = plan(vec![
        hop(lim, HopKind::Proc, 1),
        PlanItem::Fork {
            branches: 1,
            size: 2,
        },
        PlanItem::Branch { size: 1 },
        hop(other, HopKind::Net, 1),
        hop(lim, HopKind::Proc, 1),
    ]);
    e.submit(0, p, 1).unwrap();
    let done = run_all(&mut e);
    assert_eq!(
        err_of(&done[0]).1,
        429,
        "the second visit finds the bucket empty"
    );
}
