//! `summarize`: the report Studio reads. Percentiles, per node, replica and link load,
//! saturation, and the sample-count warnings, checked against hand-computed queueing values.

use crate::common::*;
use kaanal_core::{ChaosState, NodeChaos};
use kaanal_sim::{SimConfig, run, summarize};
use std::time::Duration;

fn summary_of(yaml: &str, cfg: &SimConfig) -> (kaanal_core::Topology, kaanal_sim::Summary) {
    let (t, e) = topo(yaml);
    let rep = run(&t, &e, &ChaosState::default(), cfg).unwrap();
    let s = summarize(&t, &e, cfg, &rep);
    (t, s)
}

fn windowed(rate: f64, secs: u64, warm: u64) -> SimConfig {
    let mut c = one_stream(rate, secs);
    c.warmup = Duration::from_secs(warm);
    c
}

#[test]
fn percentiles_of_a_fixed_latency_system_are_exact() {
    let y = "version: 1\nnodes:\n  gw: { type: gateway }\n  app: { type: service, proc: 10ms }\nedges:\n  - { from: gw, to: app }\n";
    let (_, s) = summary_of(y, &windowed(50.0, 60, 5));
    assert!(s.requests > 1000);
    for v in [
        s.latency.p50_ms,
        s.latency.p95_ms,
        s.latency.p99_ms,
        s.latency.max_ms,
    ] {
        assert!((v - 10.0).abs() < 1e-6, "{v}");
    }
    assert!(s.errors.is_empty() && s.failed == 0);
    assert!(s.label.contains("not a prediction"));
    let in_buckets: u64 = s.latency.buckets.iter().map(|b| b.count).sum();
    assert_eq!(in_buckets, s.requests);
}

#[test]
fn offered_load_matches_lambda_times_service_over_slots() {
    // M/M/1: lambda 50/s, mean service 10 ms, one slot: rho = 0.5
    let (t, s) = summary_of(QUEUED, &windowed(50.0, 600, 20));
    let app = t.id("app").unwrap();
    let load = s.nodes[app].offered_load.expect("app has a slot");
    assert!((load - 0.5).abs() < 0.05, "{load}");
    assert!(!s.nodes[app].saturated);
    assert_eq!(s.nodes[t.id("gw").unwrap()].offered_load, None);
    assert_eq!(s.bottlenecks[0], app);
}

#[test]
fn replicas_split_visits_and_load() {
    let y = "version: 1\nnodes:\n  gw: { type: gateway }\n  app: { type: service, concurrency: 1, proc: { mean: 10ms }, replicas: 2 }\nedges:\n  - { from: gw, to: app }\n";
    let (t, s) = summary_of(y, &windowed(50.0, 600, 20));
    let n = &s.nodes[t.id("app").unwrap()];
    assert_eq!(n.replicas.len(), 2);
    let total: u64 = n.replicas.iter().map(|r| r.visits).sum();
    for r in &n.replicas {
        let share =
            f64::from(u32::try_from(r.visits).unwrap()) / f64::from(u32::try_from(total).unwrap());
        assert!((share - 0.5).abs() < 0.03, "{share}");
    }
    // 50/s x 10 ms over 2 replicas x 1 slot
    let load = n.offered_load.unwrap();
    assert!((load - 0.25).abs() < 0.04, "{load}");
}

