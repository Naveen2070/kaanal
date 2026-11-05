//! Stacked chaos layers: banded failures, determinism and latency clamps.

use crate::common::*;
use kaanal_core::{
    ChainOutcome, ChainRequest, ChainResult, ChaosKey, ChaosLayer, ChaosState, ErrorKind, FailKind,
    Topology, execute,
};
use std::time::Duration;

pub(crate) fn layer_on(
    t: &Topology,
    node: &str,
    rate: f64,
    kind: ErrorKind,
    status: u16,
) -> ChaosState {
    let mut s = ChaosState::default();
    s.set(
        ChaosKey::Node(t.id(node).unwrap()),
        ChaosLayer {
            error_rate: Some(rate),
            error_kind: Some(kind),
            error_status: Some(status),
            ..ChaosLayer::default()
        },
    );
    s
}

pub(crate) fn fail_kind(o: &ChainOutcome) -> Option<(FailKind, u16)> {
    match o.result {
        ChainResult::Err { kind, status, .. } => Some((kind, status)),
        ChainResult::Ok => None,
    }
}

#[test]
fn a_banded_mix_fails_with_each_kind_at_its_share() {
    // 30% reset on top of 30% 503: the top layer owns the first 30% of rolls, the lower
    // layer the next 21% (0.3 + 0.3 - 0.09 = 0.51).
    let (t, e) = topo();
    let (low, top) = (
        layer_on(&t, "db", 0.3, ErrorKind::Status, 503),
        layer_on(&t, "db", 0.3, ErrorKind::Reset, 502),
    );
    let eff = kaanal_core::compile_layers(&[&low, &top]);
    let (mut reset, mut status) = (0u32, 0u32);
    for n in 0..2000 {
        let o = execute(
            &t,
            &eff,
            &ChainRequest {
                at: std::time::Duration::ZERO,
                endpoint: e.get("GET /users/{id}").unwrap(),
                n,
                seed: 42,
                pin_delay: std::time::Duration::ZERO,
                req_bytes: 0,
                resp_bytes: 0,
            },
        );
        match fail_kind(&o) {
            Some((FailKind::Reset, 502)) => reset += 1,
            Some((FailKind::Status, 503)) => status += 1,
            None => {}
            other => panic!("unexpected failure {other:?}"),
        }
    }
    let pct = |c: u32| f64::from(c) / 20.0;
    assert!((pct(reset) - 30.0).abs() < 3.0, "reset {}%", pct(reset));
    assert!((pct(status) - 21.0).abs() < 3.0, "status {}%", pct(status));
}

#[test]
fn a_stack_is_deterministic_and_a_single_layer_ignores_the_stack() {
    let (t, e) = topo();
    let ep = e.get("GET /users/{id}").unwrap();
    let out = |eff: &kaanal_core::EffectiveChaos, n| {
        execute(
            &t,
            eff,
            &ChainRequest {
                at: std::time::Duration::ZERO,
                endpoint: ep,
                n,
                seed: 42,
                pin_delay: std::time::Duration::ZERO,
                req_bytes: 0,
                resp_bytes: 0,
            },
        )
    };
    let (low, top) = (
        layer_on(&t, "db", 0.3, ErrorKind::Status, 503),
        layer_on(&t, "db", 0.3, ErrorKind::Reset, 502),
    );
    let stacked = kaanal_core::compile_layers(&[&low, &top]);
    let again = kaanal_core::compile_layers(&[&low, &top]);
    for n in 0..500 {
        assert_eq!(out(&stacked, n), out(&again, n), "n={n}");
    }
    // One layer alone (the pre-stack compile) gives the same outcome as that layer inside a
    // stack of empty layers, for every failure kind.
    let empty = ChaosState::default();
    for kind in [
        ErrorKind::Status,
        ErrorKind::Reset,
        ErrorKind::Hang,
        ErrorKind::Malformed,
    ] {
        let one = layer_on(&t, "db", 0.5, kind, 504);
        let alone = one.compile();
        let wrapped = kaanal_core::compile_layers(&[&empty, &one, &empty]);
        let mut failed = 0;
        for n in 0..500 {
            let o = out(&alone, n);
            failed += u32::from(fail_kind(&o).is_some());
            assert_eq!(o, out(&wrapped, n), "{kind:?} n={n}");
        }
        assert!((150..350).contains(&failed), "{kind:?}: {failed}/500");
    }
}

#[test]
fn stacked_latency_adds_up_and_clamps() {
    let (t, _) = topo();
    let lat = |d: u64| {
        let mut s = ChaosState::default();
        s.set(
            ChaosKey::Node(t.id("db").unwrap()),
            ChaosLayer {
                latency_add: Some(Duration::from_secs(d)),
                ..ChaosLayer::default()
            },
        );
        s
    };
    let (ten, twenty) = (lat(10), lat(20));
    let eff = kaanal_core::compile_layers(&[&ten, &twenty]);
    let at = |eff: &kaanal_core::EffectiveChaos| {
        eff.visit(t.id("db").unwrap(), "x").unwrap().latency_add
    };
    assert_eq!(at(&eff), Duration::from_secs(30));
    let (big, bigger) = (lat(200), lat(200));
    let eff = kaanal_core::compile_layers(&[&big, &bigger]);
    assert_eq!(at(&eff), kaanal_core::MAX_LATENCY_ADD);
}
