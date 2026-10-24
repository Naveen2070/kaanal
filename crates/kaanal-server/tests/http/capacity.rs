//! Pools, concurrency and rate limits, and what they do to timing and traces.

use crate::common::*;
use serde_json::json;
use std::time::Duration;

/// Gateway → fixture service → db whose pool is `pool`; every db op is 100 ms so waits are visible.
pub(crate) fn pooled(pool: &str) -> String {
    format!(
        "version: 1\nnodes:\n  nginx: {{ type: gateway }}\n  app: {{ type: service, spec: openapi.yaml }}\n  db: {{ type: database, ops: {{ read: 300ms, write: 300ms, scan: 300ms }}, {pool} }}\nedges:\n  - {{ from: nginx, to: app }}\n  - {{ from: app, to: db }}\nendpoints:\n"
    )
}

pub(crate) async fn burst(
    s: &Srv,
    path: &'static str,
    n: usize,
) -> Vec<(u16, Duration, String, Option<String>)> {
    let mut tasks = Vec::new();
    for _ in 0..n {
        let (client, url) = (s.client.clone(), format!("{}{path}", s.base));
        tasks.push(tokio::spawn(async move {
            let t = std::time::Instant::now();
            let r = client.get(url).send().await.unwrap();
            let retry = r
                .headers()
                .get("retry-after")
                .map(|v| v.to_str().unwrap().to_string());
            let trace = r.headers()["x-kaanal-trace"].to_str().unwrap().to_string();
            (r.status().as_u16(), t.elapsed(), trace, retry)
        }));
    }
    let mut out = Vec::new();
    for t in tasks {
        out.push(t.await.unwrap());
    }
    out
}

#[tokio::test]
async fn a_full_pool_queues_then_overflows() {
    let s = start_yaml(&pooled("pool: { size: 1, queue: 1 }")).await;
    let mut got = burst(&s, "/users/1", 3).await;
    got.sort_by_key(|g| g.1);
    let statuses: Vec<u16> = got.iter().map(|g| g.0).collect();
    assert_eq!(
        statuses.iter().filter(|s| **s == 200).count(),
        2,
        "{statuses:?}"
    );
    assert_eq!(
        statuses.iter().filter(|s| **s == 503).count(),
        1,
        "{statuses:?}"
    );
    let ok: Vec<_> = got.iter().filter(|g| g.0 == 200).collect();
    assert!(
        ok[1].1 >= Duration::from_millis(200),
        "the queued request waited a hop: {:?}",
        ok[1].1
    );
    assert!(ok[1].2.contains("db.wait="), "{}", ok[1].2);
    assert!(
        !ok[0].2.contains(".wait="),
        "the first request did not wait: {}",
        ok[0].2
    );
}

#[tokio::test]
async fn zero_load_trace_has_no_wait_pieces() {
    let s = start_yaml(&pooled("concurrency: 4")).await;
    let r = s.get("/users/1").await;
    assert_eq!(r.status(), 200);
    assert!(!trace_of(&r).contains("wait"), "{}", trace_of(&r));
}

#[tokio::test]
async fn rate_limit_429_carries_retry_after_and_error_map_can_drop_it() {
    let yaml = |entry: &str| {
        format!(
            "version: 1\nnodes:\n  nginx: {{ type: gateway, rate_limit: {{ rps: 1, burst: 1 }} }}\n  app: {{ type: service, spec: openapi.yaml }}\nedges:\n  - {{ from: nginx, to: app }}\nendpoints:\n{entry}"
        )
    };
    let s = start_yaml(&yaml("")).await;
    assert_eq!(s.get("/users/1").await.status(), 200);
    let r = s.get("/users/1").await;
    assert_eq!(r.status(), 429);
    assert_eq!(r.headers()["retry-after"], "1");
    assert!(
        trace_of(&r).ends_with(",nginx.error=429") || trace_of(&r) == "nginx.error=429",
        "{}",
        trace_of(&r)
    );

    let s = start_yaml(&yaml(
        "  \"GET /users/{id}\": { error_map: { \"429\": 503 } }\n",
    ))
    .await;
    s.get("/users/1").await;
    let r = s.get("/users/1").await;
    assert_eq!(r.status(), 503, "error_map turned the 429 into a 503");
    assert!(
        r.headers().get("retry-after").is_none(),
        "no Retry-After unless the final status is 429"
    );
}

#[tokio::test]
async fn a_chaos_down_node_beats_the_rate_limit() {
    let s = start_yaml("version: 1\nnodes:\n  nginx: { type: gateway, rate_limit: { rps: 1, burst: 1 } }\n  app: { type: service, spec: openapi.yaml }\nedges:\n  - { from: nginx, to: app }\n").await;
    s.put_chaos("nginx", json!({ "down": true })).await;
    for _ in 0..3 {
        assert_eq!(
            s.get("/users/1").await.status(),
            503,
            "a chaos failure takes no token, so it never becomes 429"
        );
    }
}

