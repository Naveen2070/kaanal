//! `least-conn` and `p2c` in the engine: the copy is chosen from the live queues when the call is
//! sent, and every hop of the visit follows it.

use crate::common::*;
use kaanal_core::replay::Engine;
use kaanal_core::{DYN_BASE, Hop, HopKind, NodeId, PickKind, PlanItem};
use std::time::Duration;

const THREE: &str = "version: 1\nnodes:\n  app: { type: service, concurrency: 1, replicas: 3 }\n";

fn on(node: NodeId, replica: u16, ms: u64) -> PlanItem {
    PlanItem::Hop(Hop {
        node,
        replica,
        kind: HopKind::Proc,
        start: Duration::ZERO,
        dur: Duration::from_millis(ms),
    })
}

fn pick(node: NodeId, kind: PickKind, cands: u64, offset: u16) -> PlanItem {
    PlanItem::Pick {
        node,
        code: DYN_BASE,
        kind,
        cands,
        offset,
        skew: None,
    }
}

/// A visit that is told to take copy `DYN_BASE` and chooses among `cands`.
fn visit(app: NodeId, kind: PickKind, cands: u64, offset: u16) -> Vec<PlanItem> {
    vec![pick(app, kind, cands, offset), on(app, DYN_BASE, 10)]
}

/// The copy the only hop of finished request `order` ran on.
fn copy_of(done: &[kaanal_core::replay::Finished], order: u64) -> u16 {
    fin(done, order).hops[0].hop.replica
}

#[test]
fn least_conn_goes_to_the_idle_copy_and_the_hop_is_recorded_on_it() {
    let t = topo(THREE);
    let app = t.id("app").unwrap();
    let mut e = Engine::new(&t);
    // copies 0 and 1 are busy for 10 ms each; the third request must take copy 2 at once
    e.submit(0, plan(vec![on(app, 0, 10)]), 0).unwrap();
    e.submit(0, plan(vec![on(app, 1, 10)]), 1).unwrap();
    e.submit(0, plan(visit(app, PickKind::LeastConn, 0b111, 0)), 2)
        .unwrap();
    let done = run_all(&mut e);
    assert_eq!(copy_of(&done, 2), 2);
    assert_eq!(fin(&done, 2).done, ms(10), "no queue: it ran at once");
}

#[test]
fn least_conn_counts_the_queue_and_ties_rotate_by_the_offset() {
    let t = topo(THREE);
    let app = t.id("app").unwrap();
    // no load: the offset picks among the tied copies
    for (offset, want) in [(0u16, 0u16), (1, 1), (2, 2), (3, 0)] {
        let mut e = Engine::new(&t);
        e.submit(0, plan(visit(app, PickKind::LeastConn, 0b111, offset)), 0)
            .unwrap();
        assert_eq!(copy_of(&run_all(&mut e), 0), want, "offset {offset}");
    }
    // copy 0 has one running and one waiting, copy 1 one running: copy 2 (idle) still wins, and
    // with copy 2 busy too, copy 1 (shorter queue) beats copy 0
    let mut e = Engine::new(&t);
    e.submit(0, plan(vec![on(app, 0, 10)]), 0).unwrap();
    e.submit(0, plan(vec![on(app, 0, 10)]), 1).unwrap();
    e.submit(0, plan(vec![on(app, 1, 10)]), 2).unwrap();
    e.submit(0, plan(vec![on(app, 2, 10)]), 3).unwrap();
    e.submit(0, plan(visit(app, PickKind::LeastConn, 0b111, 0)), 4)
        .unwrap();
    assert_eq!(copy_of(&run_all(&mut e), 4), 1);
}

#[test]
fn p2c_takes_the_less_loaded_of_its_two_and_the_first_on_a_tie() {
    let t = topo(THREE);
    let app = t.id("app").unwrap();
    let mut e = Engine::new(&t);
    e.submit(0, plan(vec![on(app, 0, 10)]), 0).unwrap();
    // candidates 0 and 2: copy 0 is busy, copy 2 idle
    e.submit(0, plan(visit(app, PickKind::P2c, 0b101, 0)), 1)
        .unwrap();
    // candidates 1 and 2, both idle: the first (copy 1)
    e.submit(0, plan(visit(app, PickKind::P2c, 0b110, 0)), 2)
        .unwrap();
    let done = run_all(&mut e);
    assert_eq!(copy_of(&done, 1), 2);
    assert_eq!(copy_of(&done, 2), 1);
}

