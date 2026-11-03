//! `hold: call`: a node keeps its slot while it waits on what it calls, so a slow dependency fills
//! the slots above it. Plans are built by hand: `Hold { size }` covers the next `size` items.

use crate::common::*;
use kaanal_core::replay::{Engine, FailCause};
use kaanal_core::{HopKind, Op, PlanItem};
use std::time::Duration;

/// One app slot, a queue of 5, and a database with one connection that takes 40 ms a read.
const TIERS: &str = "version: 1
nodes:
  app: { type: service, concurrency: { limit: 1, queue: 5 }, hold: call }
  db: { type: database, ops: { read: 40ms, write: 40ms, scan: 40ms }, pool: { size: 1, queue: 5 } }
edges:
  - { from: app, to: db }
";

/// The same system without the setting: the app holds its slot only while it works.
const TIERS_OWN: &str = "version: 1
nodes:
  app: { type: service, concurrency: { limit: 1, queue: 5 } }
  db: { type: database, ops: { read: 40ms, write: 40ms, scan: 40ms }, pool: { size: 1, queue: 5 } }
edges:
  - { from: app, to: db }
";

fn held(app: usize, db: usize) -> std::sync::Arc<kaanal_core::Plan> {
    // the app works 10 ms, then calls the database for 40 ms, all inside its held slot
    plan(vec![
        PlanItem::Hold {
            node: app,
            replica: 0,
            size: 2,
        },
        hop(app, HopKind::Proc, 10),
        hop(db, HopKind::Op(Op::Read), 40),
    ])
}

fn own(app: usize, db: usize) -> std::sync::Arc<kaanal_core::Plan> {
    plan(vec![
        hop(app, HopKind::Proc, 10),
        hop(db, HopKind::Op(Op::Read), 40),
    ])
}

#[test]
fn a_held_slot_makes_the_next_request_wait_for_the_whole_call() {
    let t = topo(TIERS);
    let (app, db) = (t.id("app").unwrap(), t.id("db").unwrap());
    let mut e = Engine::new(&t);
    e.submit(0, held(app, db), 0).unwrap();
    e.submit(0, held(app, db), 1).unwrap();
    let f = run_all(&mut e);
    // request 0: 10 ms of its own work and 40 ms at the database
    assert_eq!(fin(&f, 0).done, ms(50));
    // request 1 could not even start until request 0's whole call returned at 50 ms
    let r1 = fin(&f, 1);
    assert_eq!(r1.done, ms(100));
    assert_eq!(
        r1.hops[0].wait,
        Duration::from_millis(50),
        "the wait shows on its first hop"
    );
    // the app's slot was busy 100 ms of the 100 ms, not 20
    assert_eq!(e.node_counters()[app].area, u128::from(ms(100)));
}

#[test]
fn without_the_setting_only_the_database_queues() {
    let t = topo(TIERS_OWN);
    let (app, db) = (t.id("app").unwrap(), t.id("db").unwrap());
    let mut e = Engine::new(&t);
    e.submit(0, own(app, db), 0).unwrap();
    e.submit(0, own(app, db), 1).unwrap();
    let f = run_all(&mut e);
    assert_eq!(fin(&f, 0).done, ms(50));
    // the app was free again at 10 ms; request 1 waited at the database instead (until 50)
    assert_eq!(fin(&f, 1).done, ms(90));
    assert_eq!(
        e.node_counters()[app].area,
        u128::from(ms(20)),
        "busy only while working"
    );
    assert_eq!(e.node_counters()[db].max_queue, 1);
}

#[test]
fn a_full_queue_at_a_held_node_refuses_the_request_at_once() {
    let t = topo(
        "version: 1\nnodes:\n  app: { type: service, concurrency: { limit: 1, queue: 0 }, hold: call }\n  db: { type: database, flavor: postgres }\nedges:\n  - { from: app, to: db }\n",
    );
    let (app, db) = (t.id("app").unwrap(), t.id("db").unwrap());
    let mut e = Engine::new(&t);
    e.submit(0, held(app, db), 0).unwrap();
    e.submit(ms(1), held(app, db), 1).unwrap();
    let f = run_all(&mut e);
    let r1 = fin(&f, 1);
    assert_eq!(err_of(r1), (app, 503, None));
    assert_eq!(r1.cause, Some(FailCause::Overflow));
    assert_eq!(r1.done, ms(1), "refused when it arrived");
    assert_eq!(r1.hops.len(), 0);
    let c = &e.node_counters()[app];
    assert_eq!((c.overflowed, c.completed), (1, 1));
}

