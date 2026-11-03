//! Links in the engine: a bandwidth-limited edge is a FIFO, single-slot resource of its own.

use crate::common::*;
use kaanal_core::{Hop, HopKind, PlanItem};
use std::time::Duration;

const NET: &str = "version: 1\nnodes:\n  gw: { type: gateway }\n  a: { type: service, concurrency: 1 }\n  b: { type: service }\nedges:\n  - { from: gw, to: a, bandwidth: 1000 }\n  - { from: gw, to: b, bandwidth: 1000 }\n  - { from: a, to: b }\n";

/// A link hop of `ms` on `edge`, belonging to `to`.
fn link(to: usize, edge: usize, resp: bool, ms: u64) -> PlanItem {
    PlanItem::Hop(Hop {
        node: to,
        replica: 0,
        kind: HopKind::Link {
            edge: u16::try_from(edge).unwrap(),
            resp,
        },
        start: Duration::ZERO,
        dur: Duration::from_millis(ms),
    })
}

#[test]
fn transfers_on_one_edge_queue_fifo() {
    let t = topo(NET);
    let (a, edge) = (t.id("a").unwrap(), 0);
    let mut e = kaanal_core::replay::Engine::new(&t);
    for tag in 0..3 {
        e.submit(0, plan(vec![link(a, edge, false, 10)]), tag)
            .unwrap();
    }
    let done = run_all(&mut e);
    let ends: Vec<u64> = (0..3).map(|o| fin(&done, o).done).collect();
    assert_eq!(ends, [ms(10), ms(20), ms(30)]);
    assert_eq!(fin(&done, 2).hops[0].wait, Duration::from_millis(20));
    let c = &e.link_counters()[edge];
    assert_eq!((c.completed, c.max_queue), (3, 2));
    assert_eq!(c.wait, u128::from(ms(30)), "0 + 10 + 20 ms");
    assert_eq!(c.area, u128::from(ms(30)), "busy the whole time");
}

#[test]
fn two_edges_do_not_queue_each_other() {
    let t = topo(NET);
    let (a, b) = (t.id("a").unwrap(), t.id("b").unwrap());
    let mut e = kaanal_core::replay::Engine::new(&t);
    e.submit(0, plan(vec![link(a, 0, false, 10)]), 0).unwrap();
    e.submit(0, plan(vec![link(b, 1, false, 10)]), 1).unwrap();
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
    e.submit(0, plan(vec![link(a, 0, false, 4)]), 1).unwrap();
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
fn nic(node: usize, replica: u16, send: bool, ms: u64) -> PlanItem {
    PlanItem::Hop(Hop {
        node,
        replica,
        kind: HopKind::Nic { send, resp: false },
        start: Duration::ZERO,
        dur: Duration::from_millis(ms),
    })
}

const NIC: &str = "version: 1\nnodes:\n  gw: { type: gateway, bandwidth: 1000, replicas: 2 }\n  a: { type: service }\n  b: { type: service }\nedges:\n  - { from: gw, to: a }\n  - { from: gw, to: b }\n";

#[test]
fn a_node_pipe_is_shared_by_every_wire_out_of_it_and_each_copy_has_its_own() {
    let t = topo(NIC);
    let gw = t.id("gw").unwrap();
    let mut e = kaanal_core::replay::Engine::new(&t);
    // two transfers on the same copy's send pipe queue; one on the other copy runs at once
    e.submit(0, plan(vec![nic(gw, 0, true, 10)]), 0).unwrap();
    e.submit(0, plan(vec![nic(gw, 0, true, 10)]), 1).unwrap();
    e.submit(0, plan(vec![nic(gw, 1, true, 10)]), 2).unwrap();
    let done = run_all(&mut e);
    assert_eq!(
        (fin(&done, 0).done, fin(&done, 1).done, fin(&done, 2).done),
        (ms(10), ms(20), ms(10))
    );
    let c = &e.nic_counters()[2 * gw];
    assert_eq!((c.completed, c.wait), (3, u128::from(ms(10))));
    assert_eq!(
        e.nic_counters()[2 * gw + 1].completed,
        0,
        "the receive pipe is idle"
    );
}
