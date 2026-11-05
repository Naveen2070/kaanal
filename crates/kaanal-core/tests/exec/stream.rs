//! Streamed responses (RULES 71, step 6c): the response transfer starts at the callee's first byte
//! and runs while it is still producing, so a call takes `first byte + size / slowest stage`, not
//! the sum of the stages. The oracle is the pipeline limit (see `exec/stream.rs`).

use crate::common::*;
use kaanal_core::{
    ChainRequest, ChaosState, Config, Hop, HopKind, NodeChaos, PlanItem, Topology, execute,
};
use std::time::Duration;

/// 100 MB/s = 104857600 B/s, so 1 KB (1024 B) is 9765.625 ns, which the engine holds as 9766 ns.
const PER_KB_NS: u64 = 9_766;
const MIB: u64 = 1 << 20;

/// An app that calls a store (`first_byte: 20ms`, `throughput: 100MB/s`) over an edge with `edge`'s
/// extra fields; the route reads from the store.
fn cfg(edge: &str, step: &str) -> String {
    format!(
        "version: 1\nnodes:\n  nginx: {{ type: gateway }}\n  app: {{ type: service }}\n  files: {{ type: storage, first_byte: 20ms, throughput: 100MB/s }}\nedges:\n  - {{ from: nginx, to: app }}\n  - {{ from: app, to: files{edge} }}\nendpoints:\n  \"GET /users/{{id}}\":\n    chain: [{{ call: files, op: read{step} }}]\n"
    )
}

fn plan(t: &Topology, e: &kaanal_core::Endpoints, chaos: &ChaosState, resp: u64) -> Vec<PlanItem> {
    execute(
        t,
        &chaos.compile(),
        &ChainRequest {
            at: std::time::Duration::ZERO,
            endpoint: e.get("GET /users/{id}").unwrap(),
            n: 0,
            seed: 42,
            pin_delay: Duration::ZERO,
            req_bytes: 0,
            resp_bytes: resp,
        },
    )
    .plan
    .items
}

fn hops_of(items: &[PlanItem]) -> Vec<Hop> {
    items
        .iter()
        .filter_map(|i| match i {
            PlanItem::Hop(h) => Some(h.clone()),
            _ => None,
        })
        .collect()
}

/// The store's own hop, the first-byte wait, and the response link hop.
fn parts(t: &Topology, h: &[Hop]) -> (Hop, Option<Hop>, Option<Hop>) {
    let files = t.id("files").unwrap();
    let find = |f: &dyn Fn(&Hop) -> bool| h.iter().find(|x| f(x)).cloned();
    (
        find(&|x| x.node == files && x.kind == HopKind::Proc).expect("the store's hop"),
        find(&|x| x.kind == HopKind::FirstByte),
        find(&|x| matches!(x.kind, HopKind::Link { resp: true, .. })),
    )
}

fn end(h: &Hop) -> Duration {
    h.start + h.dur
}

/// How long after `from` started `to` ended.
fn since(from: &Hop, to: &Hop) -> Duration {
    end(to).saturating_sub(from.start)
}

const STORE_SIZE_TERM: Duration = Duration::from_nanos(1024 * PER_KB_NS);

#[test]
fn a_streamed_read_over_a_slow_wire_takes_first_byte_plus_the_wire_time() {
    // 1 MiB over a 1 MiB/s wire: the wire (1 s) is the slowest stage; the store needs 10 ms
    let (t, e) = build(&cfg(", bandwidth: 1MB", ""));
    let items = plan(&t, &e, &ChaosState::default(), MIB);
    let (proc, first, link) = parts(&t, &hops_of(&items));
    let (first, link) = (first.expect("a first-byte wait"), link.expect("a link hop"));
    assert_eq!(proc.dur, ms(20) + STORE_SIZE_TERM);
    assert_eq!(first.dur, ms(20), "the first byte comes after first_byte");
    assert_eq!(
        first.start, proc.start,
        "the wait runs beside the store's hop"
    );
    assert_eq!(
        link.start,
        end(&first),
        "the transfer starts at the first byte"
    );
    assert_eq!(link.dur, Duration::from_secs(1));
    // the pipeline limit: first byte + size / slowest stage
    assert_eq!(since(&proc, &link), ms(20) + Duration::from_secs(1));
    assert!(
        items
            .iter()
            .any(|i| matches!(i, PlanItem::Fork { branches: 2, .. })),
        "the overlap is an ordinary fork"
    );
}

