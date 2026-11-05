//! Loss and compression on a wire (RULES 74, step 6d). Off by default. The oracles are the
//! published models (see `exec/loss.rs`): fast retransmit and the retransmission timer for short
//! transfers, the RFC 5348 rate for long ones, and the handshake's 1 s SYN timeout.

use crate::common::*;
use kaanal_core::{ChainRequest, ChaosState, Config, Hop, HopKind, PlanItem, Topology, execute};
use std::time::Duration;

/// A gateway, an app and a database: the db edge is the one under test, `edge` its extra fields.
fn cfg(edge: &str) -> String {
    format!(
        "version: 1\nnodes:\n  nginx: {{ type: gateway }}\n  app: {{ type: service }}\n  db: {{ type: database, ops: {{ read: 4ms, write: 8ms, scan: 30ms }} }}\nedges:\n  - {{ from: nginx, to: app }}\n  - {{ from: app, to: db{edge} }}\nendpoints:\n  \"GET /users/{{id}}\":\n    chain: [{{ call: db, op: read }}]\n"
    )
}

fn hops(t: &Topology, e: &kaanal_core::Endpoints, n: u64, (req, resp): (u64, u64)) -> Vec<Hop> {
    execute(
        t,
        &ChaosState::default().compile(),
        &ChainRequest {
            endpoint: e.get("GET /users/{id}").unwrap(),
            n,
            seed: 42,
            pin_delay: Duration::ZERO,
            req_bytes: req,
            resp_bytes: resp,
        },
    )
    .plan
    .items
    .into_iter()
    .filter_map(|i| match i {
        PlanItem::Hop(h) => Some(h),
        _ => None,
    })
    .collect()
}

fn find(h: &[Hop], kind: HopKind) -> Option<Hop> {
    h.iter().find(|x| x.kind == kind).cloned()
}

const MIB: u64 = 1 << 20;

#[test]
fn nothing_changes_while_loss_and_compression_are_unset() {
    let (t, e) = build(&cfg(", net: 10ms"));
    for n in 0..300 {
        let h = hops(&t, &e, n, (50_000, 50_000));
        assert!(
            h.iter()
                .all(|x| !matches!(x.kind, HopKind::Retransmit { .. })),
            "call {n} has a retransmit hop"
        );
    }
}

#[test]
fn a_one_segment_body_that_is_always_lost_waits_the_retransmission_timer() {
    // 1 segment: no duplicate ACKs, so the loss is a timeout: max(200 ms, 3 x 10 ms) = 200 ms
    let (t, e) = build(&cfg(", net: 10ms, loss: 1"));
    let h = hops(&t, &e, 0, (1, 1));
    for resp in [false, true] {
        let r = find(&h, HopKind::Retransmit { resp }).expect("a retransmit hop");
        assert_eq!(r.dur, Duration::from_millis(200), "resp = {resp}");
    }
    // a long round trip: 3 x 100 ms = 300 ms (RFC 6298's first-sample timeout)
    let (t, e) = build(&cfg(", net: 100ms, loss: 1"));
    let r = find(&hops(&t, &e, 0, (0, 1)), HopKind::Retransmit { resp: true }).unwrap();
    assert_eq!(r.dur, Duration::from_millis(300));
}

#[test]
fn no_net_means_no_round_trip_so_loss_costs_nothing() {
    let (t, e) = build(&cfg(", loss: 1"));
    let h = hops(&t, &e, 0, (1, 1));
    assert!(
        h.iter()
            .all(|x| !matches!(x.kind, HopKind::Retransmit { .. }))
    );
}

#[test]
fn a_lost_syn_waits_one_second_each() {
    // a new plain connection sends two handshake packets; both lost at loss = 1: 2 x 1 s on top
    // of TCP's one round trip
    let (t, e) = build(&cfg(", net: 10ms, loss: 1, keepalive: 1"));
    let c = find(&hops(&t, &e, 0, (0, 0)), HopKind::Connect { resp: false }).unwrap();
    assert_eq!(c.dur, Duration::from_millis(10) + Duration::from_secs(2));
    // TLS 1.3 sends four packets
    let (t, e) = build(&cfg(", net: 10ms, loss: 1, keepalive: 1, tls: 1.3"));
    let c = find(&hops(&t, &e, 0, (0, 0)), HopKind::Connect { resp: false }).unwrap();
    assert_eq!(c.dur, Duration::from_millis(20) + Duration::from_secs(4));
}

