//! Tests of the engine readings in a tick (RULES 115): per-route figures, the retry funnel, link
//! load, per-copy load and breaker states. The oracles are the definitions: a route's histogram
//! holds exactly its requests, a load is busy time over the span.

use super::*;
use crate::actor::Probe;
use kaanal_core::chain::{EndpointsInput, RouteSpec, compile};
use kaanal_core::replay::{BreakerState, Engine, WireCounters};
use kaanal_core::{ChainRequest, Config, EffectiveChaos, sample_plan};
use std::sync::Arc;

const STATUS: TickStatus = TickStatus {
    rate: 100.0,
    paused: false,
    overloaded: false,
    in_flight: 0,
};

const CHAIN: &str = "version: 1\nnodes:\n  gw: { type: gateway, proc: 1ms }\n  app: { type: service, proc: 2ms }\nedges:\n  - { from: gw, to: app, net: 1ms, bandwidth: 1000000 }\n";

/// A generated request of route `stream`, as the live generator tags it (`actor.rs`): bit 63, the
/// route in bits 40 to 55.
const fn generated(stream: u64, n: u64) -> u64 {
    (1 << 63) | (stream << 40) | n
}

/// `n` requests through `CHAIN`, tagged as generated requests of routes `0, 1, 0, 1, ...`.
fn two_routes(n: u64) -> (Topology, Vec<Finished>) {
    let t = Topology::build(&Config::from_yaml(CHAIN).unwrap()).unwrap();
    let eps = compile(
        &t,
        &[RouteSpec::inferred("GET /x", t.id("app").unwrap())],
        EndpointsInput::default(),
    )
    .unwrap();
    let ep = eps.get("GET /x").unwrap();
    let mut e = Engine::live(&t);
    for i in 0..n {
        let (plan, _) = sample_plan(
            &t,
            &EffectiveChaos::default(),
            &ChainRequest {
                endpoint: ep,
                at: Duration::ZERO,
                n: i,
                seed: 1,
                pin_delay: Duration::ZERO,
                req_bytes: 0,
                resp_bytes: 0,
            },
        );
        e.submit(i * 5_000_000, Arc::new(plan), generated(i % 2, i))
            .unwrap();
    }
    e.advance_to(u64::MAX / 4).unwrap();
    (t, e.drain_finished())
}

fn counts(h: &[(u16, u32)]) -> u32 {
    h.iter().map(|(_, c)| c).sum()
}

#[test]
fn each_route_has_its_own_requests_and_latencies_that_add_up_to_the_tick() {
    let (t, fin) = two_routes(30);
    let mut a = Aggregator::new(&t);
    for f in &fin {
        a.add(f, false);
    }
    let tick = a.take((250, 250), &[(0, 0), (0, 0)], &t, STATUS);
    assert_eq!(tick.endpoints.len(), 2);
    assert_eq!(
        tick.endpoints
            .iter()
            .map(|e| e.requests)
            .collect::<Vec<_>>(),
        [15, 15]
    );
    assert!(tick.endpoints.iter().all(|e| e.failed == 0));
    // the routes' histograms add up to the tick's, bucket for bucket (one latency, so one bucket)
    assert_eq!(counts(&tick.hist), 30);
    for e in &tick.endpoints {
        assert_eq!(counts(&e.hist), 15);
        assert_eq!(
            e.hist,
            tick.hist.iter().map(|&(b, _)| (b, 15)).collect::<Vec<_>>()
        );
    }
}

#[test]
fn requests_with_no_route_leave_no_endpoints_and_no_key() {
    let (t, mut fin) = two_routes(6);
    for f in &mut fin {
        f.tag &= !(1 << 63); // as a real mock request is tagged
    }
    let mut a = Aggregator::new(&t);
    for f in &fin {
        a.add(f, true);
    }
    let tick = a.take((250, 250), &[(0, 0), (0, 0)], &t, STATUS);
    assert_eq!(tick.endpoints.len(), 0);
    let json = serde_json::to_value(&tick).unwrap();
    assert!(json.get("endpoints").is_none() && json.get("breakers").is_none());
}

fn probe(wires: Vec<WireCounters>, busy: Vec<u128>) -> Probe {
    Probe {
        copies: vec![vec![(1, 2), (0, 0)], vec![(3, 0)]],
        wires,
        link_busy: busy,
        breakers: vec![(0, 0, BreakerState::Open), (0, 1, BreakerState::Closed)],
    }
}

