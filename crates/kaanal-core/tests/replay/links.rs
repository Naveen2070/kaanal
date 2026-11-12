//! Links in the engine: a bandwidth-limited edge shares its rate fairly between the transfers on it
//! (RULES 62). The test topologies run at 1000 B/s, so a transfer of `ms` milliseconds is `ms` bytes.

use crate::common::*;
use kaanal_core::{Hop, HopKind, Pipe, Plan, PlanItem, Xfer};
use std::sync::Arc;
use std::time::Duration;

const NET: &str = "version: 1\nnodes:\n  gw: { type: gateway }\n  a: { type: service, concurrency: 1 }\n  b: { type: service }\nedges:\n  - { from: gw, to: a, bandwidth: 1000 }\n  - { from: gw, to: b, bandwidth: 1000 }\n  - { from: a, to: b }\n";

/// A transfer over `pipes` of `ms` ms at 1000 B/s (so `ms` bytes), at zero load.
fn xfer(ms: u64, pipes: &[Pipe]) -> Xfer {
    let mut p = [None; 3];
    for (slot, pipe) in p.iter_mut().zip(pipes) {
        *slot = Some(*pipe);
    }
    Xfer {
        bytes: ms,
        pipes: p,
        limit: None,
    }
}

/// A one-hop plan: the hop and the transfer it carries.
fn one((item, x): (PlanItem, Xfer)) -> Arc<Plan> {
    Arc::new(Plan {
        items: vec![item],
        xfers: vec![(0, x)],
        ..Plan::default()
    })
}

/// A link hop of `ms` on `edge`, belonging to `to`, with its transfer.
fn link(to: usize, edge: usize, resp: bool, ms: u64) -> (PlanItem, Xfer) {
    let edge = u16::try_from(edge).unwrap();
    let hop = PlanItem::Hop(Hop {
        node: to,
        replica: 0,
        kind: HopKind::Link { edge, resp },
        start: Duration::ZERO,
        dur: Duration::from_millis(ms),
    });
    (hop, xfer(ms, &[Pipe::Wire { edge }]))
}

#[test]
fn transfers_on_one_edge_share_its_rate_instead_of_queueing() {
    let t = topo(NET);
    let (a, edge) = (t.id("a").unwrap(), 0);
    let mut e = kaanal_core::replay::Engine::new(&t);
    for tag in 0..3 {
        e.submit(0, one(link(a, edge, false, 10)), tag).unwrap();
    }
    let done = run_all(&mut e);
    // a third of 1000 B/s is floored to 333 B/s (RULES 62: under 1 B/s of rounding per share), so
    // each ends 0.1% after 30 ms
    let near = |got: u128, want: u64| got.abs_diff(u128::from(want)) * 500 < u128::from(want);
    for o in 0..3 {
        assert!(
            near(u128::from(fin(&done, o).done), ms(30)),
            "{}",
            fin(&done, o).done
        );
    }
    let lost = fin(&done, 2).hops[0].wait;
    assert!(near(lost.as_nanos(), ms(20)), "{lost:?}");
    let c = &e.link_counters()[edge];
    assert_eq!((c.completed, c.max_queue), (3, 0), "nobody queues");
    assert!(near(c.wait, ms(60)), "20 ms lost, three times");
    assert!(
        near(c.area, ms(30)),
        "30 bytes at 1000 B/s: busy the whole time"
    );
}

#[test]
fn a_small_transfer_is_not_held_up_by_a_big_one() {
    let t = topo(NET);
    let (a, edge) = (t.id("a").unwrap(), 0);
    let mut e = kaanal_core::replay::Engine::new(&t);
    e.submit(0, one(link(a, edge, false, 100)), 0).unwrap();
    e.submit(0, one(link(a, edge, false, 10)), 1).unwrap();
    let done = run_all(&mut e);
    // 10 B at half of 1000 B/s: 20 ms (FIFO would have made it wait 100 ms)
    assert_eq!(fin(&done, 1).done, ms(20));
    // the big one moved 10 B meanwhile, then has the pipe: 20 + 90 ms
    assert_eq!(fin(&done, 0).done, ms(110));
}

#[test]
fn a_transfer_crossing_two_pipes_is_held_by_the_slower_and_leaves_the_rest_of_the_other() {
    // gw -> a: the wire is 1000 B/s and gw has a 200 B/s send pipe, so a transfer over both runs
    // at 200 and a second one on the wire alone takes the other 800, not an equal share of 500.
    let t = topo(
        "version: 1\nnodes:\n  gw: { type: gateway, bandwidth: { out: 200 } }\n  a: { type: service }\nedges:\n  - { from: gw, to: a, bandwidth: 1000 }\n",
    );
    let (gw, a) = (t.id("gw").unwrap(), t.id("a").unwrap());
    let both = [
        Pipe::Wire { edge: 0 },
        Pipe::Send {
            node: gw,
            replica: 0,
        },
    ];
    let mut e = kaanal_core::replay::Engine::new(&t);
    // 20 B over both pipes: 100 ms alone (200 B/s)
    let (held, _) = link(a, 0, false, 100);
    e.submit(0, one((held, xfer(20, &both))), 0).unwrap();
    e.submit(0, one(link(a, 0, false, 40)), 1).unwrap();
    let done = run_all(&mut e);
    // the other gets 800 B/s: 40 B in 50 ms (an equal split would be 500 B/s, 80 ms)
    assert_eq!(fin(&done, 1).done, ms(50));
    // the held one moved 10 B by then and has 10 B left at 200 B/s: 50 + 50 ms
    assert_eq!(fin(&done, 0).done, ms(100));
}

