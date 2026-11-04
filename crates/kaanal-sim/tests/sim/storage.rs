//! The storage node end to end (RULES 61): a service reading objects, what their size costs, a
//! rate limit that answers 503, and chaos on the node.

use crate::common::*;
use kaanal_core::{ChainResult, ChaosState, NodeChaos};
use kaanal_sim::{run, summarize};

/// A service `app` calling the object store `files`; its costs are fixed, so the figures are exact.
const STORE: &str = "version: 1\nseed: 11\nnodes:\n  app: { type: service, proc: 2ms }\n  files: { type: storage, flavor: s3, first_byte: 20ms, throughput: 100MB/s }\nedges:\n  - { from: app, to: files }\nendpoints:\n  \"GET /users/{id}\": { chain: [ { call: files, op: read } ] }\n";

fn p50_ms(resp_bytes: u64) -> f64 {
    let (t, e) = topo(STORE);
    let mut cfg = one_stream(5.0, 120);
    cfg.streams[0].resp_bytes = resp_bytes;
    let rep = run(&t, &e, &ChaosState::default(), &cfg).unwrap();
    assert!(rep.requests.iter().all(|r| r.result == ChainResult::Ok));
    summarize(&t, &e, &cfg, &rep).latency.p50_ms
}

#[test]
fn a_service_calling_a_store_pays_proc_plus_first_byte() {
    let p50 = p50_ms(0);
    assert!((p50 - 22.0).abs() < 0.01, "2 ms + 20 ms: {p50}");
}

#[test]
fn a_bigger_response_raises_the_p50_by_bytes_over_throughput() {
    let base = p50_ms(0);
    // 5 MB over 100 MB/s is 50 ms
    let big = p50_ms(5 * 1024 * 1024);
    assert!((big - base - 50.0).abs() < 0.05, "{big} vs {base}");
    // 5 KB adds under a millisecond
    let small = p50_ms(5 * 1024);
    assert!(small - base < 1.0 && small > base, "{small} vs {base}");
}

#[test]
fn a_rate_limit_turns_requests_away_with_503_and_the_summary_counts_them() {
    let yaml = STORE.replace(
        "throughput: 100MB/s",
        "throughput: 100MB/s, rate_limit: { rps: 100, burst: 10 }",
    );
    let (t, e) = topo(&yaml);
    let files = t.id("files").unwrap();
    // 200 req/s offered into 100 rps: about half are turned away
    let cfg = one_stream(200.0, 30);
    let rep = run(&t, &e, &ChaosState::default(), &cfg).unwrap();
    let refused = rep
        .requests
        .iter()
        .filter(|r| matches!(r.result, ChainResult::Err { status: 503, .. }))
        .count() as u64;
    assert!(refused > 1000, "{refused}");
    assert!(
        rep.requests.iter().all(|r| match r.result {
            ChainResult::Err { status, .. } => status == 503,
            ChainResult::Ok => true,
        }),
        "a storage rate limit never answers 429"
    );
    assert_eq!(rep.nodes[files].limited, refused);
    let s = summarize(&t, &e, &cfg, &rep);
    assert_eq!(
        s.nodes[files].limited, refused,
        "the summary counts them for that node"
    );
    // the passed rate is the limit: 100 a second (plus the burst)
    let passed = rep.requests.len() as u64 - refused;
    assert!((2900..=3100).contains(&passed), "{passed}");
}

#[test]
fn other_kinds_keep_429() {
    let yaml = "version: 1\nnodes:\n  app: { type: service, rate_limit: { rps: 50, burst: 5 } }\n  db: { type: database, flavor: postgres }\nedges:\n  - { from: app, to: db }\n";
    let (t, e) = topo(yaml);
    let rep = run(&t, &e, &ChaosState::default(), &one_stream(200.0, 10)).unwrap();
    let statuses: Vec<u16> = rep
        .requests
        .iter()
        .filter_map(|r| match r.result {
            ChainResult::Err { status, .. } => Some(status),
            ChainResult::Ok => None,
        })
        .collect();
    assert!(
        !statuses.is_empty() && statuses.iter().all(|s| *s == 429),
        "{statuses:?}"
    );
}

#[test]
fn a_store_that_is_down_fails_the_requests_that_need_it() {
    let (t, e) = topo(STORE);
    let mut chaos = ChaosState::default();
    chaos.insert(
        t.id("files").unwrap(),
        NodeChaos {
            down: true,
            ..NodeChaos::default()
        },
    );
    let rep = run(&t, &e, &chaos, &one_stream(10.0, 10)).unwrap();
    assert_ne!(rep.requests.len(), 0);
    assert!(
        rep.requests
            .iter()
            .all(|r| matches!(r.result, ChainResult::Err { status: 503, .. })),
        "{:?}",
        rep.requests.first()
    );
}

#[test]
fn a_slowed_store_adds_its_latency() {
    let (t, e) = topo(STORE);
    let mut chaos = ChaosState::default();
    chaos.insert(
        t.id("files").unwrap(),
        NodeChaos {
            latency_add: std::time::Duration::from_millis(30),
            ..NodeChaos::default()
        },
    );
    let cfg = one_stream(5.0, 60);
    let rep = run(&t, &e, &chaos, &cfg).unwrap();
    let p50 = summarize(&t, &e, &cfg, &rep).latency.p50_ms;
    assert!((p50 - 52.0).abs() < 0.01, "{p50}");
}