fn wire(calls: u64, retried: u64, recovered: u64, failed: u64) -> WireCounters {
    WireCounters {
        calls,
        retried,
        recovered,
        failed,
        ..WireCounters::default()
    }
}

#[test]
fn the_funnel_is_the_change_in_the_engines_counts_and_load_is_busy_time_over_the_span() {
    let (t, _) = two_routes(0);
    let before = probe(vec![wire(100, 10, 6, 5)], vec![200_000_000]);
    let after = probe(vec![wire(160, 25, 15, 12)], vec![700_000_000]);
    // half a second of busy time in one second: the wire was half full
    let r = readings(&t, Some(&before), &after, 1_000_000_000);
    assert_eq!(
        r.funnel,
        [Funnel {
            calls: 60,
            retried: 15,
            recovered: 9,
            failed: 7
        }]
    );
    assert_eq!(r.link_load, [Some(0.5)]);
    // breakers and copies are read at the instant, with or without a span
    let first = readings(&t, None, &after, 0);
    assert!(first.funnel.is_empty() && first.link_load.is_empty());
    assert_eq!(
        first.breakers,
        [
            BreakerTick {
                edge: 0,
                replica: 0,
                state: "open"
            },
            BreakerTick {
                edge: 0,
                replica: 1,
                state: "closed"
            },
        ]
    );
    assert_eq!(first.copies, after.copies);
}

#[test]
fn the_tick_carries_the_readings_only_where_they_mean_something() {
    let (t, _) = two_routes(0);
    let mut a = Aggregator::new(&t);
    let before = probe(vec![wire(0, 0, 0, 0)], vec![0]);
    let after = probe(vec![wire(10, 4, 3, 1)], vec![250_000_000]);
    a.set_readings(readings(&t, Some(&before), &after, 1_000_000_000));
    let tick = a.take((250, 250), &[(0, 0), (0, 0)], &t, STATUS);
    // a node with two copies reports each; one with a single copy has nothing to split
    assert_eq!(tick.nodes[0].copy_load, [(1, 2), (0, 0)]);
    assert_eq!(tick.nodes[1].copy_load.len(), 0);
    assert_eq!(tick.wires[0].funnel.retried, 4);
    assert_eq!(tick.wires[0].load, Some(0.25));
    assert_eq!(tick.breakers.len(), 2);
    // the readings belong to one tick
    let next = a.take((500, 500), &[], &t, STATUS);
    assert!(next.breakers.is_empty() && next.wires[0].funnel.is_empty());
}

#[test]
fn a_request_to_a_pinned_error_route_is_failed_at_its_entry_after_the_chain_ran() {
    let (t, mut fin) = two_routes(10);
    // the first four went to a route pinned to an error (bit 62 of the tag, set by the generator)
    for f in fin.iter_mut().take(4) {
        f.tag |= 1 << 62;
    }
    let mut a = Aggregator::new(&t);
    for f in &fin {
        a.add(f, false);
    }
    let tick = a.take((250, 250), &[(0, 0), (0, 0)], &t, STATUS);
    assert_eq!((tick.requests, tick.failed), (10, 4));
    // the chain ran for all of them: every node saw every request's own hop
    assert!(
        tick.nodes.iter().all(|n| n.arrived == 10),
        "{:?}",
        tick.nodes
    );
    // the four are failed at the entry component (gw, node 0) with cause pinned, and are in no
    // latency histogram: only the six that were served are
    assert_eq!(tick.nodes[0].dropped.pinned, 4);
    assert_eq!(tick.nodes[1].dropped, Dropped::default());
    assert_eq!(counts(&tick.hist), 6);
    assert_eq!(tick.endpoints.iter().map(|e| e.failed).sum::<u32>(), 4);
    let pinned: Vec<_> = tick
        .journeys
        .iter()
        .filter(|j| j.cause == Some("pinned"))
        .collect();
    assert_eq!(pinned.len(), 4);
    assert!(pinned.iter().all(|j| j.dropped_at == Some(0)));
    // a real request is never marked, and the flag is not mistaken for a generated one's route
    assert!(fin[..4].iter().all(|f| kaanal_server_stream(f) < 2));
}

fn kaanal_server_stream(f: &Finished) -> u16 {
    crate::actor::stream_of(f).unwrap()
}