#[test]
fn two_edges_do_not_share_each_other() {
    let t = topo(NET);
    let (a, b) = (t.id("a").unwrap(), t.id("b").unwrap());
    let mut e = kaanal_core::replay::Engine::new(&t);
    e.submit(0, one(link(a, 0, false, 10)), 0).unwrap();
    e.submit(0, one(link(b, 1, false, 10)), 1).unwrap();
    let done = run_all(&mut e);
    assert_eq!((fin(&done, 0).done, fin(&done, 1).done), (ms(10), ms(10)));
}

#[test]
fn a_transfer_holds_no_node_slot() {
    // `a` has one slot and is busy for 10 ms; a transfer to `a` still runs at once.
    let t = topo(NET);
    let a = t.id("a").unwrap();
    let mut e = kaanal_core::replay::Engine::new(&t);
    e.submit(0, plan(vec![hop(a, HopKind::Proc, 10)]), 0)
        .unwrap();
    e.submit(0, one(link(a, 0, false, 4)), 1).unwrap();
    let done = run_all(&mut e);
    assert_eq!(fin(&done, 1).done, ms(4));
    let n = &e.node_counters()[a];
    assert_eq!((n.completed, n.wait), (1, 0), "node counters ignore links");
}

#[test]
fn a_busy_link_is_not_a_node_and_an_unlimited_edge_has_zero_counters() {
    let t = topo(NET);
    assert_eq!(e_len(&t), t.edges.len());
    let e = kaanal_core::replay::Engine::new(&t);
    assert_eq!(e.node_counters().len(), t.nodes.len());
    assert_eq!(e.replica_counters().len(), t.nodes.len());
    assert_eq!(
        e.link_counters()[2],
        kaanal_core::replay::NodeCounters::default(),
        "a to b: no bandwidth"
    );
}

fn e_len(t: &kaanal_core::Topology) -> usize {
    kaanal_core::replay::Engine::new(t).link_counters().len()
}

/// A node's network pipe hop of `ms`, owned by `node` replica `replica`.
fn nic(node: usize, replica: u16, send: bool, ms: u64) -> (PlanItem, Xfer) {
    let pipe = if send {
        Pipe::Send { node, replica }
    } else {
        Pipe::Recv { node, replica }
    };
    let hop = PlanItem::Hop(Hop {
        node,
        replica,
        kind: HopKind::Nic { send, resp: false },
        start: Duration::ZERO,
        dur: Duration::from_millis(ms),
    });
    (hop, xfer(ms, &[pipe]))
}

const NIC: &str = "version: 1\nnodes:\n  gw: { type: gateway, bandwidth: 1000, replicas: 2 }\n  a: { type: service }\n  b: { type: service }\nedges:\n  - { from: gw, to: a }\n  - { from: gw, to: b }\n";

#[test]
fn a_node_pipe_is_shared_by_every_wire_out_of_it_and_each_copy_has_its_own() {
    let t = topo(NIC);
    let gw = t.id("gw").unwrap();
    let mut e = kaanal_core::replay::Engine::new(&t);
    // two transfers on the same copy's send pipe share it; one on the other copy has its own
    e.submit(0, one(nic(gw, 0, true, 10)), 0).unwrap();
    e.submit(0, one(nic(gw, 0, true, 10)), 1).unwrap();
    e.submit(0, one(nic(gw, 1, true, 10)), 2).unwrap();
    let done = run_all(&mut e);
    assert_eq!(
        (fin(&done, 0).done, fin(&done, 1).done, fin(&done, 2).done),
        (ms(20), ms(20), ms(10))
    );
    let c = &e.nic_counters()[2 * gw];
    assert_eq!(
        (c.completed, c.wait),
        (3, u128::from(ms(20))),
        "10 ms lost, twice"
    );
    assert_eq!(
        e.nic_counters()[2 * gw + 1].completed,
        0,
        "the receive pipe is idle"
    );
}

#[test]
fn link_busy_reads_a_links_busy_time_up_to_now() {
    let t = topo(NET);
    let (a, edge) = (t.id("a").unwrap(), 0);
    let mut e = kaanal_core::replay::Engine::new(&t);
    e.submit(0, one(link(a, edge, false, 10)), 0).unwrap();
    e.advance_to(ms(4)).unwrap();
    // mid-transfer: busy since 0, so 4 ms of one transfer; the other wires are idle
    assert_eq!(e.link_busy_at(ms(4)), vec![u128::from(ms(4)), 0, 0]);
    run_all(&mut e);
    assert_eq!(e.link_busy_at(ms(100))[0], u128::from(ms(10)));
}
