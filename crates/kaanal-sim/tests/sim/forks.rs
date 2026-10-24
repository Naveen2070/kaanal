//! Fork and join: failure ties, cancelled siblings, slot sharing, branch waits.

use crate::shared::compiled;
use kaanal_core::{ChainRequest, ChainResult, ChaosState, FailKind, HopKind, NodeChaos, execute};
use kaanal_sim::{SimConfig, Stream, run};
use std::time::Duration;

#[test]
fn zero_cost_failure_ties_go_to_the_first_branch() {
    // D17: both branches fail at t0 = 0, one of them through a zero-duration hop. The lower
    // branch (`s`) wins, in `execute()` and in the engine (simulator and live mode alike).
    let (t, e) = compiled(
        "version: 1\nnodes:\n  app: { type: service }\n  z: { type: service }\n  s: { type: external-api }\n  a: { type: external-api }\nedges:\n  - { from: app, to: z }\n  - { from: app, to: s }\n  - { from: app, to: a }\nendpoints:\n  \"GET /x\": { chain: [ { parallel: [ [ z, s ], [ a ] ] } ] }\n",
        &[("GET /x", "app")],
    );
    let (s, a) = (t.id("s").unwrap(), t.id("a").unwrap());
    let chaos = ChaosState::from([
        (
            s,
            NodeChaos {
                down: true,
                ..NodeChaos::default()
            },
        ),
        (
            a,
            NodeChaos {
                error_rate: 1.0,
                error_status: 500,
                ..NodeChaos::default()
            },
        ),
    ]);
    let cfg = rare();
    let rep = run(&t, &e, &chaos, &cfg).unwrap();
    assert_ne!(rep.requests.len(), 0);
    for r in &rep.requests {
        let live = execute(
            &t,
            &chaos.compile(),
            &ChainRequest {
                endpoint: e.get("GET /x").unwrap(),
                n: r.n,
                seed: cfg.seed,
                pin_delay: std::time::Duration::ZERO,
            },
        );
        assert_eq!(
            live.result,
            ChainResult::Err {
                node: s,
                status: 503,
                kind: FailKind::Status,
                retry_after: None
            }
        );
        assert_eq!(
            r.result,
            ChainResult::Err {
                node: s,
                status: 503,
                kind: FailKind::Status,
                retry_after: None
            }
        );
    }
}

/// One stream on `GET /x` at 0.01 req/s for 10 000 s: about 100 requests, rarely overlapping.
pub(crate) fn rare() -> SimConfig {
    SimConfig {
        streams: vec![Stream {
            endpoint_key: "GET /x".into(),
            rate_per_s: 0.01,
        }],
        duration: Duration::from_secs(10_000),
        warmup: Duration::ZERO,
        seed: 3,
    }
}

#[test]
fn cancelled_siblings_keep_their_slot_but_queued_ones_leave() {
    // Each request calls `slow` (one slot, 10 ms) twice in parallel next to a branch that
    // fails at 5 ms. The running call keeps its slot to 10 ms (cut to 5 ms in the record);
    // the queued call leaves its queue and never runs.
    let (t, e) = compiled(
        "version: 1\nnodes:\n  app: { type: service }\n  slow: { type: external-api, concurrency: 1, proc: 10ms }\n  wait: { type: external-api, proc: 5ms }\n  bad: { type: external-api }\nedges:\n  - { from: app, to: slow }\n  - { from: app, to: wait }\n  - { from: app, to: bad }\nendpoints:\n  \"GET /x\": { chain: [ { parallel: [ slow, slow, [ wait, bad ] ] } ] }\n",
        &[("GET /x", "app")],
    );
    let (slow, bad) = (t.id("slow").unwrap(), t.id("bad").unwrap());
    let down = ChaosState::from([(
        bad,
        NodeChaos {
            down: true,
            ..NodeChaos::default()
        },
    )]);
    let cfg = rare();
    let rep = run(&t, &e, &down, &cfg).unwrap();
    assert!(rep.requests.len() > 50, "{}", rep.requests.len());
    let mut busy = Duration::ZERO;
    for r in &rep.requests {
        assert_eq!(r.latency, Duration::from_millis(5));
        assert_eq!(
            r.result,
            ChainResult::Err {
                node: bad,
                status: 503,
                kind: FailKind::Status,
                retry_after: None
            }
        );
        let ran: Vec<_> = r
            .hops
            .iter()
            .filter(|(h, _)| h.node == slow && h.kind == HopKind::Proc)
            .collect();
        assert_eq!(
            ran.len(),
            1,
            "one call runs, the queued one never does: {r:?}"
        );
        for (h, _) in ran {
            assert_eq!(
                h.start + h.dur,
                Duration::from_millis(5),
                "cut at the failure: {h:?}"
            );
            let from = r.arrival + h.start;
            busy += cfg
                .duration
                .min(from + Duration::from_millis(10))
                .saturating_sub(from);
        }
    }
    // Utilization counts the full 10 ms of every call that started: the slot was kept.
    let util = rep.nodes[slow].utilization;
    assert!(
        (util * cfg.duration.as_secs_f64() - busy.as_secs_f64()).abs() < 1e-9,
        "{util} vs {busy:?}"
    );
}

