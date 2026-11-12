//! Replicas in the engine: each replica has its own slots, queue and rate limit.

use crate::common::*;
use kaanal_core::replay::Engine;
use kaanal_core::{ChainResult, FailKind, Hop, HopKind, NodeId, PlanItem};
use std::time::Duration;

const TWO: &str = "version: 1\nnodes:\n  app: { type: service, concurrency: 1, replicas: 2 }\n";
const LIMITED2: &str =
    "version: 1\nnodes:\n  gw: { type: gateway, rate_limit: { rps: 1, burst: 1 }, replicas: 2 }\n";

/// A hop of `ms` on `replica` of `node`.
fn on(node: NodeId, replica: u16, kind: HopKind, ms: u64) -> PlanItem {
    PlanItem::Hop(Hop {
        node,
        replica,
        kind,
        start: Duration::ZERO,
        dur: Duration::from_millis(ms),
    })
}

/// Submits one single-hop plan per replica in `which` at t = 0 and runs them out, by tag.
fn run_on(
    t: &kaanal_core::Topology,
    which: &[u16],
) -> (Engine, Vec<kaanal_core::replay::Finished>) {
    let app = t.id("app").unwrap();
    let mut e = Engine::new(t);
    for (tag, r) in which.iter().enumerate() {
        e.submit(0, plan(vec![on(app, *r, HopKind::Proc, 10)]), tag as u64)
            .unwrap();
    }
    let mut done = run_all(&mut e);
    done.sort_by_key(|f| f.tag);
    (e, done)
}

#[test]
fn each_replica_has_its_own_slots() {
    let t = topo(TWO);
    // different replicas run at once; the same replica queues
    let (_, done) = run_on(&t, &[0, 1]);
    assert_eq!((done[0].done, done[1].done), (ms(10), ms(10)));
    let (_, done) = run_on(&t, &[0, 0]);
    assert_eq!((done[0].done, done[1].done), (ms(10), ms(20)));
    assert_eq!(done[1].hops[0].wait, Duration::from_millis(10));
}

#[test]
fn rate_limit_is_per_replica() {
    let t = topo(LIMITED2);
    let gw = t.id("gw").unwrap();
    let run = |which: [u16; 2]| {
        let mut e = Engine::new(&t);
        for (tag, r) in which.iter().enumerate() {
            e.submit(0, plan(vec![on(gw, *r, HopKind::Proc, 1)]), tag as u64)
                .unwrap();
        }
        let mut done = run_all(&mut e);
        done.sort_by_key(|f| f.tag);
        done
    };
    let done = run([0, 1]);
    assert!(
        done.iter().all(|f| f.result == ChainResult::Ok),
        "one token each"
    );
    let done = run([0, 0]);
    assert_eq!(done[0].result, ChainResult::Ok);
    assert_eq!(
        err_of(&done[1]).1,
        429,
        "the second request on one replica finds its bucket empty"
    );
}

#[test]
fn node_counters_sum_the_replicas_and_replica_counters_split_them() {
    let t = topo(TWO);
    let app = t.id("app").unwrap();
    let (e, _) = run_on(&t, &[0, 0, 0, 1, 1]);
    let node = &e.node_counters()[app];
    let per = &e.replica_counters()[app];
    assert_eq!(per.len(), 2);
    assert_eq!((per[0].completed, per[1].completed), (3, 2));
    assert_eq!(node.completed, 5);
    assert_eq!((per[0].max_queue, per[1].max_queue), (2, 1));
    assert_eq!(node.max_queue, 2, "the longest queue of any replica");
    assert_eq!(node.wait, per[0].wait + per[1].wait);
    assert_eq!(node.wait, u128::from(ms(10) + ms(20) + ms(10)));
}

#[test]
fn busy_time_sums_over_the_replicas() {
    let t = topo(TWO);
    let app = t.id("app").unwrap();
    let app_plan = |r| plan(vec![on(app, r, HopKind::Proc, 10)]);
    let mut e = Engine::new(&t);
    e.set_window(0, ms(100));
    e.submit(0, app_plan(0), 0).unwrap();
    e.submit(0, app_plan(1), 1).unwrap();
    run_all(&mut e);
    let per = &e.replica_counters()[app];
    assert_eq!(
        (per[0].area, per[1].area),
        (u128::from(ms(10)), u128::from(ms(10)))
    );
    assert_eq!(e.node_counters()[app].area, u128::from(ms(20)));
}

#[test]
fn a_failed_request_leaves_the_queue_of_its_replica() {
    let t = topo(TWO);
    let app = t.id("app").unwrap();
    let mut e = Engine::new(&t);
    // A holds replica 1 for 100 ms.
    e.submit(0, plan(vec![on(app, 1, HopKind::Proc, 100)]), 0)
        .unwrap();
    // B: one branch queues behind A on replica 1, the other fails at 5 ms and cancels it.
    let b = plan(vec![
        PlanItem::Fork {
            branches: 2,
            size: 5,
        },
        PlanItem::Branch { size: 1 },
        on(app, 1, HopKind::Proc, 50),
        PlanItem::Branch { size: 2 },
        on(app, 0, HopKind::Net, 5),
        PlanItem::Fail {
            node: app,
            status: 500,
            kind: FailKind::Status,
            at: Duration::from_millis(5),
        },
    ]);
    e.submit(0, b, 1).unwrap();
    e.advance_to(ms(200)).unwrap();
    // C arrives long after: replica 1 must be free, with no ghost waiter holding its slot
    e.submit(ms(200), plan(vec![on(app, 1, HopKind::Proc, 10)]), 2)
        .unwrap();
    let mut done = run_all(&mut e);
    done.sort_by_key(|f| f.tag);
    assert_eq!(err_of(&done[1]).1, 500);
    assert_eq!(done[2].done, ms(210));
    assert_eq!(done[2].hops[0].wait, Duration::ZERO);
    assert_eq!(e.replica_counters()[app][1].max_queue, 1);
}

#[test]
fn copy_loads_split_a_nodes_slots_and_queue_by_copy() {
    let t = topo(TWO);
    let app = t.id("app").unwrap();
    let mut e = Engine::new(&t);
    // one slot a copy: copy 0 runs one hop and queues a second, copy 1 runs one
    for (tag, r) in [0u16, 0, 1].into_iter().enumerate() {
        e.submit(0, plan(vec![on(app, r, HopKind::Proc, 10)]), tag as u64)
            .unwrap();
    }
    e.advance_to(5 * MS).unwrap();
    assert_eq!(e.copy_loads(), vec![vec![(1, 1), (1, 0)]]);
    // the node's own figure is their sum
    assert_eq!(e.node_loads(), vec![(2, 1)]);
    run_all(&mut e);
    assert_eq!(e.copy_loads(), vec![vec![(0, 0), (0, 0)]]);
}
