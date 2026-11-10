//! Killing a copy for lack of memory (step 8K, RULES 112): what the kill fails, how long the copy
//! is down, what a retry does, and that a node that only refuses behaves as before.

use crate::common::*;
use kaanal_core::replay::{Engine, FailCause};
use kaanal_core::{ChainResult, HopKind};

/// One copy that holds one request at once (1 GB, 1 GB per request) and dies of it for 100 ms.
const KILL: &str = "version: 1\nnodes:\n  app: { type: service, memory: { limit: 1GB, per_request: 1GB, on_limit: kill, restart: 100ms } }\n";
/// The same node that refuses instead.
const REFUSE: &str =
    "version: 1\nnodes:\n  app: { type: service, memory: { limit: 1GB, per_request: 1GB } }\n";

fn submit(e: &mut Engine, app: usize, at: u64, ms_: u64, tag: u64) {
    e.submit(at, plan(vec![hop(app, HopKind::Proc, ms_)]), tag)
        .unwrap();
}

#[test]
fn a_request_the_memory_cannot_hold_kills_the_copy_and_fails_whoever_it_was_serving() {
    let topology = topo(KILL);
    let app = topology.id("app").unwrap();
    let mut engine = Engine::new(&topology);
    submit(&mut engine, app, 0, 50, 0); // takes the one slot
    submit(&mut engine, app, ms(10), 50, 1); // finds it full: the copy dies
    let done = run_all(&mut engine);
    let (a, b) = (fin(&done, 0), fin(&done, 1));
    for fin_ in [a, b] {
        assert_eq!(err_of(fin_).1, 502, "{fin_:?}");
        assert_eq!(fin_.cause, Some(FailCause::Oom));
        assert_eq!(fin_.done, ms(10), "both are answered at the kill");
    }
    assert!(matches!(a.result, ChainResult::Err { .. }));
    assert_eq!(engine.node_counters()[app].oom_kills, 1);
}

#[test]
fn the_copy_refuses_with_503_while_it_is_down_and_serves_again_after_the_restart() {
    let t = topo(KILL);
    let app = t.id("app").unwrap();
    let mut e = Engine::new(&t);
    submit(&mut e, app, 0, 50, 0);
    submit(&mut e, app, ms(10), 50, 1); // kill at 10 ms, down until 110 ms
    submit(&mut e, app, ms(60), 5, 2); // while down
    submit(&mut e, app, ms(109), 5, 3); // the last instant of the outage
    submit(&mut e, app, ms(110), 5, 4); // back
    let f = run_all(&mut e);
    for tag in [2, 3] {
        let x = fin(&f, tag);
        assert_eq!((err_of(x).1, x.cause), (503, Some(FailCause::Oom)), "{tag}");
    }
    let back = fin(&f, 4);
    assert_eq!(back.result, ChainResult::Ok);
    assert_eq!(back.done, ms(115));
    assert_eq!(
        e.node_counters()[app].oom_kills,
        1,
        "a refusal is not a second kill"
    );
}

#[test]
fn a_node_that_refuses_is_unchanged_and_never_dies() {
    let t = topo(REFUSE);
    let app = t.id("app").unwrap();
    let mut e = Engine::new(&t);
    submit(&mut e, app, 0, 50, 0);
    submit(&mut e, app, ms(10), 50, 1);
    submit(&mut e, app, ms(20), 5, 2);
    let f = run_all(&mut e);
    assert_eq!(
        fin(&f, 0).result,
        ChainResult::Ok,
        "the one being served finishes"
    );
    for tag in [1, 2] {
        let x = fin(&f, tag);
        assert_eq!((err_of(x).1, x.cause), (503, Some(FailCause::Overflow)));
    }
    assert_eq!(e.node_counters()[app].oom_kills, 0);
}

#[test]
fn only_the_copy_that_died_is_down() {
    let t = topo(
        "version: 1\nnodes:\n  app: { type: service, replicas: 2, memory: { limit: 1GB, per_request: 1GB, on_limit: kill, restart: 100ms } }\n",
    );
    let app = t.id("app").unwrap();
    let mut e = Engine::new(&t);
    let on = |rep: u16, ms_: u64| {
        plan(vec![kaanal_core::PlanItem::Hop(kaanal_core::Hop {
            node: app,
            replica: rep,
            kind: HopKind::Proc,
            start: std::time::Duration::ZERO,
            dur: std::time::Duration::from_millis(ms_),
        })])
    };
    e.submit(0, on(0, 50), 0).unwrap();
    e.submit(ms(10), on(0, 50), 1).unwrap(); // copy 0 dies
    e.submit(ms(20), on(1, 5), 2).unwrap(); // copy 1 carries on
    e.submit(ms(30), on(0, 5), 3).unwrap(); // copy 0 is down
    let f = run_all(&mut e);
    assert_eq!(fin(&f, 2).result, ChainResult::Ok);
    assert_eq!(err_of(fin(&f, 3)).1, 503);
    assert_eq!(e.replica_counters()[app][0].oom_kills, 1);
    assert_eq!(e.replica_counters()[app][1].oom_kills, 0);
}

