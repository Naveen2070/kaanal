//! The storage node in the plan: `first_byte` plus bytes over `throughput`, on the existing
//! `proc` and `per_kb` hops (RULES 61).

use crate::common::*;
use kaanal_core::{ChainRequest, ChaosState, Hop, HopKind, PlanItem, Topology, execute};
use std::time::Duration;

/// A service `app` behind a gateway, calling the store `files` and, last, a database.
fn cfg(files: &str, chain: &str) -> String {
    format!(
        "version: 1\nnodes:\n  nginx: {{ type: gateway }}\n  app: {{ type: service }}\n  files: {{ type: storage, {files} }}\n  db: {{ type: database, ops: {{ read: 4ms, write: 8ms, scan: 30ms }} }}\nedges:\n  - {{ from: nginx, to: app }}\n  - {{ from: app, to: files }}\n  - {{ from: app, to: db }}\n{chain}"
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

/// The storage hop's length (its `Proc` hop).
fn files_hop(t: &Topology, h: &[Hop]) -> Duration {
    let id = t.id("files").unwrap();
    let mine: Vec<&Hop> = h
        .iter()
        .filter(|h| h.node == id && h.kind == HopKind::Proc)
        .collect();
    assert_eq!(mine.len(), 1, "one hop at the store: {h:?}");
    mine[0].dur
}

const FIXED: &str = "first_byte: 20ms, throughput: 100MB/s";

/// The same numbers, as the arithmetic: 100MB/s = 104857600 B/s, 1 KB = 1024 B, so one KB takes
/// 1024 / 104857600 s = 9765.625 ns, which the engine holds as 9766 ns.
const PER_KB_NS: u64 = 9_766;

#[test]
fn with_no_sizes_the_hop_is_exactly_first_byte() {
    let (t, e) = build(&cfg(FIXED, ""));
    assert_eq!(files_hop(&t, &hops(&t, &e, 0, 0)), ms(20));
}

#[test]
fn a_five_megabyte_response_adds_bytes_over_throughput() {
    let (t, e) = build(&cfg(FIXED, ""));
    // 5 MB = 5242880 B over 104857600 B/s = 50 ms, on 20 ms
    let d = files_hop(&t, &hops(&t, &e, 0, 5 * 1024 * 1024));
    let want = ms(20) + Duration::from_nanos(PER_KB_NS * 5 * 1024);
    assert_eq!(d, want);
    let off = d.abs_diff(ms(70));
    assert!(off < Duration::from_micros(10), "about 70 ms: {d:?}");
}

#[test]
fn request_and_response_each_count_once() {
    let (t, e) = build(&cfg(FIXED, ""));
    let both = files_hop(&t, &hops(&t, &e, 1024 * 1024, 4 * 1024 * 1024));
    let resp = files_hop(&t, &hops(&t, &e, 0, 5 * 1024 * 1024));
    let req = files_hop(&t, &hops(&t, &e, 5 * 1024 * 1024, 0));
    assert_eq!(both, resp, "5 MB in two bodies costs what 5 MB in one does");
    assert_eq!(both, req);
}

#[test]
fn a_five_kilobyte_object_adds_under_a_millisecond() {
    let (t, e) = build(&cfg(FIXED, ""));
    let d = files_hop(&t, &hops(&t, &e, 0, 5 * 1024));
    assert_eq!(d, ms(20) + Duration::from_nanos(PER_KB_NS * 5));
    assert!(d.abs_diff(ms(20)) < ms(1), "{d:?}");
}

#[test]
fn without_throughput_sizes_cost_nothing() {
    let (t, e) = build(&cfg("first_byte: 20ms", ""));
    assert_eq!(files_hop(&t, &hops(&t, &e, 0, 5 * 1024 * 1024)), ms(20));
}

#[test]
fn a_chain_call_with_an_op_costs_the_same_for_read_write_and_scan() {
    let mut seen = Vec::new();
    for op in ["read", "write", "scan"] {
        let chain = format!(
            "endpoints:\n  \"GET /users/{{id}}\": {{ chain: [ {{ call: files, op: {op} }} ] }}\n"
        );
        let (t, e) = build(&cfg(FIXED, &chain));
        let h = hops(&t, &e, 0, 5 * 1024 * 1024);
        assert!(
            h.iter().all(|h| !matches!(h.kind, HopKind::Op(_))),
            "a storage node has no op hop: {h:?}"
        );
        seen.push(files_hop(&t, &h));
    }
    assert_eq!(seen[0], seen[1]);
    assert_eq!(seen[1], seen[2]);
    assert!(
        seen[0].abs_diff(ms(70)) < Duration::from_micros(10),
        "{:?}",
        seen[0]
    );
}

#[test]
fn a_bare_chain_step_calls_the_store_and_a_service_body_calls_it_by_default() {
    let chain = "endpoints:\n  \"GET /users/{id}\": { chain: [ files ] }\n";
    let (t, e) = build(&cfg(FIXED, chain));
    assert_eq!(files_hop(&t, &hops(&t, &e, 0, 0)), ms(20));
    // no endpoint entry: the service's default body calls the store, then the database
    let (t, e) = build(&cfg(FIXED, ""));
    let h = hops(&t, &e, 0, 0);
    let work: Vec<&str> = h
        .iter()
        .filter(|h| h.kind == HopKind::Proc || matches!(h.kind, HopKind::Op(_)))
        .map(|h| t.nodes[h.node].name.as_str())
        .collect();
    assert_eq!(work, ["nginx", "app", "files", "db"]);
}