#[test]
fn a_long_flow_runs_at_the_rfc_5348_rate() {
    // 1 MiB is 719 segments; at p = 5 % that is 36 expected losses, over the 32 that make it a long
    // flow. Unlimited wire, so the whole transfer time is the loss-limited one: bytes / X, with X
    // from RFC 5348 section 3.1 (b = 1, s = 1460, t_RTO = max(0.2, 4R)), computed here independently.
    let (topo, eps) = build(&cfg(", net: 10ms, loss: 0.05"));
    let hop = find(
        &hops(&topo, &eps, 0, (0, MIB)),
        HopKind::Retransmit { resp: true },
    )
    .expect("a hop");
    let (seg, rtt, prob) = (1460.0_f64, 0.01_f64, 0.05_f64);
    let t_rto = (4.0 * rtt).max(0.2);
    let rate = seg
        / (rtt * (2.0 * prob / 3.0).powf(0.5)
            + t_rto * 3.0 * (3.0 * prob / 8.0).powf(0.5) * prob * (1.0 + 32.0 * prob.powi(2)));
    let want = f64::from(u32::try_from(MIB).unwrap()) / rate;
    let got = hop.dur.as_secs_f64();
    assert!(
        (got - want).abs() / want < 1e-3,
        "{got} s vs {want} s at {rate:.0} B/s"
    );
}

#[test]
fn a_pipe_already_slower_than_the_loss_limit_adds_nothing_for_a_long_flow() {
    // 1 KB/s is far below the loss-limited rate, so loss changes nothing for the long flow
    let (t, e) = build(&cfg(", net: 10ms, loss: 0.05, bandwidth: 1KB"));
    let h = hops(&t, &e, 0, (0, MIB));
    assert!(find(&h, HopKind::Retransmit { resp: true }).is_none());
}

#[test]
fn a_short_flow_pays_the_binomial_mean_of_the_timeouts() {
    // one segment lost with p = 0.1 and no duplicate ACKs: each request waits RTO with chance p, so
    // the mean extra is p x RTO = 20 ms; the draw is Bernoulli, so the mean of 20,000 calls is
    // within five standard errors (sigma = RTO sqrt(p (1 - p)) = 60 ms, over sqrt(20,000))
    let (t, e) = build(&cfg(", net: 10ms, loss: 0.1"));
    let n = 20_000_u64;
    let total: f64 = (0..n)
        .map(|i| {
            find(&hops(&t, &e, i, (0, 1)), HopKind::Retransmit { resp: true })
                .map_or(0.0, |r| r.dur.as_secs_f64())
        })
        .sum();
    let mean = total / 20_000.0;
    let se = 0.2 * (0.1_f64 * 0.9).sqrt() / 20_000.0_f64.sqrt();
    assert!(
        (mean - 0.02).abs() < 5.0 * se,
        "mean {mean} s, want 0.020 +- {se}"
    );
}

#[test]
fn compression_shrinks_the_response_on_the_wire_and_not_the_request() {
    // a 1 MiB/s wire, no headers: 1 MiB takes 1 s; compress 0.5 sends half of a response
    let (t, e) = build(&cfg(", bandwidth: 1MB, compress: 0.5"));
    let h = hops(&t, &e, 0, (MIB, MIB));
    let req = find(
        &h,
        HopKind::Link {
            edge: 1,
            resp: false,
        },
    )
    .unwrap();
    let resp = find(
        &h,
        HopKind::Link {
            edge: 1,
            resp: true,
        },
    )
    .unwrap();
    assert_eq!(
        req.dur,
        Duration::from_secs(1),
        "a request is not compressed"
    );
    assert_eq!(
        resp.dur,
        Duration::from_millis(500),
        "half the bytes, half the time"
    );
}

#[test]
fn loss_and_compression_are_checked_when_the_file_loads() {
    let load = |edge: &str| Config::from_yaml(&cfg(edge)).map_err(|e| e.to_string());
    assert!(load(", loss: 0, compress: 1").is_ok());
    assert!(load(", loss: 1, compress: 0.001").is_ok());
    for (edge, field) in [
        (", loss: 1.5", "loss"),
        (", loss: -0.1", "loss"),
        (", loss: lots", "loss"),
        (", compress: 0", "compress"),
        (", compress: 1.5", "compress"),
        (", compress: small", "compress"),
    ] {
        let err = load(edge).unwrap_err();
        assert!(err.contains(field), "{edge}: {err}");
    }
}