#[tokio::test]
async fn a_dropped_client_does_not_leak_a_slot() {
    let s = start_yaml(&pooled("pool: { size: 1, queue: 5 }")).await;
    // Start requests and abandon them after 30 ms; the engine keeps them until they end.
    for _ in 0..3 {
        let c = reqwest::Client::builder()
            .timeout(Duration::from_millis(30))
            .build()
            .unwrap();
        let _ = c.get(format!("{}/users/1", s.base)).send().await;
    }
    tokio::time::sleep(Duration::from_millis(1200)).await; // 3 abandoned hops of 300 ms drain
    let r = s.get("/users/1").await;
    assert_eq!(r.status(), 200, "the slot came back");
    assert!(
        !trace_of(&r).contains(".wait="),
        "and nobody is still queued: {}",
        trace_of(&r)
    );
}

#[tokio::test]
async fn many_parallel_requests_all_complete_through_one_slot() {
    // 40 requests x 2 ms through one slot with an unbounded queue: all complete, none overflow.
    let s = start_yaml("version: 1\nnodes:\n  nginx: { type: gateway }\n  app: { type: service, spec: openapi.yaml, concurrency: 1, proc: 2ms }\nedges:\n  - { from: nginx, to: app }\n").await;
    let got = burst(&s, "/users/1", 40).await;
    assert!(got.iter().all(|g| g.0 == 200));
}

#[tokio::test]
async fn failing_branch_answers_before_the_running_sibling_ends() {
    let yaml = "version: 1\nnodes:\n  app: { type: service, spec: openapi.yaml }\n  slow: { type: external-api, proc: 400ms }\n  fast: { type: external-api, proc: 60ms }\n  bad: { type: external-api }\nedges:\n  - { from: app, to: slow }\n  - { from: app, to: fast }\n  - { from: app, to: bad }\nendpoints:\n  \"GET /users/{id}\": { chain: [ { parallel: [ slow, [ fast, bad ] ] } ] }\n";
    let s = start_yaml(yaml).await;
    s.put_chaos("bad", json!({ "down": true })).await;
    let t = std::time::Instant::now();
    let r = s.get("/users/1").await;
    assert_eq!(r.status(), 503);
    assert!(
        t.elapsed() < Duration::from_millis(350),
        "answered at the failure (~60 ms), not when `slow` ends: {:?}",
        t.elapsed()
    );
}

#[tokio::test]
async fn cors_exposes_retry_after_to_browsers() {
    let s = start_yaml("version: 1\nnodes:\n  nginx: { type: gateway, rate_limit: { rps: 1, burst: 1 } }\n  app: { type: service, spec: openapi.yaml }\nedges:\n  - { from: nginx, to: app }\n").await;
    s.get("/users/1").await;
    let r = s.get("/users/1").await;
    assert_eq!(r.status(), 429);
    let expose = r.headers()["access-control-expose-headers"]
        .to_str()
        .unwrap()
        .to_ascii_lowercase();
    assert!(
        expose.contains("retry-after") && expose.contains("x-kaanal-trace"),
        "{expose}"
    );
}

/// Sum of the `=Xms` pieces of a trace without brackets: the simulated latency in ms.
pub(crate) fn trace_total_ms(trace: &str) -> f64 {
    trace
        .split(',')
        .filter_map(|p| {
            p.rsplit_once('=')?
                .1
                .strip_suffix("ms")?
                .parse::<f64>()
                .ok()
        })
        .sum()
}

/// `pct` in 0..=100.
pub(crate) fn pct(sorted: &[f64], pct: usize) -> f64 {
    sorted[(sorted.len() - 1) * pct / 100]
}

