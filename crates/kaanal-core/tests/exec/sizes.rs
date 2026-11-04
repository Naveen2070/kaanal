//! Payload sizes per hop (RULES 65, step 6e): `sends` and `returns` on an edge or a `call:` step
//! set what a call carries; unset passes the incoming size on.

use crate::common::*;
use kaanal_core::{ChainRequest, ChaosState, Hop, HopKind, PlanItem, Topology, execute};
use std::time::Duration;

/// 100 MB/s = 104857600 B/s, so 1 KB (1024 B) is 9765.625 ns, which the engine holds as 9766 ns.
const PER_KB_NS: u64 = 9_766;

/// A gateway, a service `app`, a store `files` and a database `db` that charges 1 ms a KB. `edges`
/// and `chain` are the parts the tests vary.
fn cfg(edges: &str, chain: &str) -> String {
    format!(
        "version: 1\nnodes:\n  nginx: {{ type: gateway }}\n  app: {{ type: service }}\n  files: {{ type: storage, first_byte: 20ms, throughput: 100MB/s }}\n  db: {{ type: database, per_kb: 1ms, ops: {{ read: 4ms, write: 8ms, scan: 30ms }} }}\nedges:\n  - {{ from: nginx, to: app }}\n{edges}{chain}"
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

fn hop_of(t: &Topology, h: &[Hop], node: &str, kind: HopKind) -> Duration {
    let id = t.id(node).unwrap();
    let mine: Vec<&Hop> = h
        .iter()
        .filter(|h| h.node == id && h.kind == kind)
        .collect();
    assert_eq!(mine.len(), 1, "one {kind:?} hop at {node}: {h:?}");
    mine[0].dur
}

const KB: u64 = 1024;
const MB: u64 = 1024 * KB;

/// `app` calls the store and then the database, as steps.
const BOTH: &str = "endpoints:\n  \"GET /users/{id}\":\n    chain: [{ call: files, op: %OP%FILES }, { call: db, op: write }]\n";

/// The store called with `op` (an upload writes, a download reads), then the database writes.
fn both_op(op: &str, files_step: &str) -> String {
    BOTH.replace("%OP", op).replace("%FILES", files_step)
}

fn both(files_step: &str) -> String {
    both_op("write", files_step)
}

const EDGES: &str = "  - { from: app, to: files%E }\n  - { from: app, to: db }\n";

fn edges(extra: &str) -> String {
    EDGES.replace("%E", extra)
}

#[test]
fn unset_passes_the_incoming_size_on() {
    let (t, e) = build(&cfg(&edges(""), &both("")));
    let h = hops(&t, &e, 5 * MB, 0);
    // 5 MiB over 100 MB/s: 5120 KB x 9766 ns on top of the 20 ms first byte
    assert_eq!(
        hop_of(&t, &h, "files", HopKind::Proc),
        ms(20) + Duration::from_nanos(5120 * PER_KB_NS)
    );
}

#[test]
fn a_step_that_sends_less_charges_the_callee_for_that_size() {
    // a 5 MiB upload becomes a 200 KB call to the store
    let (t, e) = build(&cfg(&edges(""), &both(", sends: 200KB")));
    let h = hops(&t, &e, 5 * MB, 0);
    assert_eq!(
        hop_of(&t, &h, "files", HopKind::Proc),
        ms(20) + Duration::from_nanos(200 * PER_KB_NS)
    );
}

#[test]
fn the_caller_sizes_come_back_after_the_call() {
    // the database write after the store call still sees the 5 MiB request, at 1 ms a KB
    let (t, e) = build(&cfg(&edges(""), &both(", sends: 200KB")));
    let h = hops(&t, &e, 5 * MB, 0);
    assert_eq!(
        hop_of(&t, &h, "db", HopKind::Op(kaanal_core::Op::Write)),
        ms(8) + Duration::from_millis(5 * 1024)
    );
}

#[test]
fn an_edge_sets_the_size_of_every_call_over_it_and_a_step_beats_the_edge() {
    let (t, e) = build(&cfg(&edges(", sends: 100KB"), &both("")));
    let h = hops(&t, &e, 5 * MB, 0);
    assert_eq!(
        hop_of(&t, &h, "files", HopKind::Proc),
        ms(20) + Duration::from_nanos(100 * PER_KB_NS)
    );
    let (t, e) = build(&cfg(&edges(", sends: 100KB"), &both(", sends: 50KB")));
    let h = hops(&t, &e, 5 * MB, 0);
    assert_eq!(
        hop_of(&t, &h, "files", HopKind::Proc),
        ms(20) + Duration::from_nanos(50 * PER_KB_NS)
    );
}

#[test]
fn returns_sets_the_response_a_call_brings_back() {
    // the store sends a 1 MiB object whatever the client asked for
    let (t, e) = build(&cfg(&edges(", returns: 1MB"), &both_op("read", "")));
    let h = hops(&t, &e, 0, 0);
    assert_eq!(
        hop_of(&t, &h, "files", HopKind::Proc),
        ms(20) + Duration::from_nanos(1024 * PER_KB_NS)
    );
}

#[test]
fn a_size_with_a_bad_unit_is_a_load_error_naming_the_field() {
    let err = kaanal_core::Config::from_yaml(&cfg(&edges(", sends: 5XB"), &both("")))
        .unwrap_err()
        .to_string();
    assert!(err.contains("sends"), "{err}");
}

#[test]
fn a_database_write_ignores_the_response_size_and_a_read_the_request() {
    let big = 5 * MB;
    // the chain's last call is `db` with op write
    let (t, e) = build(&cfg(&edges(""), &both("")));
    let write = |req, resp| {
        hop_of(
            &t,
            &hops(&t, &e, req, resp),
            "db",
            HopKind::Op(kaanal_core::Op::Write),
        )
    };
    assert_eq!(write(0, big), ms(8), "a write does not send the big reply");
    assert_eq!(write(big, 0), ms(8) + Duration::from_millis(5 * 1024));
    let chain = "endpoints:
  \"GET /users/{id}\":
    chain: [{ call: db, op: read }]
";
    let (t, e) = build(&cfg(&edges(""), chain));
    let read = |req, resp| {
        hop_of(
            &t,
            &hops(&t, &e, req, resp),
            "db",
            HopKind::Op(kaanal_core::Op::Read),
        )
    };
    assert_eq!(read(big, 0), ms(4), "a read does not take a big request");
    assert_eq!(read(0, big), ms(4) + Duration::from_millis(5 * 1024));
}

#[test]
fn a_listing_is_one_request_a_page_each_paying_first_byte_and_its_page() {
    // RULES 61 point 5: a LIST returns one page (S3: at most 1,000 keys), so a listing of
    // several pages is `repeat: pages` with `returns: page size`, all on the existing machinery.
    let chain = "endpoints:\n  \"GET /users/{id}\":\n    chain: [{ call: files, op: scan, repeat: 3, returns: 200KB }]\n";
    let (t, e) = build(&cfg(&edges(""), chain));
    let id = t.id("files").unwrap();
    let pages: Vec<Duration> = hops(&t, &e, 0, 0)
        .iter()
        .filter(|h| h.node == id && h.kind == HopKind::Proc)
        .map(|h| h.dur)
        .collect();
    assert_eq!(
        pages,
        vec![ms(20) + Duration::from_nanos(200 * PER_KB_NS); 3]
    );
}

// ---- headers (RULES 65 point 3) ----

/// A gateway, an app and a database, with a 1000 B/s wire from the gateway to the app: 1 B takes 1 ms.
fn wire(top: &str) -> String {
    format!(
        "version: 1\n{top}nodes:\n  nginx: {{ type: gateway }}\n  app: {{ type: service }}\n  db: {{ type: database, ops: {{ read: 4ms, write: 8ms, scan: 30ms }} }}\nedges:\n  - {{ from: nginx, to: app, bandwidth: 1000 }}\n  - {{ from: app, to: db }}\n"
    )
}

/// The `(request, response)` durations of the wire transfers.
fn wire_ms(t: &Topology, h: &[Hop]) -> (Option<Duration>, Option<Duration>) {
    let pick = |resp: bool| {
        h.iter()
            .find(|h| matches!(h.kind, HopKind::Link { resp: r, .. } if r == resp))
            .map(|h| h.dur)
    };
    let _ = t;
    (pick(false), pick(true))
}

#[test]
fn by_default_a_call_with_no_body_still_carries_its_headers() {
    // 600 B in, 400 B back over 1000 B/s: 600 ms and 400 ms, though the bodies are empty
    let (t, e) = build_raw(&wire(""));
    assert_eq!(t.header_bytes, (600, 400));
    assert_eq!(
        wire_ms(&t, &hops(&t, &e, 0, 0)),
        (Some(ms(600)), Some(ms(400)))
    );
}

#[test]
fn a_body_and_its_headers_add_up() {
    let (t, e) = build_raw(&wire(""));
    // a 1000 B body: 1600 ms in; the 2000 B reply: 2400 ms back
    assert_eq!(
        wire_ms(&t, &hops(&t, &e, 1000, 2000)),
        (Some(ms(1600)), Some(ms(2400)))
    );
}

#[test]
fn header_bytes_zero_turns_headers_off() {
    let (t, e) = build_raw(&wire("header_bytes: 0\n"));
    assert_eq!(t.header_bytes, (0, 0));
    assert_eq!(
        wire_ms(&t, &hops(&t, &e, 0, 0)),
        (None, None),
        "no bytes, no transfer"
    );
    assert_eq!(
        wire_ms(&t, &hops(&t, &e, 100, 200)),
        (Some(ms(100)), Some(ms(200)))
    );
}

#[test]
fn one_size_sets_both_sides_and_a_pair_sets_each_with_the_default_for_a_missing_one() {
    let (t, _) = build_raw(&wire("header_bytes: 1KB\n"));
    assert_eq!(t.header_bytes, (1024, 1024));
    let (t, _) = build_raw(&wire("header_bytes: { request: 100B }\n"));
    assert_eq!(t.header_bytes, (100, 400), "the response keeps its default");
    let (t, _) = build_raw(&wire("header_bytes: { response: 2KB }\n"));
    assert_eq!(t.header_bytes, (600, 2048));
}

#[test]
fn a_topology_with_no_size_cost_does_not_move() {
    // no bandwidth, no per_kb, no throughput: headers have nothing to cost
    let src = "version: 1\nnodes:\n  nginx: { type: gateway }\n  app: { type: service }\nedges:\n  - { from: nginx, to: app, net: 2ms }\n";
    let (t, e) = build_raw(src);
    let with = hops(&t, &e, 5000, 5000);
    let (t0, e0) = build_raw(&src.replacen("version: 1\n", "version: 1\nheader_bytes: 0\n", 1));
    let without = hops(&t0, &e0, 5000, 5000);
    assert_eq!(with, without);
    assert!(
        with.iter()
            .all(|h| !matches!(h.kind, HopKind::Link { .. } | HopKind::Nic { .. }))
    );
}

#[test]
fn headers_count_on_an_http_nodes_per_kb_and_not_on_a_database_op() {
    let src = |top: &str| {
        format!(
            "version: 1\n{top}nodes:\n  nginx: {{ type: gateway }}\n  app: {{ type: service, per_kb: 1ms }}\n  db: {{ type: database, per_kb: 1ms, ops: {{ read: 4ms, write: 8ms, scan: 30ms }} }}\nedges:\n  - {{ from: nginx, to: app }}\n  - {{ from: app, to: db }}\n"
        )
    };
    // 1 KB of headers each way, no body
    let (t, e) = build_raw(&src("header_bytes: 1KB\n"));
    let h = hops(&t, &e, 0, 0);
    assert_eq!(
        hop_of(&t, &h, "app", HopKind::Proc),
        Duration::from_millis(2),
        "the service pays for 1 KB in and 1 KB out"
    );
    // the database speaks its own protocol: a read pays nothing for the HTTP headers
    assert_eq!(
        hop_of(&t, &h, "db", HopKind::Op(kaanal_core::Op::Read)),
        ms(4)
    );
}

#[test]
fn a_bad_header_size_names_the_field() {
    for bad in [
        "header_bytes: 5XB\n",
        "header_bytes: -1\n",
        "header_bytes: { colour: 1 }\n",
    ] {
        let err = kaanal_core::Config::from_yaml(&wire(bad))
            .unwrap_err()
            .to_string();
        assert!(
            err.contains("header_bytes") || err.contains("colour"),
            "{bad}: {err}"
        );
    }
}