#[test]
fn a_streamed_read_over_a_fast_wire_is_limited_by_the_store() {
    // 1 MiB over 1 GiB/s: the wire needs under 1 ms, the store 10 ms after its first byte
    let (t, e) = build(&cfg(", bandwidth: 1GB", ""));
    let items = plan(&t, &e, &ChaosState::default(), MIB);
    let h = hops_of(&items);
    let (proc, _, link) = parts(&t, &h);
    let link = link.unwrap();
    assert_eq!(link.dur, Duration::from_nanos(976_562));
    // the store's hop ends last: first byte + size / throughput
    assert!(end(&proc) >= end(&link));
    assert_eq!(since(&proc, &proc), ms(20) + STORE_SIZE_TERM);
}

#[test]
fn stream_false_is_the_buffered_sum() {
    let (t, e) = build(&cfg(", bandwidth: 1MB, stream: false", ""));
    let items = plan(&t, &e, &ChaosState::default(), MIB);
    let (proc, first, link) = parts(&t, &hops_of(&items));
    assert!(first.is_none(), "no first-byte wait");
    assert!(!items.iter().any(|i| matches!(i, PlanItem::Fork { .. })));
    let link = link.unwrap();
    assert_eq!(
        link.start,
        end(&proc),
        "the transfer waits for the store to finish"
    );
    assert_eq!(
        since(&proc, &link),
        ms(20) + STORE_SIZE_TERM + Duration::from_secs(1)
    );
}

#[test]
fn a_step_beats_its_edge() {
    let (t, e) = build(&cfg(", bandwidth: 1MB, stream: true", ", stream: false"));
    let (_, first, _) = parts(&t, &hops_of(&plan(&t, &e, &ChaosState::default(), MIB)));
    assert!(first.is_none(), "the step says buffered");
    let (t, e) = build(&cfg(", bandwidth: 1MB, stream: false", ", stream: true"));
    let (_, first, _) = parts(&t, &hops_of(&plan(&t, &e, &ChaosState::default(), MIB)));
    assert!(first.is_some(), "the step says streamed");
}

#[test]
fn only_a_storage_read_streams_by_default() {
    // a write to the store, and a read from a service, stay buffered
    let (t, e) = build(&cfg(", bandwidth: 1MB", "").replace("op: read", "op: write"));
    let (_, first, _) = parts(&t, &hops_of(&plan(&t, &e, &ChaosState::default(), MIB)));
    assert!(first.is_none(), "a PUT is not a stream");
}

#[test]
fn nothing_streams_without_a_pipe_or_a_body() {
    // no bandwidth anywhere: nothing to overlap, so the plan has no fork
    let (t, e) = build(&cfg("", ""));
    let items = plan(&t, &e, &ChaosState::default(), MIB);
    assert!(!items.iter().any(|i| matches!(i, PlanItem::Fork { .. })));
    // a wire but no response bytes
    let (t, e) = build(&cfg(", bandwidth: 1MB", ""));
    let items = plan(&t, &e, &ChaosState::default(), 0);
    assert!(!items.iter().any(|i| matches!(i, PlanItem::Fork { .. })));
}