/// Open-loop load (a request every 1/RPS s whether or not the last one answered) against a
/// pooled database at about 75% utilisation, so some requests queue. Checks that every hop
/// keeps its exact simulated duration, that the trace never claims more time than the client
/// really waited, and how far the real latency drifts above the trace (HTTP + timer lag).
/// A third of the requests hit an `empty`-pinned endpoint (pinned by a scenario, so the pin path runs
/// under load), and every other one of the rest sends an empty `X-Kaanal-Scenario`, so the scoped path is under load too.
///
/// `cargo test -p kaanal-server --release --test http -- --ignored --nocapture load_`
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "load test; run with --release"]
async fn load_pooled_node_keeps_latency_and_trace_accurate() {
    const RPS: u64 = 3000;
    const SECS: u64 = 4;
    // db: 8 slots (concurrency: unbounded queue) x 2 ms = 4000 requests/s; 3000/s offered = 75% busy.
    let s = start_yaml("version: 1\nnodes:\n  nginx: { type: gateway }\n  app: { type: service, spec: openapi.yaml }\n  db: { type: database, ops: { read: 2ms, write: 2ms, scan: 2ms }, concurrency: 8 }\nedges:\n  - { from: nginx, to: app }\n  - { from: app, to: db }\nendpoints:\nscenarios:\n  pin-empty: { pins: [ { endpoint: \"GET /users\", state: empty } ] }\n").await;
    let client = reqwest::Client::builder()
        .pool_max_idle_per_host(256)
        .build()
        .unwrap();
    let url = format!("{}/users/1", s.base);
    let pinned_url = format!("{}/users", s.base);
    let mut tick = tokio::time::interval(Duration::from_nanos(1_000_000_000 / RPS));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Burst);
    let mut tasks = Vec::new();
    for i in 0..RPS * SECS {
        tick.tick().await;
        let (client, url) = (client.clone(), url.clone());
        let pinned_url = pinned_url.clone();
        tasks.push(tokio::spawn(async move {
            let t = std::time::Instant::now();
            // A third of the requests hit an `empty`-pinned endpoint (the pin comes from a
            // scenario, so it rides the scoped path; the chain still runs under a pin). Of the
            // rest, every other one carries an empty scope (an explicit baseline): the scoped
            // path (one read lock, one short-string hash) must not move the p99.
            let mut req = if i % 3 == 2 {
                client
                    .get(pinned_url)
                    .header("X-Kaanal-Scenario", "pin-empty")
            } else {
                client.get(url)
            };
            if i % 3 != 2 && i % 2 == 1 {
                req = req.header("X-Kaanal-Scenario", "");
            }
            let r = req.send().await.unwrap();
            let status = r.status().as_u16();
            let trace = r.headers()["x-kaanal-trace"].to_str().unwrap().to_string();
            (status, t.elapsed().as_secs_f64() * 1000.0, trace)
        }));
    }
    let (mut lag, mut waits, mut waited) = (Vec::new(), Vec::new(), 0usize);
    let mut pinned = 0usize;
    for (i, t) in tasks.into_iter().enumerate() {
        let (status, real, trace) = t.await.unwrap();
        assert_eq!(status, 200, "unbounded queue: nothing may fail ({trace})");
        if i % 3 == 2 {
            pinned += 1;
            assert!(trace.ends_with("pinned=empty"), "pin missing: {trace}");
        } else {
            assert!(!trace.contains("pinned="), "stray pin: {trace}");
        }
        assert!(
            trace.contains("db.read=2.0ms") || trace.contains("db.scan=2.0ms"),
            "every db hop keeps its exact 2 ms: {trace}"
        );
        let sim = trace_total_ms(&trace);
        assert!(
            real + 1.0 >= sim,
            "the trace claims {sim:.1} ms but the client waited {real:.1} ms: {trace}"
        );
        lag.push(real - sim);
        if let Some(w) = trace.split(',').find(|p| p.starts_with("db.wait=")) {
            waited += 1;
            waits.push(w["db.wait=".len()..w.len() - 2].parse::<f64>().unwrap());
        }
    }
    lag.sort_by(f64::total_cmp);
    waits.sort_by(f64::total_cmp);
    eprintln!(
        "{} requests, {waited} queued; real - trace ms: p50 {:.1} p99 {:.1} max {:.1}; db.wait ms: p50 {:.1} p99 {:.1}",
        lag.len(),
        pct(&lag, 50),
        pct(&lag, 99),
        lag[lag.len() - 1],
        waits.first().map_or(0.0, |_| pct(&waits, 50)),
        waits.first().map_or(0.0, |_| pct(&waits, 99)),
    );
    assert_eq!(pinned, (RPS * SECS / 3) as usize, "a third hit the pin");
    assert!(waited > 0, "at 75% load some requests must queue");
    assert!(
        pct(&lag, 50) < 25.0,
        "median lag above the trace: {:.1} ms",
        pct(&lag, 50)
    );
    assert!(
        pct(&lag, 99) < 150.0,
        "p99 lag above the trace: {:.1} ms",
        pct(&lag, 99)
    );
}

#[tokio::test]
async fn parallel_failure_tie_goes_to_the_first_branch_live() {
    // Both branches fail at t = 0 (default zero costs); the first branch reaches its failure
    // through a zero-duration hop. The first branch wins, as the docs promise.
    let yaml = "version: 1\nnodes:\n  app: { type: service, spec: openapi.yaml }\n  z: { type: external-api }\n  s: { type: external-api }\n  a: { type: external-api }\nedges:\n  - { from: app, to: z }\n  - { from: app, to: s }\n  - { from: app, to: a }\nendpoints:\n  \"GET /users/{id}\": { chain: [ { parallel: [ [ z, s ], [ a ] ] } ] }\n";
    let s = start_yaml(yaml).await;
    s.put_chaos("s", json!({ "down": true })).await;
    s.put_chaos("a", json!({ "error_rate": 1.0, "error_status": 500 }))
        .await;
    let r = s.get("/users/1").await;
    assert_eq!(r.status(), 503, "{}", trace_of(&r));
    assert!(trace_of(&r).ends_with("s.error=503"), "{}", trace_of(&r));
}