#[test]
fn a_bandwidth_link_reports_its_transfers() {
    let y = "version: 1\nnodes:\n  gw: { type: gateway }\n  app: { type: service, proc: 1ms }\nedges:\n  - { from: gw, to: app, bandwidth: 1000000 }\n";
    let mut cfg = windowed(20.0, 60, 5);
    cfg.streams[0].req_bytes = 1000;
    cfg.streams[0].resp_bytes = 1000;
    let (_, s) = summary_of(y, &cfg);
    assert_eq!(s.links.len(), 1);
    assert_eq!(
        (s.links[0].from.as_str(), s.links[0].to.as_str()),
        ("gw", "app")
    );
    assert_eq!(s.links[0].transfers, 2 * s.requests);
    let load = s.links[0].offered_load;
    // 20/s x 2 KB over 1 MB/s = 0.04
    assert!((load - 0.04).abs() < 0.01, "{load}");
    // the link fills at 20 / 0.04 = 500 req/s
    let at = s.links[0].saturates_at_req_s.expect("it carried transfers");
    assert!(
        (at - 20.0 / load).abs() < 1e-6 && (at - 500.0).abs() < 125.0,
        "{at}"
    );
}

#[test]
fn a_wire_reports_its_time_on_the_wire_per_window() {
    // 20/s x 2 KB over a 100 KB/s wire
    let y = "version: 1\nnodes:\n  gw: { type: gateway }\n  app: { type: service, proc: 1ms }\nedges:\n  - { from: gw, to: app, bandwidth: 100000 }\n";
    let mut cfg = windowed(20.0, 60, 5);
    cfg.streams[0].req_bytes = 1000;
    cfg.streams[0].resp_bytes = 1000;
    let (_, s) = summary_of(y, &cfg);
    let ser = &s.links[0].series;
    assert_eq!(ser.latency.len(), 60);
    // every body that crossed is in exactly one cell
    let cells: u64 = ser.latency.iter().flatten().map(|&c| u64::from(c)).sum();
    assert_eq!(cells, s.links[0].transfers);
}

#[test]
fn a_link_nothing_crossed_has_no_saturation_point() {
    let y = "version: 1\nnodes:\n  gw: { type: gateway }\n  app: { type: service, proc: 1ms }\nedges:\n  - { from: gw, to: app, bandwidth: 1000000 }\n";
    // no payload, so the link never carries a byte
    let (_, s) = summary_of(y, &windowed(20.0, 60, 5));
    assert_eq!(s.links[0].saturates_at_req_s, None);
}

#[test]
fn errors_are_counted_by_kind() {
    let (t, e) = topo(QUEUED);
    let app = t.id("app").unwrap();
    let chaos = ChaosState::from([(
        app,
        NodeChaos {
            error_rate: 1.0,
            error_status: 503,
            ..NodeChaos::default()
        },
    )]);
    let cfg = windowed(20.0, 30, 2);
    let rep = run(&t, &e, &chaos, &cfg).unwrap();
    let s = summarize(&t, &e, &cfg, &rep);
    assert_eq!(s.failed, s.requests);
    assert_eq!(s.errors["status"], s.requests);
}

#[test]
fn expected_visits_match_measured() {
    let (t, s) = summary_of(PROB, &windowed(100.0, 300, 10));
    let db = &s.nodes[t.id("db").unwrap()];
    assert!(db.expected_visits_per_s > 10.0);
    let off = (db.visits_per_s / db.expected_visits_per_s - 1.0).abs();
    assert!(
        off < 0.1,
        "measured {} expected {}",
        db.visits_per_s,
        db.expected_visits_per_s
    );
}

#[test]
fn an_empty_window_summarizes_to_zeros() {
    let mut cfg = windowed(0.001, 3, 2);
    cfg.seed = 5;
    let (_, s) = summary_of(QUEUED, &cfg);
    assert_eq!(s.requests, 0);
    for v in [
        s.latency.p50_ms,
        s.latency.p99_ms,
        s.latency.max_ms,
        s.window_s,
    ] {
        assert!(v.is_finite());
    }
    assert!(s.nodes.iter().all(|n| n.mean_wait_ms.is_finite()));
}

#[test]
fn few_samples_warn_about_the_tail() {
    let (_, few) = summary_of(QUEUED, &windowed(20.0, 11, 1));
    assert!(
        few.warnings.iter().any(|w| w.contains("p99")),
        "{:?}",
        few.warnings
    );
    let (_, many) = summary_of(QUEUED, &windowed(20.0, 400, 10));
    assert!(
        !many.warnings.iter().any(|w| w.contains("p99")),
        "{:?}",
        many.warnings
    );
}