#[test]
fn a_failing_call_frees_the_callers_slot_at_the_failure_not_when_the_zombie_ends() {
    let t = topo(TIERS);
    let (app, db) = (t.id("app").unwrap(), t.id("db").unwrap());
    let mut e = Engine::new(&t);
    // request 0: the call to the database has a 10 ms deadline but the query takes 40 ms
    let doomed = plan(vec![
        PlanItem::Hold {
            node: app,
            replica: 0,
            size: 3,
        },
        hop(app, HopKind::Proc, 5),
        span(db, 10, 1),
        hop(db, HopKind::Op(Op::Read), 40),
    ]);
    e.submit(0, doomed, 0).unwrap();
    // request 1 queues for the app's slot at 10 ms
    e.submit(
        ms(10),
        plan(vec![
            PlanItem::Hold {
                node: app,
                replica: 0,
                size: 1,
            },
            hop(app, HopKind::Proc, 5),
        ]),
        1,
    )
    .unwrap();
    let f = run_all(&mut e);
    // the deadline fires at 15 ms (5 ms of work, then 10 ms): the app answers 504 and its worker is
    // free, though the database query keeps running until 45 ms
    let r0 = fin(&f, 0);
    assert_eq!(r0.cause, Some(FailCause::Deadline));
    assert_eq!(r0.done, ms(15));
    let r1 = fin(&f, 1);
    assert_eq!(
        r1.hops[0].wait,
        Duration::from_millis(5),
        "it got the slot at 15 ms"
    );
    assert_eq!(r1.done, ms(20));
}

#[test]
fn a_zombie_hop_of_the_held_node_keeps_the_slot_until_it_ends() {
    let t = topo(TIERS);
    let app = t.id("app").unwrap();
    let mut e = Engine::new(&t);
    // the app's own 30 ms of work is under a 10 ms deadline set on the app itself
    e.submit(
        0,
        plan(vec![
            span(app, 10, 2),
            PlanItem::Hold {
                node: app,
                replica: 0,
                size: 1,
            },
            hop(app, HopKind::Proc, 30),
        ]),
        0,
    )
    .unwrap();
    e.submit(
        0,
        plan(vec![
            PlanItem::Hold {
                node: app,
                replica: 0,
                size: 1,
            },
            hop(app, HopKind::Proc, 5),
        ]),
        1,
    )
    .unwrap();
    let f = run_all(&mut e);
    assert_eq!(fin(&f, 0).done, ms(10));
    // request 0's own hop is still running at 10 ms, so the slot is not free until it ends at 30
    let r1 = fin(&f, 1);
    assert_eq!(r1.hops[0].wait, Duration::from_millis(30));
    assert_eq!(r1.done, ms(35));
}

#[test]
fn a_hold_over_a_parallel_block_lets_go_only_after_the_last_branch() {
    let t = topo(TIERS);
    let (app, db) = (t.id("app").unwrap(), t.id("db").unwrap());
    let mut e = Engine::new(&t);
    // app works 5 ms, then two branches at the database (pool 1: they run one after the other)
    let fork = |size_a: usize| {
        plan(vec![
            PlanItem::Hold {
                node: app,
                replica: 0,
                size: size_a,
            },
            hop(app, HopKind::Proc, 5),
            PlanItem::Fork {
                branches: 2,
                size: 4,
            },
            PlanItem::Branch { size: 1 },
            hop(db, HopKind::Op(Op::Read), 40),
            PlanItem::Branch { size: 1 },
            hop(db, HopKind::Op(Op::Read), 40),
        ])
    };
    e.submit(0, fork(6), 0).unwrap();
    e.submit(0, fork(6), 1).unwrap();
    let f = run_all(&mut e);
    // request 0: 5 + 40 + 40 = 85 ms; the slot is held through the join, so request 1 starts at 85
    assert_eq!(fin(&f, 0).done, ms(85));
    assert_eq!(fin(&f, 1).hops[0].wait, Duration::from_millis(85));
    assert_eq!(fin(&f, 1).done, ms(170));
}

#[test]
fn nothing_is_left_held_when_the_run_ends() {
    let t = topo(TIERS);
    let (app, db) = (t.id("app").unwrap(), t.id("db").unwrap());
    let mut e = Engine::new(&t);
    for i in 0..20 {
        e.submit(ms(i * 3), held(app, db), i).unwrap();
    }
    let f = run_all(&mut e);
    assert_eq!(f.len(), 20);
    // a later request finds the app free: no slot leaked
    e.submit(ms(5_000), held(app, db), 99).unwrap();
    let g = run_all(&mut e);
    assert_eq!(g[0].hops[0].wait, Duration::ZERO);
    assert_eq!(g[0].done - g[0].arrival, ms(50));
}
