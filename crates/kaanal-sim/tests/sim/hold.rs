//! `hold: call` end to end: a slow database fills the app above it only when the app holds its slot
//! across the call, and the report's figures for that node agree with what the engine measured.

use crate::common::*;
use kaanal_core::ChaosState;
use kaanal_sim::{SimConfig, run, summarize};
use std::time::Duration;

/// An app with 4 slots calls a database that serves 2 at a time, 40 ms each (50 a second at most).
fn system(hold: &str) -> String {
    format!(
        "version: 1\nseed: 7\nnodes:\n  app: {{ type: service, concurrency: 4, proc: 5ms{hold} }}\n  db: {{ type: database, ops: {{ read: 40ms, write: 40ms, scan: 40ms }}, pool: {{ size: 2, queue: 1000 }} }}\nedges:\n  - {{ from: app, to: db }}\n"
    )
}

fn cfg() -> SimConfig {
    let mut c = one_stream(45.0, 300);
    c.warmup = Duration::from_secs(20);
    c
}

fn run_summary(yaml: &str) -> (kaanal_core::Topology, kaanal_sim::Summary) {
    let (t, e) = topo(yaml);
    let c = cfg();
    let report = run(&t, &e, &ChaosState::default(), &c).unwrap();
    let s = summarize(&t, &e, &c, &report);
    (t, s)
}

#[test]
fn a_held_slot_shows_the_app_busy_while_the_database_is_the_slow_part() {
    let (t, own) = run_summary(&system(""));
    let (_, held) = run_summary(&system(", hold: call"));
    let app = t.id("app").unwrap();
    let (a_own, a_held) = (&own.nodes[app], &held.nodes[app]);

    // without the setting the app is only ever busy for its own 5 ms: 45/s x 5 ms / 4 slots
    let u_own = a_own.utilization.unwrap();
    assert!(u_own < 0.1, "{u_own}");
    // with it the app is busy for its 5 ms plus the whole wait on the database
    let u_held = a_held.utilization.unwrap();
    assert!(u_held > 0.5, "{u_held}");

    // the report's load for the held node is what the engine measured, so the two agree
    let load = a_held.offered_load.unwrap();
    assert!(
        (load - u_held).abs() < 0.05,
        "load {load} vs utilization {u_held}"
    );
    // and so does the busy share per window: its mean is the utilization
    let busy = &a_held.series.busy;
    let mean = busy.iter().sum::<f64>() / f64::from(u32::try_from(busy.len()).unwrap());
    assert!(
        (mean - u_held).abs() < 0.1,
        "series {mean} vs utilization {u_held}"
    );

    // the app's saturation point drops from hundreds a second to about the database's limit
    let at_own = a_own.saturates_at_req_s.unwrap();
    let at_held = a_held.saturates_at_req_s.unwrap();
    assert!(at_own > 400.0, "{at_own}");
    assert!(at_held < 120.0, "{at_held}");

    // the database's own figures do not change: it was the slow part either way
    let db = t.id("db").unwrap();
    let du = (own.nodes[db].utilization.unwrap() - held.nodes[db].utilization.unwrap()).abs();
    assert!(du < 0.05, "{du}");
}

#[test]
fn the_setting_does_nothing_for_a_node_with_no_slots() {
    // no `concurrency`: nothing to hold, so the plans and the figures are those of `own`
    let y = |hold: &str| {
        format!(
            "version: 1\nseed: 7\nnodes:\n  app: {{ type: service, proc: 5ms{hold} }}\n  db: {{ type: database, flavor: postgres }}\nedges:\n  - {{ from: app, to: db }}\n"
        )
    };
    let (t, a) = run_summary(&y(""));
    let (_, b) = run_summary(&y(", hold: call"));
    let app = t.id("app").unwrap();
    assert_eq!(a.nodes[app].utilization, b.nodes[app].utilization);
    assert_eq!(a.latency.p99_ms, b.latency.p99_ms);
}