#[test]
fn a_callee_that_calls_on_cannot_send_early_so_it_stays_as_slow_as_buffered() {
    // `mid` runs its own hop and then calls the store; with `stream: true` on the edge into `mid`,
    // its sequence does not end in its own hop, so the first byte is the end of the sequence
    let yaml = "version: 1
header_bytes: 0
nodes:
  nginx: { type: gateway }
  app: { type: service }
  mid: { type: service, proc: 5ms }
  files: { type: storage, first_byte: 20ms }
edges:
  - { from: nginx, to: app }
  - { from: app, to: mid, bandwidth: 1MB, stream: true }
  - { from: mid, to: files }
endpoints:
  \"GET /users/{id}\":
    chain: [{ call: mid, chain: [{ call: files, op: read }] }]
";
    let (t, e) = build(yaml);
    let h = hops_of(&plan(&t, &e, &ChaosState::default(), MIB));
    let files = t.id("files").unwrap();
    let store = h
        .iter()
        .find(|x| x.node == files && x.kind == HopKind::Proc)
        .unwrap();
    let link = h
        .iter()
        .find(|x| matches!(x.kind, HopKind::Link { resp: true, .. }))
        .unwrap();
    assert_eq!(
        link.start,
        end(store),
        "the transfer starts when the callee is done"
    );
}

#[test]
fn a_failing_callee_sends_nothing() {
    let (t, e) = build(&cfg(", bandwidth: 1MB", ""));
    let files = t.id("files").unwrap();
    let down = ChaosState::from([(
        files,
        NodeChaos {
            down: true,
            ..NodeChaos::default()
        },
    )]);
    let items = plan(&t, &e, &down, MIB);
    assert!(items.iter().any(|i| matches!(i, PlanItem::Fail { .. })));
    let h = hops_of(&items);
    assert!(h.iter().all(
        |x| x.kind != HopKind::FirstByte && !matches!(x.kind, HopKind::Link { resp: true, .. })
    ));
}

#[test]
fn stream_is_checked_when_the_file_loads() {
    let load = |edge: &str| Config::from_yaml(&cfg(edge, "")).map_err(|e| e.to_string());
    assert!(load(", stream: true").is_ok());
    let err = load(", stream: 1").unwrap_err();
    assert!(err.contains("stream"), "{err}");
}

// ---- S4: streamed uploads (RULES 76) ----

/// The plan of a write of `req` bytes (the route's op is a write, the store counts the request).
fn put_plan(
    t: &Topology,
    e: &kaanal_core::Endpoints,
    chaos: &ChaosState,
    req: u64,
) -> Vec<PlanItem> {
    execute(
        t,
        &chaos.compile(),
        &ChainRequest {
            at: std::time::Duration::ZERO,
            endpoint: e.get("GET /users/{id}").unwrap(),
            n: 0,
            seed: 42,
            pin_delay: Duration::ZERO,
            req_bytes: req,
            resp_bytes: 0,
        },
    )
    .plan
    .items
}

fn put_cfg(edge: &str, step: &str) -> String {
    cfg(edge, step).replace("op: read", "op: write")
}

/// The store's hop and the request link hop.
fn put_parts(t: &Topology, items: &[PlanItem]) -> (Hop, Hop) {
    let h = hops_of(items);
    let files = t.id("files").unwrap();
    (
        h.iter()
            .find(|x| x.node == files && x.kind == HopKind::Proc)
            .cloned()
            .expect("the store's hop"),
        h.iter()
            .find(|x| matches!(x.kind, HopKind::Link { resp: false, .. }))
            .cloned()
            .expect("the request link"),
    )
}

#[test]
fn a_streamed_upload_over_a_slow_wire_takes_the_wire_time() {
    // a store write counts the request: 1 MiB at 100 MB/s is 10 ms after the 20 ms first byte, but
    // the wire needs 1 s, so the upload is the slowest stage. Streamed, the store works while the
    // bytes arrive: the call ends with the wire (max of the two), not 1 s + 30 ms.
    let (t, e) = build(&put_cfg(", bandwidth: 1MB", ""));
    let items = put_plan(&t, &e, &ChaosState::default(), MIB);
    let (proc, link) = put_parts(&t, &items);
    assert_eq!(proc.dur, ms(20) + STORE_SIZE_TERM);
    assert_eq!(link.dur, Duration::from_secs(1));
    assert_eq!(
        proc.start, link.start,
        "the store starts as the bytes start to arrive"
    );
    assert!(
        items
            .iter()
            .any(|i| matches!(i, PlanItem::Fork { branches: 2, .. })),
        "the overlap is an ordinary fork"
    );
}

