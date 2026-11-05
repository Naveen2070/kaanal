//! Connection set-up (RULES 68, step 6b): a call that opens a new connection pays the TCP and TLS
//! round trips and slow start; a call on a kept-alive connection pays nothing extra. The oracles
//! are the published round counts (RFC 9293, RFC 8446, RFC 5246, RFC 6928; see `exec/conn.rs`).

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
            at: std::time::Duration::ZERO,
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

/// The db edge's wire latency, and its `Connect` hops (request side, response side).
fn parts(t: &Topology, h: &[Hop]) -> (Duration, Option<Hop>, Option<Hop>) {
    let db = t.id("db").unwrap();
    let net = h
        .iter()
        .find(|x| x.node == db && x.kind == HopKind::Net)
        .unwrap()
        .dur;
    let find = |resp| {
        h.iter()
            .find(|x| x.kind == HopKind::Connect { resp })
            .cloned()
    };
    (net, find(false), find(true))
}

const NEW_EVERY_CALL: &str = ", net: 10ms, keepalive: 1";

#[test]
fn a_new_plain_connection_costs_one_round_trip_before_the_callee() {
    let (t, e) = build(&cfg(NEW_EVERY_CALL));
    let h = hops(&t, &e, 0, (0, 0));
    let (net, req, resp) = parts(&t, &h);
    let c = req.expect("a connect hop");
    assert_eq!(c.dur, net, "TCP: one round trip (RFC 9293)");
    assert!(resp.is_none(), "no body, so no slow-start stall");
    // it runs right before the wire latency
    let net_hop = h.iter().find(|x| x.kind == HopKind::Net && x.dur == net);
    assert_eq!(c.start + c.dur, net_hop.unwrap().start);
}

#[test]
fn tls_adds_one_round_trip_for_1_3_and_two_for_1_2() {
    for (tls, rtts) in [("1.3", 2), ("true", 2), ("1.2", 3)] {
        let (t, e) = build(&cfg(&format!("{NEW_EVERY_CALL}, tls: {tls}")));
        let (net, req, _) = parts(&t, &hops(&t, &e, 0, (0, 0)));
        assert_eq!(req.unwrap().dur, net * rtts, "tls: {tls}");
    }
    let (t, e) = build(&cfg(&format!("{NEW_EVERY_CALL}, tls: false")));
    let (net, req, _) = parts(&t, &hops(&t, &e, 0, (0, 0)));
    assert_eq!(req.unwrap().dur, net);
}

#[test]
fn a_big_request_on_a_new_connection_stalls_in_slow_start() {
    // 1 MiB on an unlimited wire: 7 rounds, 6 of them stall (see exec/conn.rs), plus TCP's one
    let (t, e) = build(&cfg(NEW_EVERY_CALL));
    let (net, req, resp) = parts(&t, &hops(&t, &e, 0, (1 << 20, 0)));
    assert_eq!(req.unwrap().dur, net * 7);
    assert!(resp.is_none());
    // and the same body coming back stalls on the way out
    let (net, req, resp) = parts(&t, &hops(&t, &e, 0, (0, 1 << 20)));
    assert_eq!(req.unwrap().dur, net, "the handshake only");
    let r = resp.expect("a response-side hop");
    assert_eq!(r.dur, net * 6);
}

#[test]
fn a_body_inside_the_first_window_does_not_stall() {
    let (t, e) = build(&cfg(NEW_EVERY_CALL));
    let (net, req, resp) = parts(&t, &hops(&t, &e, 0, (14_600, 14_600)));
    assert_eq!(req.unwrap().dur, net);
    assert!(resp.is_none());
}

#[test]
fn a_narrow_wire_is_full_from_the_first_window_so_it_does_not_stall() {
    // 1 KB/s over 10 ms is a 10 B pipe: the link hop is the limit, not the window
    let (t, e) = build(&cfg(", net: 10ms, keepalive: 1, bandwidth: 1KB"));
    let (net, req, _) = parts(&t, &hops(&t, &e, 0, (1 << 20, 0)));
    assert_eq!(req.unwrap().dur, net);
}

#[test]
fn no_net_means_no_round_trip_so_no_set_up_hop() {
    let (t, e) = build(&cfg(", keepalive: 1, tls: 1.2"));
    let h = hops(&t, &e, 0, (1 << 20, 1 << 20));
    assert!(h.iter().all(|x| !matches!(x.kind, HopKind::Connect { .. })));
}

/// How many of the first `n` calls opened a connection.
fn opened_in(t: &Topology, e: &kaanal_core::Endpoints, n: u64) -> f64 {
    let count = (0..n)
        .filter(|&i| {
            hops(t, e, i, (0, 0))
                .iter()
                .any(|x| matches!(x.kind, HopKind::Connect { .. }))
        })
        .count();
    f64::from(u32::try_from(count).unwrap())
}

#[test]
fn the_default_keepalive_opens_about_one_connection_in_a_thousand() {
    let (t, e) = build(&cfg(", net: 1ms"));
    let n = 200_000_u64;
    let opened = opened_in(&t, &e, n);
    // binomial(n, 1/1000): mean 200, sigma 14.1; five sigma either side
    assert!((130.0..=270.0).contains(&opened), "opened {opened}");
    // and a set keepalive moves it: 1 call in 10 is mean 20,000, sigma 134
    let (t, e) = build(&cfg(", net: 1ms, keepalive: 10"));
    let opened = opened_in(&t, &e, n);
    assert!((19_300.0..=20_700.0).contains(&opened), "opened {opened}");
}

#[test]
fn the_same_inputs_give_the_same_set_up() {
    let (t, e) = build(&cfg(", net: 1ms, keepalive: 3"));
    for n in 0..50 {
        assert_eq!(hops(&t, &e, n, (0, 0)), hops(&t, &e, n, (0, 0)));
    }
}

#[test]
fn tls_and_keepalive_are_checked_when_the_file_loads() {
    let load = |edge: &str| Config::from_yaml(&cfg(edge)).map_err(|e| e.to_string());
    assert!(load(", tls: 1.3, keepalive: 1000000").is_ok());
    for (edge, field) in [
        (", tls: 1.1", "tls"),
        (", tls: 2", "tls"),
        (", tls: yes", "tls"),
        (", keepalive: 0", "keepalive"),
        (", keepalive: 1000001", "keepalive"),
        (", keepalive: -3", "keepalive"),
        (", keepalive: fast", "keepalive"),
    ] {
        let err = load(edge).unwrap_err();
        assert!(err.contains(field), "{edge}: {err}");
    }
}
