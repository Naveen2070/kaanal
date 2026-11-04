//! F3 (RULES 56): what the queue and timeout that `init` and Studio write change. A 16-slot app at
//! 1.3x its capacity either grows an endless queue (p99 of a minute, "0 failed") or turns the
//! excess away and stays under the client timeout.

use crate::common::*;
use kaanal_core::ChaosState;
use kaanal_sim::{run, summarize};
use std::time::Duration;

/// 16 slots x 100 ms of work is 160 req/s of capacity.
fn system(app: &str, gw: &str) -> String {
    format!(
        "version: 1\nnodes:\n  gw: {{ type: gateway{gw} }}\n  app: {{ type: service, {app}, proc: 100ms }}\nedges:\n  - {{ from: gw, to: app }}\n"
    )
}

/// (failed, p99 in ms) of 240 s at 208 req/s, 1.3x the capacity.
fn overloaded(yaml: &str) -> (u64, f64) {
    let (t, e) = topo(yaml);
    let cfg = one_stream(208.0, 240);
    let rep = run(&t, &e, &ChaosState::default(), &cfg).unwrap();
    let s = summarize(&t, &e, &cfg, &rep);
    (s.failed, s.latency.p99_ms)
}

#[test]
fn a_bounded_queue_and_a_gateway_timeout_keep_p99_under_30_s_where_the_default_does_not() {
    let limit = Duration::from_secs(30).as_secs_f64() * 1000.0;
    let (failed, p99) = overloaded(&system("concurrency: 16", ""));
    assert_eq!(failed, 0, "unbounded: nobody is ever turned away");
    assert!(
        p99 > 2.0 * limit,
        "unbounded p99 {p99} ms should be far past 30 s"
    );

    let (failed, p99) = overloaded(&system(
        "concurrency: { limit: 16, queue: 100 }",
        ", timeout: 30s",
    ));
    assert!(failed > 0, "bounded: the excess is turned away");
    assert!(p99 <= limit, "bounded p99 {p99} ms should be within 30 s");
}

/// The saturation warning of a 1.3x overload, as the summary words it.
fn saturated_warning(yaml: &str) -> String {
    let (t, e) = topo(yaml);
    let cfg = one_stream(208.0, 240);
    let rep = run(&t, &e, &ChaosState::default(), &cfg).unwrap();
    let s = summarize(&t, &e, &cfg, &rep);
    s.warnings
        .iter()
        .find(|w| w.contains("is saturated"))
        .cloned()
        .unwrap_or_default()
}

#[test]
fn the_saturation_warning_says_what_a_full_queue_does_instead_of_growing_for_ever() {
    let unbounded = saturated_warning(&system("concurrency: 16", ""));
    assert!(unbounded.contains("its queue grows for as long as the run lasts"));

    let bounded = saturated_warning(&system("concurrency: { limit: 16, queue: 100 }", ""));
    assert!(
        bounded.contains("its queue is full and it turns requests away"),
        "{bounded}"
    );
    assert!(!bounded.contains("grows for as long"), "{bounded}");
}