#[test]
fn an_overloaded_node_is_flagged_saturated() {
    // lambda 200/s x 10 ms on one slot: rho = 2
    let (t, s) = summary_of(QUEUED, &windowed(200.0, 30, 2));
    let app = t.id("app").unwrap();
    assert!(s.nodes[app].saturated);
    assert!(
        s.warnings
            .iter()
            .any(|w| w.contains("app") && w.contains("saturated")),
        "{:?}",
        s.warnings
    );
}

#[test]
fn summary_visits_equal_the_engine_counters() {
    let (t, e) = topo(PROB);
    let cfg = windowed(100.0, 120, 5);
    let rep = run(&t, &e, &ChaosState::default(), &cfg).unwrap();
    let s = summarize(&t, &e, &cfg, &rep);
    for (i, stats) in rep.nodes.iter().enumerate() {
        let sum: u64 = s.nodes[i].replicas.iter().map(|r| r.visits).sum();
        assert_eq!(sum, stats.completed, "{}", t.nodes[i].name);
    }
}

#[test]
fn a_monolith_visit_is_one_visit_not_two_hops() {
    let y = "version: 1\nnodes:\n  gw: { type: gateway }\n  shop: { type: monolith, proc: 5ms, flavor: mysql }\nedges:\n  - { from: gw, to: shop }\n";
    let (t, e) = crate::shared::compiled(y, &[("GET /users/{id}", "shop")]);
    let cfg = windowed(100.0, 120, 5);
    let rep = run(&t, &e, &ChaosState::default(), &cfg).unwrap();
    let s = summarize(&t, &e, &cfg, &rep);
    let shop = &s.nodes[t.id("shop").unwrap()];
    let ratio = shop.visits_per_s / shop.expected_visits_per_s;
    assert!(
        (ratio - 1.0).abs() < 0.1,
        "measured {} expected {}",
        shop.visits_per_s,
        shop.expected_visits_per_s
    );
}

#[test]
fn timeouts_do_not_hide_a_saturated_node() {
    // 100 ms of work per request on one slot at 50/s is 5x capacity; the 10 ms deadline cuts
    // every recorded hop, but the slot is held for the full 100 ms
    let y = "version: 1\nnodes:\n  gw: { type: gateway }\n  app: { type: service, concurrency: 1, proc: 100ms, timeout: 10ms }\nedges:\n  - { from: gw, to: app }\n";
    let (t, s) = summary_of(y, &windowed(50.0, 60, 5));
    let app = &s.nodes[t.id("app").unwrap()];
    assert!(app.utilization.unwrap() >= 0.8, "{:?}", app.utilization);
    assert!(app.timed_out > 0);
    assert!(app.saturated);
    assert_eq!(s.bottlenecks[0], t.id("app").unwrap());
}

#[test]
fn a_deadline_at_a_node_without_slots_is_a_warning_not_saturation() {
    // `app` has no slot limit but a 10 ms deadline, and its callee `db` takes 100 ms
    let y = "version: 1\nnodes:\n  gw: { type: gateway }\n  app: { type: service, proc: 1ms, timeout: 10ms }\n  db: { type: database, flavor: postgres, ops: { read: 100ms, write: 100ms, scan: 100ms } }\nedges:\n  - { from: gw, to: app }\n  - { from: app, to: db }\n";
    let (t, s) = summary_of(y, &windowed(20.0, 30, 2));
    let app = &s.nodes[t.id("app").unwrap()];
    assert!(app.timed_out > 0, "the deadline fires");
    assert!(!app.saturated, "no slots, no queue to saturate");
    assert!(
        s.warnings
            .iter()
            .any(|w| w.contains("deadlines fired at node app")),
        "{:?}",
        s.warnings
    );
}
