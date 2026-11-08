//! Step 8 (RULES 101): what the summary says about a node with `cpu: { cores }`: the CPU load, the
//! lower of the slot and CPU capacities as the saturation point, and which one binds.

use crate::common::*;
use kaanal_core::ChaosState;
use kaanal_sim::{SimConfig, run, summarize};
use std::time::Duration;

/// One service, 10 ms of own work a call, with the given slots and cores.
fn system(extra: &str) -> String {
    format!("version: 1\nseed: 7\nnodes:\n  app: {{ type: service, proc: 10ms{extra} }}\n")
}

fn cfg(rate: f64) -> SimConfig {
    let mut c = one_stream(rate, 120);
    c.warmup = Duration::from_secs(10);
    c
}

fn summary(yaml: &str, rate: f64) -> (kaanal_core::Topology, kaanal_sim::Summary) {
    let (t, e) = topo(yaml);
    let c = cfg(rate);
    let report = run(&t, &e, &ChaosState::default(), &c).unwrap();
    let s = summarize(&t, &e, &c, &report);
    (t, s)
}

#[test]
fn eight_slots_on_one_core_saturate_by_cpu_at_one_over_the_mean() {
    let (t, s) = summary(&system(", concurrency: 8, cpu: { cores: 1 }"), 50.0);
    let n = &s.nodes[t.id("app").unwrap()];
    // 1 core / 10 ms = 100 visits a second; the slots alone would allow 800
    assert_eq!(n.saturates_by, Some("cpu"));
    let at = n.saturates_at_req_s.unwrap();
    assert!((at - 100.0).abs() < 1.0, "{at}");
    let cap = n.cpu_capacity_per_s.unwrap();
    assert!((cap - 100.0).abs() < 1.0, "{cap}");
    // 50 a second x 10 ms on one core is half loaded
    let load = n.cpu_load.unwrap();
    assert!((load - 0.5).abs() < 0.05, "{load}");
    assert!(!n.saturated);
}

#[test]
fn one_slot_on_four_cores_saturates_by_slots() {
    let (t, s) = summary(&system(", concurrency: 1, cpu: { cores: 4 }"), 50.0);
    let n = &s.nodes[t.id("app").unwrap()];
    // one slot / 10 ms = 100 a second; four cores would allow 400
    assert_eq!(n.saturates_by, Some("slots"));
    let at = n.saturates_at_req_s.unwrap();
    assert!((at - 100.0).abs() < 1.0, "{at}");
}

#[test]
fn without_cpu_nothing_changes() {
    let (t, s) = summary(&system(", concurrency: 2"), 50.0);
    let n = &s.nodes[t.id("app").unwrap()];
    assert_eq!((n.cpu_load, n.cpu_capacity_per_s), (None, None));
    assert_eq!(n.saturates_by, Some("slots"));
}

#[test]
fn cpus_that_cannot_keep_up_flag_the_node_without_any_slot_limit() {
    // 150 a second x 10 ms on one core is 1.5: no slots to fill, but the node is saturated
    let (t, s) = summary(&system(", cpu: { cores: 1 }"), 150.0);
    let n = &s.nodes[t.id("app").unwrap()];
    assert!(n.cpu_load.unwrap() > 1.0, "{:?}", n.cpu_load);
    assert!(n.saturated);
    assert_eq!(n.saturates_by, Some("cpu"));
    assert!(
        s.warnings.iter().any(|w| w.contains("CPU load")),
        "{:?}",
        s.warnings
    );
}

#[test]
fn a_held_slot_does_not_count_the_wait_on_a_callee_as_cpu() {
    let yaml = "version: 1\nseed: 7\nnodes:\n  app: { type: service, proc: 10ms, concurrency: 50, hold: call, cpu: { cores: 1 } }\n  db: { type: database, ops: { read: 200ms, write: 200ms, scan: 200ms } }\nedges:\n  - { from: app, to: db }\n";
    let (t, s) = summary(yaml, 20.0);
    let n = &s.nodes[t.id("app").unwrap()];
    // 20 a second x 10 ms of its own work on one core: 0.2, not the 4 x that the held slots show
    let load = n.cpu_load.unwrap();
    assert!((load - 0.2).abs() < 0.03, "{load}");
}