/// `app` with fixed-cost external APIs: `a` 3 ms, `s` 4 ms.
pub(crate) const FORK: &str = "version: 1\nnodes:\n  app: { type: service }\n  a: { type: external-api, proc: 3ms }\n  s: { type: external-api, proc: 4ms }\nedges:\n  - { from: app, to: a }\n  - { from: app, to: s }\nendpoints:\n";

#[test]
fn fork_join_ends_with_the_slowest_branch() {
    let (t, e) = compiled(
        &format!("{FORK}  \"GET /x\": {{ chain: [ {{ parallel: [ a, s ] }} ] }}\n"),
        &[("GET /x", "app")],
    );
    let rep = run(&t, &e, &ChaosState::default(), &rare()).unwrap();
    assert_ne!(rep.requests.len(), 0);
    assert!(
        rep.requests
            .iter()
            .all(|r| r.latency == Duration::from_millis(4))
    );
}

#[test]
fn one_slot_serializes_parallel_branches() {
    let yaml = FORK.replace("proc: 4ms }", "proc: 4ms, concurrency: 1 }");
    let (t, e) = compiled(
        &format!("{yaml}  \"GET /x\": {{ chain: [ {{ parallel: [ s, s, s ] }} ] }}\n"),
        &[("GET /x", "app")],
    );
    let rep = run(&t, &e, &ChaosState::default(), &rare()).unwrap();
    let s = t.id("s").unwrap();
    let first = &rep.requests[0];
    let at_s: Vec<(Duration, Duration)> = first
        .hops
        .iter()
        .filter(|(h, _)| h.node == s && h.kind == HopKind::Proc)
        .map(|(h, w)| (h.start, *w))
        .collect();
    let ms = Duration::from_millis;
    assert_eq!(
        at_s,
        [(ms(0), ms(0)), (ms(4), ms(4)), (ms(8), ms(8))],
        "FIFO in branch order"
    );
    assert_eq!(first.latency, ms(12));
    assert!(rep.requests.iter().all(|r| r.latency >= ms(12)));
}

#[test]
fn failure_ties_go_to_the_lowest_branch_like_live() {
    // Both branches fail at 2 ms. The higher branch reaches its Fail first in event order
    // (its 2 ms hop was scheduled before the lower branch's second 1 ms hop), so the lower
    // branch must replace it at the same instant (D17).
    let (t, e) = compiled(
        "version: 1\nnodes:\n  app: { type: service }\n  p: { type: external-api, proc: 1ms }\n  q: { type: external-api, proc: 2ms }\n  s: { type: external-api }\n  a: { type: external-api }\nedges:\n  - { from: app, to: p }\n  - { from: app, to: q }\n  - { from: app, to: s }\n  - { from: app, to: a }\nendpoints:\n  \"GET /x\": { chain: [ { parallel: [ [ p, p, s ], [ q, a ] ] } ] }\n",
        &[("GET /x", "app")],
    );
    let (s, a) = (t.id("s").unwrap(), t.id("a").unwrap());
    let chaos = ChaosState::from([
        (
            s,
            NodeChaos {
                down: true,
                ..NodeChaos::default()
            },
        ),
        (
            a,
            NodeChaos {
                error_rate: 1.0,
                error_status: 500,
                ..NodeChaos::default()
            },
        ),
    ]);
    let cfg = rare();
    let rep = run(&t, &e, &chaos, &cfg).unwrap();
    assert_ne!(rep.requests.len(), 0);
    for r in &rep.requests {
        assert_eq!(
            r.result,
            ChainResult::Err {
                node: s,
                status: 503,
                kind: FailKind::Status,
                retry_after: None
            }
        );
        let live = execute(
            &t,
            &chaos.compile(),
            &ChainRequest {
                endpoint: e.get("GET /x").unwrap(),
                n: r.n,
                seed: cfg.seed,
                pin_delay: std::time::Duration::ZERO,
            },
        );
        let hops: Vec<_> = r.hops.iter().map(|(h, _)| h.clone()).collect();
        assert_eq!((hops, r.latency), (live.hops.clone(), live.total()));
    }
}

#[test]
fn branch_waits_under_load() {
    // R7.6: the queueing branch is exercised by parallel blocks, not only by sequences.
    let (t, e) = compiled(
        "version: 1\nnodes:\n  app: { type: service }\n  s: { type: service, concurrency: 1, proc: { mean: 10ms } }\nedges:\n  - { from: app, to: s }\nendpoints:\n  \"GET /x\": { chain: [ { parallel: [ s, s ] } ] }\n",
        &[("GET /x", "app")],
    );
    let cfg = SimConfig {
        streams: vec![Stream {
            endpoint_key: "GET /x".into(),
            rate_per_s: 30.0,
        }],
        duration: Duration::from_secs(100),
        warmup: Duration::from_secs(10),
        seed: 9,
    };
    let rep = run(&t, &e, &ChaosState::default(), &cfg).unwrap();
    let s = t.id("s").unwrap();
    assert!(
        rep.requests.iter().all(|r| r
            .hops
            .iter()
            .any(|(h, w)| h.node == s && h.kind == HopKind::Proc && !w.is_zero())),
        "the second branch always queues behind the first"
    );
    assert!(rep.nodes[s].mean_wait > Duration::ZERO);
}
