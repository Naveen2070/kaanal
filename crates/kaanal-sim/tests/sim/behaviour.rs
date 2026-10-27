//! Queueing behaviour: the Lindley recursion, FIFO order, determinism, overload, utilization, warmup.

use crate::common::*;
use kaanal_core::{ChainResult, ChaosState, HopKind, NodeChaos, Topology};
use kaanal_sim::{SimConfig, run};
use std::time::Duration;

/// Asserts every request's queue wait at `app` equals the Lindley recursion, exactly.
pub(crate) fn assert_lindley(t: &Topology, rep: &kaanal_sim::SimReport) {
    let app = t.id("app").unwrap();
    let (mut prev_a, mut prev_s, mut prev_w) = (0i128, 0i128, 0i128);
    for (k, r) in rep.requests.iter().enumerate() {
        let (hop, wait) = r
            .hops
            .iter()
            .find(|(h, _)| h.node == app && h.kind == HopKind::Proc)
            .unwrap();
        let a = nanos(r.arrival);
        // Lindley: W(k+1) = max(0, W(k) + S(k) − (A(k+1) − A(k))), W(0) = 0.
        let expected = if k == 0 {
            0
        } else {
            (prev_w + prev_s - (a - prev_a)).max(0)
        };
        assert_eq!(nanos(*wait), expected, "request {k}");
        (prev_a, prev_s, prev_w) = (a, nanos(hop.dur), expected);
    }
}

#[test]
fn engine_matches_lindley_recursion() {
    let (t, e) = topo(QUEUED);
    let rep = run(&t, &e, &ChaosState::default(), &one_stream(80.0, 125)).unwrap();
    assert_lindley(&t, &rep);
    assert!(rep.requests.len() > 9_000, "{}", rep.requests.len());
    let queued = rep
        .requests
        .iter()
        .filter(|r| r.hops.iter().any(|(_, w)| !w.is_zero()))
        .count();
    assert!(queued > 1_000, "the test must really queue: {queued}");
}

#[test]
fn simultaneous_events_keep_fifo_order() {
    // 1e6 req/s: the mean gap is 1 µs, so ~0.05% of gaps round to 0 ns and arrivals tie with
    // hop ends and with each other's timestamps. This pins FIFO queue order under equal times. The 800 ns fixed service keeps the queue busy (ρ = 0.8).
    let (t, e) = topo(&QUEUED.replace("{ mean: 10ms }", "800ns"));
    let rep = run(&t, &e, &ChaosState::default(), &one_stream(1e6, 1)).unwrap();
    let ties = rep
        .requests
        .windows(2)
        .filter(|w| w[0].arrival == w[1].arrival)
        .count();
    assert!(ties > 100, "the test must really produce ties: {ties}");
    assert_lindley(&t, &rep);
}

#[test]
fn same_seed_same_report() {
    // Determinism only. FIFO queue order under equal timestamps is pinned by
    // `simultaneous_events_keep_fifo_order`; the (time, seq) rule itself is reasoned, not tested.
    let (t, e) = topo(&QUEUED.replace("{ mean: 10ms }", "10ms"));
    let cfg = one_stream(90.0, 60);
    let a = run(&t, &e, &ChaosState::default(), &cfg).unwrap();
    assert_eq!(a, run(&t, &e, &ChaosState::default(), &cfg).unwrap());
    let b = run(
        &t,
        &e,
        &ChaosState::default(),
        &SimConfig { seed: 43, ..cfg },
    )
    .unwrap();
    assert_ne!(a, b, "a different seed gives a different run");
}

#[test]
fn down_gateway_completes_with_no_hops() {
    let (t, e) = topo(QUEUED);
    let mut down = ChaosState::default();
    down.insert(
        t.gateway.unwrap(),
        NodeChaos {
            down: true,
            ..NodeChaos::default()
        },
    );
    let rep = run(&t, &e, &down, &one_stream(10.0, 10)).unwrap();
    assert_ne!(rep.requests.len(), 0, "the stream must produce requests");
    for r in &rep.requests {
        assert!(r.hops.is_empty() && r.latency.is_zero(), "{r:?}");
        assert!(matches!(r.result, ChainResult::Err { status: 503, .. }));
    }
}

#[test]
fn overload_grows_the_queue_but_finishes() {
    let (t, e) = topo(QUEUED);
    // ρ = 1.5: arrivals outpace the single slot.
    let rep = run(&t, &e, &ChaosState::default(), &one_stream(150.0, 20)).unwrap();
    let app = &rep.nodes[t.id("app").unwrap()];
    assert!(app.max_queue > 500, "{app:?}");
    assert!(
        (0.95..=1.0).contains(&app.utilization),
        "busy time is clipped to the window: {app:?}"
    );
    assert_eq!(app.completed, rep.requests.len() as u64);
}

#[test]
fn unlimited_nodes_report_zero_utilization_and_no_queue() {
    let (t, e) = topo(QUEUED);
    let rep = run(&t, &e, &ChaosState::default(), &one_stream(50.0, 20)).unwrap();
    let gw = &rep.nodes[t.gateway.unwrap()];
    assert_eq!(
        (gw.utilization, gw.max_queue, gw.mean_wait),
        (0.0, 0, Duration::ZERO)
    );
    assert_eq!(gw.completed, rep.requests.len() as u64);
}

#[test]
fn warmup_requests_are_not_reported() {
    let (t, e) = topo(QUEUED);
    let mut cfg = one_stream(50.0, 20);
    cfg.warmup = Duration::from_secs(10);
    let rep = run(&t, &e, &ChaosState::default(), &cfg).unwrap();
    assert!(rep.requests.iter().all(|r| r.arrival >= cfg.warmup));
    assert!(
        rep.requests.first().unwrap().n > 0,
        "counters keep running through warmup"
    );
}

#[test]
fn replicated_node_utilization_is_per_slot_of_every_replica() {
    // 10 ms fixed work on 2 replicas of one slot each; 100 req/s round-robin gives each replica
    // 50 req/s, so each is busy half the time and the node's utilization is 0.5 (not 1.0).
    let yaml = "version: 1\nnodes:\n  gw: { type: gateway }\n  app: { type: service, concurrency: 1, replicas: 2, proc: 10ms }\nedges:\n  - { from: gw, to: app }\n";
    let (t, e) = topo(yaml);
    let rep = kaanal_sim::run(
        &t,
        &e,
        &kaanal_core::ChaosState::default(),
        &one_stream(100.0, 400),
    )
    .unwrap();
    let util = rep.nodes[t.id("app").unwrap()].utilization;
    assert!((util - 0.5).abs() < 0.03, "utilization {util}");
}
