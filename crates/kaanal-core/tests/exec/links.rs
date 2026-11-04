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
    let edge = u16::try_from(edge).unwrap();
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
    // GET /users/{id} reads: the 1 KB reply counts, the 1 KB request does not (RULES 65)
    assert_eq!(op(&hops(&t, &e, 1024, 1024)), ms(4) + ms(1));
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

#[test]
fn a_node_bandwidth_queues_on_the_narrowest_pipe_and_a_tie_goes_to_the_wire() {
    // app sends 500 kB/s, db receives 2 MB/s, the wire is 1 MB/s: the sender is narrowest.
    let (topo, eps) = build(&cfg(", bandwidth: { out: 500000 }", ", bandwidth: 1000000"));
    let (app, db) = (topo.id("app").unwrap(), topo.id("db").unwrap());
    let h = hops(&topo, &eps, 10_000, 0);
    let n: Vec<_> = h
        .iter()
        .filter(|h| {
            matches!(
                h.kind,
                HopKind::Nic { send: true, .. } | HopKind::Link { .. }
            )
        })
        .collect();
    assert_eq!(n.len(), 1, "one pipelined transfer, not one hop per pipe");
    assert_eq!(n[0].dur, ms(20), "10 KB over the narrowest, 500 kB/s");
    assert_eq!(
        n[0].kind,
        HopKind::Nic {
            send: true,
            resp: false
        }
    );
    assert_eq!(n[0].node, app, "the sender's pipe");
    // the response: db sends (no limit), app receives (no limit), only the wire: a link hop
    let h = hops(&topo, &eps, 0, 10_000);
    let l = links(&h);
    assert_eq!((l.len(), l[0].dur, l[0].node), (1, ms(10), db));
    // equal rates: the wire wins the tie
    let (topo, eps) = build(&cfg(", bandwidth: 1000000", ", bandwidth: 1000000"));
    let h = hops(&topo, &eps, 10_000, 0);
    assert_eq!(links(&h).len(), 1);
    assert!(
        !h.iter()
            .any(|h| matches!(h.kind, HopKind::Nic { send: true, .. }))
    );
}

#[test]
fn a_receiving_pipe_binds_the_response_and_belongs_to_the_caller() {
    // the caller (app) receives only 250 kB/s; the response comes back to it
    let (t, e) = build(&cfg(", bandwidth: { in: 250000 }", ""));
    let app = t.id("app").unwrap();
    let h = hops(&t, &e, 0, 10_000);
    let n: Vec<_> = h
        .iter()
        .filter(|h| matches!(h.kind, HopKind::Nic { .. }))
        .collect();
    assert_eq!(n.len(), 1);
    assert_eq!((n[0].dur, n[0].node), (ms(40), app));
    assert_eq!(
        n[0].kind,
        HopKind::Nic {
            send: false,
            resp: true
        }
    );
}