#[test]
fn a_streamed_upload_over_a_fast_wire_is_limited_by_the_store() {
    let (t, e) = build(&put_cfg(", bandwidth: 1GB", ""));
    let (proc, link) = put_parts(&t, &put_plan(&t, &e, &ChaosState::default(), MIB));
    assert_eq!(link.dur, Duration::from_nanos(976_562));
    assert!(end(&proc) > end(&link), "the store ends last");
}

#[test]
fn stream_false_restores_the_buffered_upload() {
    let (t, e) = build(&put_cfg(", bandwidth: 1MB, stream: false", ""));
    let items = put_plan(&t, &e, &ChaosState::default(), MIB);
    let (proc, link) = put_parts(&t, &items);
    assert!(!items.iter().any(|i| matches!(i, PlanItem::Fork { .. })));
    assert_eq!(proc.start, end(&link), "the store works after the upload");
}

#[test]
fn stream_request_streams_a_read_s_upload_and_response_does_not() {
    // a read from the store has no request body to stream by default; `request` asks for it
    let (t, e) = build(&cfg(", bandwidth: 1MB, stream: request", ""));
    let items = put_plan(&t, &e, &ChaosState::default(), MIB);
    assert!(items.iter().any(|i| matches!(i, PlanItem::Fork { .. })));
    let (t, e) = build(&cfg(", bandwidth: 1MB, stream: response", ""));
    let h = hops_of(&put_plan(&t, &e, &ChaosState::default(), MIB));
    let link = h
        .iter()
        .find(|x| matches!(x.kind, HopKind::Link { resp: false, .. }))
        .unwrap();
    let files = t.id("files").unwrap();
    let proc = h
        .iter()
        .find(|x| x.node == files && x.kind == HopKind::Proc)
        .unwrap();
    assert_eq!(
        proc.start,
        end(link),
        "`response` leaves the upload buffered"
    );
}

#[test]
fn a_failing_store_takes_the_upload_down_with_it() {
    let (t, e) = build(&put_cfg(", bandwidth: 1MB", ""));
    let files = t.id("files").unwrap();
    let down = ChaosState::from([(
        files,
        NodeChaos {
            down: true,
            ..NodeChaos::default()
        },
    )]);
    let items = put_plan(&t, &e, &down, MIB);
    assert!(items.iter().any(|i| matches!(i, PlanItem::Fail { .. })));
}

#[test]
fn nothing_streams_up_without_request_bytes_or_a_pipe() {
    let (t, e) = build(&put_cfg(", bandwidth: 1MB", ""));
    let items = put_plan(&t, &e, &ChaosState::default(), 0);
    assert!(!items.iter().any(|i| matches!(i, PlanItem::Fork { .. })));
    let (t, e) = build(&put_cfg("", ""));
    let items = put_plan(&t, &e, &ChaosState::default(), MIB);
    assert!(!items.iter().any(|i| matches!(i, PlanItem::Fork { .. })));
}

#[test]
fn the_stream_modes_are_checked_when_the_file_loads() {
    let load = |edge: &str| Config::from_yaml(&cfg(edge, "")).map_err(|e| e.to_string());
    for ok in ["true", "false", "request", "response", "both", "none"] {
        assert!(load(&format!(", stream: {ok}")).is_ok(), "{ok}");
    }
    let err = load(", stream: up").unwrap_err();
    assert!(err.contains("stream"), "{err}");
}