#[test]
fn a_node_without_slots_ties_every_time_and_the_hold_follows_the_pick() {
    let t = topo("version: 1\nnodes:\n  app: { type: service, replicas: 2 }\n");
    let app = t.id("app").unwrap();
    let mut e = Engine::new(&t);
    e.submit(0, plan(visit(app, PickKind::LeastConn, 0b11, 1)), 0)
        .unwrap();
    assert_eq!(
        copy_of(&run_all(&mut e), 0),
        1,
        "no gate: the offset decides"
    );
    // a hold on the chosen copy
    let t = topo(
        "version: 1\nnodes:\n  app: { type: service, concurrency: 1, replicas: 2, hold: call }\n",
    );
    let app = t.id("app").unwrap();
    let mut e = Engine::new(&t);
    e.submit(0, plan(vec![on(app, 0, 10)]), 0).unwrap();
    let items = vec![
        pick(app, PickKind::LeastConn, 0b11, 0),
        PlanItem::Hold {
            node: app,
            replica: DYN_BASE,
            size: 1,
        },
        on(app, DYN_BASE, 10),
    ];
    e.submit(0, plan(items), 1).unwrap();
    let done = run_all(&mut e);
    assert_eq!(fin(&done, 1).holds[0].replica, 1);
    assert_eq!(
        fin(&done, 1).done,
        ms(10),
        "copy 0 was busy; it took copy 1"
    );
}

#[test]
fn the_same_plan_always_gives_the_same_run() {
    let t = topo(THREE);
    let app = t.id("app").unwrap();
    let run = || {
        let mut e = Engine::new(&t);
        for i in 0..30u64 {
            e.submit(
                i * ms(3),
                plan(visit(
                    app,
                    PickKind::LeastConn,
                    0b111,
                    u16::try_from(i).unwrap(),
                )),
                i,
            )
            .unwrap();
        }
        let mut done = run_all(&mut e);
        done.sort_by_key(|f| f.order);
        done.iter()
            .map(|f| (copy_of(std::slice::from_ref(f), f.order), f.done))
            .collect::<Vec<_>>()
    };
    assert_eq!(run(), run());
}

#[test]
fn a_copy_killed_for_memory_is_not_a_candidate_while_it_is_down() {
    // two requests of room per copy: least-conn would choose the idle copy 0, which is down, so the
    // busier copy 1 gets the call
    let t = topo(
        "version: 1
nodes:
  app: { type: service, replicas: 2, memory: { limit: 2GB, per_request: 1GB, on_limit: kill, restart: 1s } }
",
    );
    let app = t.id("app").unwrap();
    let mut e = Engine::new(&t);
    for (tag, at) in [(0, 0), (1, 1), (2, 2)] {
        e.submit(ms(at), plan(vec![on(app, 0, 5)]), tag).unwrap(); // the third kills copy 0
    }
    e.submit(ms(3), plan(vec![on(app, 1, 50)]), 4).unwrap(); // copy 1 holds one
    e.submit(ms(4), plan(visit(app, PickKind::LeastConn, 0b11, 0)), 3)
        .unwrap();
    let done = run_all(&mut e);
    assert_eq!(copy_of(&done, 4), 1);
    // and with every candidate down the pick still names one (the call is then refused there)
    let mut e = Engine::new(&t);
    for rep in [0u16, 1] {
        for n in 0..3u64 {
            let at = u64::from(rep) * 10 + n;
            e.submit(ms(at), plan(vec![on(app, rep, 5)]), u64::from(rep) * 3 + n)
                .unwrap();
        }
    }
    e.submit(ms(30), plan(visit(app, PickKind::LeastConn, 0b11, 0)), 9)
        .unwrap();
    let done = run_all(&mut e);
    assert_eq!(err_of(fin(&done, 6)).1, 503);
}
