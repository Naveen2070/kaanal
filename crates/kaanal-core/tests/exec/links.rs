//! Payload cost in the plan: link hops (`bandwidth`) and the `per_kb` surcharge.

use crate::common::*;
use kaanal_core::{ChainRequest, ChaosState, Hop, HopKind, PlanItem, Topology, execute};
use std::time::Duration;

/// `edge_f` on the app-to-db edge, `app_f` on the app node.
fn cfg(app_f: &str, edge_f: &str) -> String {
    format!(
        "version: 1\nnodes:\n  nginx: {{ type: gateway, proc: 1ms }}\n  app: {{ type: service, proc: 5ms{app_f} }}\n  db: {{ type: database, ops: {{ read: 4ms, write: 8ms, scan: 30ms }} }}\nedges:\n  - {{ from: nginx, to: app, net: 1ms }}\n  - {{ from: app, to: db, net: 1ms{edge_f} }}\nendpoints:\n  \"GET /users/{{id}}\": {{ chain: [ db ] }}\n"
    )
}

fn hops(t: &Topology, e: &kaanal_core::Endpoints, req: u64, resp: u64) -> Vec<Hop> {
    execute(
        t,
        &ChaosState::default().compile(),
        &ChainRequest {
            endpoint: e.get("GET /users/{id}").unwrap(),
            n: 0,
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

fn links(h: &[Hop]) -> Vec<&Hop> {
    h.iter()
        .filter(|h| matches!(h.kind, HopKind::Link { .. }))
        .collect()
}

#[test]
fn no_bandwidth_means_no_link_hops() {
    let (t, e) = build(&cfg("", ""));
    assert_eq!(links(&hops(&t, &e, 10_000, 10_000)).len(), 0);
}

#[test]
fn zero_bytes_means_no_link_hops() {
    let (t, e) = build(&cfg("", ", bandwidth: 1000000"));
    assert_eq!(links(&hops(&t, &e, 0, 0)).len(), 0);
}

#[test]
fn a_link_hop_lasts_bytes_over_bandwidth() {
    let (t, e) = build(&cfg("", ", bandwidth: 1000000"));
    let h = hops(&t, &e, 10_000, 0);
    let l = links(&h);
    assert_eq!(l.len(), 1, "only the request direction has bytes");
    assert_eq!(l[0].dur, ms(10), "10 KB over 1 MB/s");
    assert_eq!(
        l[0].node,
        t.id("db").unwrap(),
        "belongs to the edge's target"
    );
    let edge = t
        .edge_id(t.id("app").unwrap(), t.id("db").unwrap())
        .unwrap();
    assert_eq!(l[0].kind, HopKind::Link { edge, resp: false });
}

#[test]
fn request_and_response_each_get_a_hop() {
    let (t, e) = build(&cfg("", ", bandwidth: 1000000"));
    let h = hops(&t, &e, 10_000, 20_000);
    let i = |p: fn(&Hop) -> bool| h.iter().position(p).unwrap();
    let (req, net, op, resp) = (
        i(|h| matches!(h.kind, HopKind::Link { resp: false, .. })),
        h.iter().rposition(|h| h.kind == HopKind::Net).unwrap(),
        i(|h| matches!(h.kind, HopKind::Op(_))),
        i(|h| matches!(h.kind, HopKind::Link { resp: true, .. })),
    );
    assert!(
        req < net && net < op && op < resp,
        "{req} {net} {op} {resp}"
    );
    assert_eq!((h[req].dur, h[resp].dur), (ms(10), ms(20)));
    // each hop starts where the one before it ended
    assert_eq!(h[net].start, h[req].start + h[req].dur);
    assert_eq!(h[resp].start, h[op].start + h[op].dur);
}

#[test]
fn transfer_is_capped_at_10_hours() {
    let (t, e) = build(&cfg("", ", bandwidth: 1"));
    let h = hops(&t, &e, 1_000_000_000, 0);
    assert_eq!(links(&h)[0].dur, Duration::from_hours(10));
}

#[test]
fn per_kb_adds_to_proc_once_per_body() {
    let (t, e) = build(&cfg(", per_kb: 1ms", ""));
    let proc_of = |h: &[Hop]| {
        h.iter()
            .find(|h| h.node == t.id("app").unwrap() && h.kind == HopKind::Proc)
            .unwrap()
            .dur
    };
    let base = proc_of(&hops(&t, &e, 0, 0));
    assert_eq!(base, ms(5));
    // 3 KB + 1 KB at 1 ms per KB
    let h = hops(&t, &e, 3 * 1024, 1024);
    assert_eq!(proc_of(&h), ms(5) + ms(4));
    assert_eq!(
        h.iter()
            .filter(|h| h.kind == HopKind::Proc && h.node == t.id("app").unwrap())
            .count(),
        1
    );
}

#[test]
fn per_kb_on_a_database_adds_to_its_op_hop() {
    let src = cfg("", "").replace(
        "db: { type: database,",
        "db: { type: database, per_kb: 1ms,",
    );
    let (t, e) = build(&src);
    let op = |h: &[Hop]| {
        h.iter()
            .find(|h| matches!(h.kind, HopKind::Op(_)))
            .unwrap()
            .dur
    };
    assert_eq!(op(&hops(&t, &e, 0, 0)), ms(4));
    assert_eq!(op(&hops(&t, &e, 1024, 1024)), ms(4) + ms(2));
}

#[test]
fn a_cache_edge_has_link_hops_and_a_miss_has_no_response_transfer() {
    let src = |ratio: &str| {
        format!(
            "version: 1\nnodes:\n  nginx: {{ type: gateway }}\n  app: {{ type: service }}\n  c: {{ type: cache, hit_ratio: {ratio}, hit: 1ms, miss: 1ms }}\nedges:\n  - {{ from: nginx, to: app }}\n  - {{ from: app, to: c, bandwidth: 1000000 }}\nendpoints:\n  \"GET /users/{{id}}\": {{ chain: [ {{ cache: c }} ] }}\n"
        )
    };
    let dirs = |ratio| {
        let (t, e) = build(&src(ratio));
        let h = hops(&t, &e, 10_000, 20_000);
        links(&h)
            .iter()
            .map(|h| (matches!(h.kind, HopKind::Link { resp: true, .. }), h.dur))
            .collect::<Vec<_>>()
    };
    assert_eq!(dirs("1.0"), [(false, ms(10)), (true, ms(20))], "a hit");
    assert_eq!(dirs("0.0"), [(false, ms(10))], "a miss returns nothing");
}
