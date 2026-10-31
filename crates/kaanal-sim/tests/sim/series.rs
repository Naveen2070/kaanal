//! The per-node time series in the summary: 60 windows of busy fraction, longest queue and a
//! latency histogram, plus capacity and headroom. Warmup is 0 where a figure is compared with the
//! engine's, because the records leave out requests that arrived before warmup.

use crate::common::*;
use kaanal_core::ChaosState;
use kaanal_sim::{SimConfig, SimReport, Summary, run, summarize};

fn go(yaml: &str, cfg: &SimConfig) -> (kaanal_core::Topology, SimReport, Summary) {
    let (t, e) = topo(yaml);
    let rep = run(&t, &e, &ChaosState::default(), cfg).unwrap();
    let s = summarize(&t, &e, cfg, &rep);
    (t, rep, s)
}

#[test]
fn series_has_sixty_windows_and_sums_to_the_whole_run() {
    let (t, rep, s) = go(QUEUED, &one_stream(50.0, 60));
    let app = t.id("app").unwrap();
    let n = &s.nodes[app];
    assert_eq!(n.series.busy.len(), 60);
    assert_eq!(n.series.queue.len(), 60);
    assert_eq!(n.series.latency.len(), 60);
    assert!((n.series.window_s - 1.0).abs() < 1e-12);
    assert!(n.series.busy.iter().all(|b| (0.0..=1.0).contains(b)));
    // one slot: the sum of busy x window slot-time is the busy slot-time of the run
    let slot_time: f64 = n.series.busy.iter().map(|b| b * n.series.window_s).sum();
    let measured = rep.nodes[app].utilization * s.window_s;
    assert!(
        (slot_time - measured).abs() < 1e-9,
        "{slot_time} vs {measured}"
    );
}

#[test]
fn queue_series_peaks_where_the_run_saturates() {
    // M/M/1 at 1.2 x capacity (100/s): the queue grows for the whole run
    let (t, rep, s) = go(QUEUED, &one_stream(120.0, 60));
    let app = t.id("app").unwrap();
    let q = &s.nodes[app].series.queue;
    // warmup 0, so max_queue (a join at or after warmup) and the records see the same joins
    assert_eq!(
        q.iter().copied().max().unwrap() as usize,
        rep.nodes[app].max_queue
    );
    let (first, last) = (q[..5].iter().sum::<u32>(), q[55..].iter().sum::<u32>());
    assert!(last > first, "{first} vs {last}: {q:?}");
}

#[test]
fn an_unlimited_node_has_no_busy_series_and_no_capacity() {
    let (t, _, s) = go(QUEUED, &one_stream(50.0, 10));
    let gw = &s.nodes[t.id("gw").unwrap()];
    assert_eq!(gw.series.busy.len(), 0);
    assert_eq!(gw.series.queue, vec![0; 60]);
    assert_eq!((gw.capacity_per_s, gw.headroom), (None, None));
    assert_eq!(gw.saturates_at_req_s, None);
}

#[test]
fn capacity_matches_slots_over_service_time() {
    let yaml = "version: 1\nnodes:\n  gw: { type: gateway }\n  app: { type: service, concurrency: 12, proc: 98ms }\nedges:\n  - { from: gw, to: app }\n";
    let (t, _, s) = go(yaml, &one_stream(20.0, 30));
    let n = &s.nodes[t.id("app").unwrap()];
    let cap = n.capacity_per_s.expect("app has slots and visits");
    assert!((cap - 122.4).abs() < 0.1, "{cap}");
    let load = n.offered_load.unwrap();
    assert!((n.headroom.unwrap() - (1.0 - load)).abs() < 1e-12);
}

#[test]
fn headroom_stops_at_zero_when_overloaded() {
    let (t, _, s) = go(QUEUED, &one_stream(150.0, 30));
    let n = &s.nodes[t.id("app").unwrap()];
    assert!(n.offered_load.unwrap() > 1.0);
    assert_eq!(n.headroom, Some(0.0));
}

#[test]
fn latency_columns_sum_to_the_node_hop_count() {
    let (t, rep, s) = go(QUEUED, &one_stream(120.0, 20));
    let app = t.id("app").unwrap();
    let cols = &s.nodes[app].series.latency;
    assert!(
        cols.iter().all(|c| c.len() == 20),
        "19 buckets and overflow"
    );
    let total: u64 = cols.iter().flatten().map(|&c| u64::from(c)).sum();
    assert_eq!(total, rep.nodes[app].completed);
}

#[test]
fn a_run_shorter_than_a_window_still_has_sixty_windows() {
    let (t, _, s) = go(QUEUED, &one_stream(50.0, 1));
    let n = &s.nodes[t.id("app").unwrap()];
    assert_eq!(
        (
            n.series.busy.len(),
            n.series.queue.len(),
            n.series.latency.len()
        ),
        (60, 60, 60)
    );
    assert!((n.series.window_s - 1.0 / 60.0).abs() < 1e-12);
}

