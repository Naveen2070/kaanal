//! Step 8 (RULES 101): processor sharing on a node's own work. A node with `cpu` millicores runs
//! `n` own hops at speed `min(1, cores / n)`. Plans are built by hand and the answers are the
//! textbook ones, exact to the nanosecond. The config key arrives in S8b, so the tests set
//! `Node::cpu` on the built topology.

use crate::common::*;
use kaanal_core::replay::Engine;
use kaanal_core::{HopKind, Op, Topology};

const APP: &str = "version: 1\nnodes:\n  app: { type: service }\n";

fn with_cpu(millicores: Option<u32>) -> (Topology, usize) {
    let mut t = topo(APP);
    let app = t.id("app").unwrap();
    t.nodes[app].cpu = millicores;
    (t, app)
}

/// Finish times (ns) of requests 0.. submitted at the given start times, each one `Proc` hop.
fn run(t: &Topology, app: usize, jobs: &[(u64, u64)]) -> Vec<u64> {
    let mut e = Engine::new(t);
    for (i, &(at, dur_ms)) in jobs.iter().enumerate() {
        e.submit(at, plan(vec![hop(app, HopKind::Proc, dur_ms)]), i as u64)
            .unwrap();
    }
    let f = run_all(&mut e);
    (0..jobs.len() as u64).map(|i| fin(&f, i).done).collect()
}

#[test]
fn a_hop_alone_takes_its_drawn_time_whatever_the_cores() {
    for cores in [1000, 4000] {
        let (t, app) = with_cpu(Some(cores));
        assert_eq!(run(&t, app, &[(0, 10)]), vec![ms(10)], "cores {cores}");
    }
}

#[test]
fn up_to_as_many_hops_as_cores_nothing_slows() {
    let (t, app) = with_cpu(Some(2000));
    assert_eq!(
        run(&t, app, &[(0, 10), (0, 10)]),
        vec![ms(10), ms(10)],
        "n = c: full speed"
    );
}

#[test]
fn two_equal_hops_on_one_core_end_at_twice_the_demand() {
    let (t, app) = with_cpu(Some(1000));
    assert_eq!(
        run(&t, app, &[(0, 10), (0, 10)]),
        vec![ms(20), ms(20)],
        "each gets half a core"
    );
}

#[test]
fn a_hop_joining_late_is_the_textbook_two_job_sequence() {
    // one core; A needs 10 ms from t=0, B needs 10 ms from t=4.
    // 0..4: A alone, 6 left. 4..: both at 1/2; A has 6 left -> done at 4+12 = 16; B has done 6, 4 left.
    // 16..: B alone, 4 left -> done at 20.
    let (t, app) = with_cpu(Some(1000));
    assert_eq!(run(&t, app, &[(0, 10), (ms(4), 10)]), vec![ms(16), ms(20)]);
}

#[test]
fn half_a_core_never_speeds_a_hop_past_its_drawn_time() {
    // 500 millicores, one hop: speed is min(1, 0.5 / 1) = 0.5, so it takes twice as long. A node
    // limited to half a core really is half as fast.
    let (t, app) = with_cpu(Some(500));
    assert_eq!(run(&t, app, &[(0, 10)]), vec![ms(20)]);
}

#[test]
fn a_database_op_shares_the_cores_like_proc() {
    let mut t = topo(
        "version: 1\nnodes:\n  app: { type: service }\n  db: { type: database, ops: { read: 10ms, write: 10ms, scan: 10ms } }\nedges:\n  - { from: app, to: db }\n",
    );
    let db = t.id("db").unwrap();
    t.nodes[db].cpu = Some(1000);
    let mut e = Engine::new(&t);
    for i in 0..2 {
        e.submit(0, plan(vec![hop(db, HopKind::Op(Op::Read), 10)]), i)
            .unwrap();
    }
    let f = run_all(&mut e);
    assert_eq!((fin(&f, 0).done, fin(&f, 1).done), (ms(20), ms(20)));
}

#[test]
fn the_stretch_shows_as_wait_on_the_hop() {
    let (t, app) = with_cpu(Some(1000));
    let mut e = Engine::new(&t);
    for i in 0..2 {
        e.submit(0, plan(vec![hop(app, HopKind::Proc, 10)]), i)
            .unwrap();
    }
    let f = run_all(&mut e);
    assert_eq!(
        fin(&f, 0).hops[0].wait,
        std::time::Duration::from_millis(10)
    );
}

#[test]
fn off_by_default_nothing_is_shared() {
    let (t, app) = with_cpu(None);
    assert_eq!(run(&t, app, &[(0, 10), (0, 10)]), vec![ms(10), ms(10)]);
}