#[test]
fn a_queued_request_dies_with_the_copy() {
    // two slots of memory, a queue of one in front of them: the third waits, the fourth is refused
    // for memory and kills the copy, taking the waiting one with it
    let t = topo(
        "version: 1\nnodes:\n  app: { type: service, concurrency: { limit: 2, queue: 1 }, memory: { limit: 2GB, per_request: 1GB, on_limit: kill, restart: 100ms } }\n",
    );
    let app = t.id("app").unwrap();
    let mut e = Engine::new(&t);
    for (tag, at) in [(0, 0), (1, 1), (2, 2), (3, 3)] {
        submit(&mut e, app, ms(at), 50, tag);
    }
    let f = run_all(&mut e);
    for tag in 0..4 {
        let x = fin(&f, tag);
        assert_eq!((err_of(x).1, x.cause), (502, Some(FailCause::Oom)), "{tag}");
        assert_eq!(x.done, ms(3));
    }
}

#[test]
fn a_long_hop_that_was_on_the_copy_does_not_hold_its_slot_after_the_kill() {
    // a 5 s hop and a 100 ms restart: when the copy is back the slot is free, as a real process
    // that was killed has no memory held; with the old zombie it would still be taken at 200 ms
    let topology = topo(KILL);
    let app = topology.id("app").unwrap();
    let mut engine = Engine::new(&topology);
    submit(&mut engine, app, 0, 5000, 0);
    submit(&mut engine, app, ms(10), 5, 1); // kills
    submit(&mut engine, app, ms(200), 5, 2); // after the restart at 110 ms
    let done = run_all(&mut engine);
    assert_eq!(err_of(fin(&done, 0)).1, 502);
    let later = fin(&done, 2);
    assert_eq!(
        (later.result.clone(), later.done),
        (ChainResult::Ok, ms(205))
    );
    assert_eq!(engine.node_counters()[app].oom_kills, 1, "no second kill");
}

#[test]
fn what_ran_on_the_cpus_stops_at_the_kill_and_frees_them() {
    // one core: the killed 5 s hop must not keep sharing it with the request that comes back
    let topology = topo(
        "version: 1\nnodes:\n  app: { type: service, cpu: { cores: 1 }, memory: { limit: 1GB, per_request: 1GB, on_limit: kill, restart: 100ms } }\n",
    );
    let app = topology.id("app").unwrap();
    let mut engine = Engine::new(&topology);
    submit(&mut engine, app, 0, 5000, 0);
    submit(&mut engine, app, ms(10), 5, 1);
    submit(&mut engine, app, ms(200), 10, 2);
    let done = run_all(&mut engine);
    let later = fin(&done, 2);
    assert_eq!(later.result, ChainResult::Ok);
    assert_eq!(later.done, ms(210), "the whole core, not half of it");
    assert_eq!(fin(&done, 0).done, ms(10), "answered at the kill");
}

#[test]
fn the_slots_of_a_hold_call_scope_go_with_the_copy() {
    // `hold: call` keeps the slot while the node waits on its callee; a kill frees it too
    let topology = topo(
        "version: 1\nnodes:\n  app: { type: service, hold: call, memory: { limit: 1GB, per_request: 1GB, on_limit: kill, restart: 100ms } }\n  db: { type: database, ops: { read: 1s, write: 1s, scan: 1s } }\nedges:\n  - { from: app, to: db }\n",
    );
    let app = topology.id("app").unwrap();
    let db = topology.id("db").unwrap();
    let mut engine = Engine::new(&topology);
    let held = plan(vec![
        kaanal_core::PlanItem::Hold {
            node: app,
            replica: 0,
            size: 2,
        },
        hop(app, HopKind::Proc, 1),
        hop(db, HopKind::Op(kaanal_core::Op::Read), 1000),
    ]);
    engine.submit(0, held, 0).unwrap();
    submit(&mut engine, app, ms(10), 5, 1); // finds the held slot taken: kill
    submit(&mut engine, app, ms(200), 5, 2); // back and free
    let done = run_all(&mut engine);
    assert_eq!(err_of(fin(&done, 0)).1, 502);
    assert_eq!(fin(&done, 2).result, ChainResult::Ok);
}