#[test]
fn saturation_is_in_requests_per_second_not_visits() {
    let base = "version: 1
nodes:
  gw: { type: gateway }
  app: { type: service }
  db: { type: service, concurrency: 12, proc: 98ms }
edges:
  - { from: gw, to: app }
  - { from: app, to: db }
";
    let sat = |chain: &str| {
        let yaml = format!(
            "{base}endpoints:
  \"GET /users/{{id}}\": {{ chain: [ {chain} ] }}
"
        );
        let (t, _, s) = go(&yaml, &one_stream(20.0, 30));
        (
            s.nodes[t.id("db").unwrap()].clone(),
            s.nodes[t.id("gw").unwrap()].saturates_at_req_s,
        )
    };
    // one visit per request: requests and visits agree
    let (n, _) = sat("{ call: db }");
    let (cap, req) = (n.capacity_per_s.unwrap(), n.saturates_at_req_s.unwrap());
    assert!((cap - req).abs() < 1e-6, "{cap} vs {req}");
    // two visits per request: half as many requests fit
    let (n, gw) = sat("{ call: db, repeat: 2 }");
    let (cap, req) = (n.capacity_per_s.unwrap(), n.saturates_at_req_s.unwrap());
    assert!((req - cap / 2.0).abs() < 1e-6, "{cap} vs {req}");
    // a node without slots has no figure
    assert_eq!(gw, None);
}

#[test]
fn system_series_sums_match_the_summary() {
    // half the requests fail (error rate 0.5 at app), warmup 0 so every request is in a window
    let (t, e) = topo(QUEUED);
    let chaos = kaanal_core::ChaosState::from([(
        t.id("app").unwrap(),
        kaanal_core::NodeChaos {
            error_rate: 0.5,
            error_status: 503,
            ..kaanal_core::NodeChaos::default()
        },
    )]);
    let cfg = one_stream(50.0, 30);
    let rep = run(&t, &e, &chaos, &cfg).unwrap();
    let s = summarize(&t, &e, &cfg, &rep);
    assert!(s.failed > 0 && s.failed < s.requests);
    let done: u64 = s.series.done.iter().map(|&n| u64::from(n)).sum();
    let failed: u64 = s.series.failed.iter().map(|&n| u64::from(n)).sum();
    assert_eq!(done + failed, s.requests);
    assert_eq!(failed, s.failed);
}

#[test]
fn system_series_has_sixty_windows() {
    let (_, _, s) = go(QUEUED, &one_stream(50.0, 60));
    assert_eq!(s.series.done.len(), 60);
    assert_eq!(s.series.failed.len(), 60);
    assert_eq!(s.series.p99_ms.len(), 60);
    assert!((s.series.window_s - 1.0).abs() < 1e-12);
    // 10 ms service at 50/s: every window with traffic has a p99 that is a bucket bound
    assert!(s.series.p99_ms.iter().all(|&p| p > 0.0));
}

#[test]
fn node_percentiles_are_bucket_bounds_and_none_without_visits() {
    let (t, _, s) = go(QUEUED, &one_stream(50.0, 30));
    let app = &s.nodes[t.id("app").unwrap()];
    let (p50, p99) = (app.p50_ms.unwrap(), app.p99_ms.unwrap());
    assert!(p50 <= p99);
    let bounds: Vec<f64> = (0..19).map(|i| 0.25 * f64::from(1u32 << i)).collect();
    assert!(
        bounds.contains(&p50) && bounds.contains(&p99),
        "{p50} {p99}"
    );
    // a service that no endpoint reaches is never visited
    let y = "version: 1\nnodes:\n  gw: { type: gateway }\n  app: { type: service, proc: 10ms }\n  idle: { type: service, proc: 10ms }\nedges:\n  - { from: gw, to: app }\n";
    let (t, _, s) = go(y, &one_stream(50.0, 10));
    let idle = &s.nodes[t.id("idle").unwrap()];
    assert_eq!((idle.p50_ms, idle.p99_ms), (None, None));
}

#[test]
fn zero_request_run_has_zero_series_and_no_nan() {
    let (_, _, s) = go(QUEUED, &one_stream(1e-6, 10));
    assert_eq!(s.requests, 0);
    assert_eq!(s.series.done, vec![0; 60]);
    assert_eq!(s.series.failed, vec![0; 60]);
    assert_eq!(s.series.p99_ms, vec![0.0; 60]);
    assert!(s.series.window_s.is_finite());
    assert!(
        s.nodes
            .iter()
            .all(|n| n.p50_ms.is_none() && n.p99_ms.is_none())
    );
}
