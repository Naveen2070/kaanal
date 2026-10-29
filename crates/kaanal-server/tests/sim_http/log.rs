//! `GET /sim/runs/{id}/log`: the sampled run log, its filters and paging, and its memory bound.

use crate::common::*;
use serde_json::{Value, json};
use std::time::Duration;

fn config() -> String {
    format!(
        "{}traffic:
  peak:
    rps: 200
    journeys: [ {{ endpoint: 'GET /users/{{id}}' }} ]
  big:
    rps: 1000
    journeys: [ {{ endpoint: 'GET /users/{{id}}' }} ]
",
        system("1ms").replace(
            "spec: openapi.yaml, proc: 1ms",
            "spec: openapi.yaml, proc: 1ms, replicas: 2"
        )
    )
}

/// Polls run `id` until it is no longer running (bounded: 60 s).
async fn wait(s: &Srv, id: &str) -> Value {
    for _ in 0..1200 {
        let v: Value = s
            .get(&format!("/__kaanal/sim/runs/{id}"))
            .await
            .json()
            .await
            .unwrap();
        if v["state"] != "running" {
            return v;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("run {id} did not finish");
}

async fn run(s: &Srv, body: Value) -> String {
    let r = s
        .send(reqwest::Method::POST, "/__kaanal/sim/run", Some(body))
        .await;
    assert_eq!(r.status(), 202);
    let v: Value = r.json().await.unwrap();
    let id = v["id"].as_str().unwrap().to_string();
    let done = wait(s, &id).await;
    assert_eq!(done["state"], "done", "{done}");
    assert!(
        done.get("log").is_none(),
        "the lines stay out of the record"
    );
    id
}

async fn log(s: &Srv, id: &str, q: &str) -> (u16, Value) {
    let r = s.get(&format!("/__kaanal/sim/runs/{id}/log?{q}")).await;
    (r.status().as_u16(), r.json().await.unwrap())
}

fn lines(v: &Value) -> &Vec<Value> {
    v["lines"].as_array().unwrap()
}

/// Every line of the sample, paged.
async fn all(s: &Srv, id: &str) -> Vec<Value> {
    let mut out = Vec::new();
    let mut after = String::new();
    loop {
        let (_, p) = log(s, id, &format!("limit=1000{after}")).await;
        out.extend(lines(&p).iter().cloned());
        match p["next"].as_u64() {
            Some(n) => after = format!("&after={n}"),
            None => return out,
        }
    }
}

/// A run with a 2% failure on `db`: most requests succeed.
async fn flaky(s: &Srv) -> String {
    s.put(
        "/__kaanal/chaos/db",
        json!({ "error_rate": 0.02, "error_status": 500 }),
    )
    .await;
    run(s, json!({ "profile": "peak", "duration_s": 60 })).await
}

#[tokio::test]
async fn every_failure_is_in_the_sample_even_when_most_requests_succeed() {
    let s = start(&config()).await;
    let id = flaky(&s).await;
    let rec = wait(&s, &id).await;
    let failed: u64 = rec["summary"]["errors"]
        .as_object()
        .unwrap()
        .values()
        .map(|v| v.as_u64().unwrap())
        .sum();
    assert!(failed > 20, "{rec}");
    let (code, v) = log(&s, &id, "level=error&limit=1000").await;
    assert_eq!(code, 200, "{v}");
    let want = usize::try_from(failed).unwrap().min(1000);
    assert_eq!(lines(&v).len(), want, "{failed} failed requests");
    assert!(
        lines(&v)
            .iter()
            .all(|l| l["event"] == "failed" && l["status"] == 500)
    );
    assert_eq!(v["sampled"], true);
    assert!(v["note"].as_str().unwrap().contains("warmup"));
}

#[tokio::test]
async fn the_slowest_requests_are_in_the_sample() {
    let s = start(&config()).await;
    let id = flaky(&s).await;
    let rec = wait(&s, &id).await;
    let max_ms = rec["summary"]["latency"]["max_ms"].as_f64().unwrap();
    let lines = all(&s, &id).await;
    // a request's lines span its latency: the last line ends at arrival + latency, so the
    // slowest request is present if some request has a line stretch close to the maximum
    let mut by: std::collections::BTreeMap<u64, (f64, f64)> = std::collections::BTreeMap::new();
    for l in &lines {
        let t = l["t_ms"].as_f64().unwrap();
        let e = t + l["service_ms"].as_f64().unwrap_or(0.0);
        let r = by.entry(l["request"].as_u64().unwrap()).or_insert((t, e));
        r.0 = r.0.min(t);
        r.1 = r.1.max(e);
    }
    let widest = by.values().map(|(a, b)| b - a).fold(0.0, f64::max);
    assert!(
        widest >= max_ms * 0.8,
        "widest logged request {widest} ms vs max {max_ms} ms"
    );
}

#[tokio::test]
async fn the_log_for_a_node_has_only_its_hops() {
    let s = start(&config()).await;
    let id = flaky(&s).await;
    let (_, v) = log(&s, &id, "node=db&limit=1000").await;
    assert_ne!(lines(&v).len(), 0);
    assert!(lines(&v).iter().all(|l| l["node"] == "db"));
    let (code, v) = log(&s, &id, "node=nope").await;
    assert_eq!((code, lines(&v).len()), (200, 0));
    assert_eq!(v["next"], Value::Null);
    let q = "endpoint=GET%20/users/%7Bid%7D&node=db&level=info&limit=5";
    let (_, v) = log(&s, &id, q).await;
    assert_eq!(lines(&v).len(), 5);
    assert_eq!(log(&s, &id, "level=loud").await.0, 400);
    assert_eq!(log(&s, &id, "request=x").await.0, 400);
    assert_eq!(log(&s, &id, "replica=x").await.0, 400);
}

#[tokio::test]
async fn level_error_returns_only_failed_lines() {
    let s = start(&config()).await;
    let id = flaky(&s).await;
    let (_, v) = log(&s, &id, "level=error&limit=1000").await;
    assert_ne!(lines(&v).len(), 0);
    assert!(lines(&v).iter().all(|l| l["level"] == "error"));
    let (_, w) = log(&s, &id, "level=info&limit=1000").await;
    assert!(lines(&w).iter().all(|l| l["level"] == "info"));
}

#[tokio::test]
async fn paging_with_after_walks_the_whole_sample_once() {
    let s = start(&config()).await;
    let id = flaky(&s).await;
    let (_, first) = log(&s, &id, "limit=3000").await;
    assert_eq!(lines(&first).len(), 1000, "limit is clamped to 1000");
    let total = usize::try_from(first["total"].as_u64().unwrap()).unwrap();
    let mut seen = 0;
    let mut after = String::new();
    loop {
        let (_, p) = log(&s, &id, &format!("limit=700{after}")).await;
        seen += lines(&p).len();
        match p["next"].as_u64() {
            Some(n) => after = format!("&after={n}"),
            None => break,
        }
    }
    assert_eq!(seen, total);
    let (_, d) = log(&s, &id, "").await;
    assert_eq!(lines(&d).len(), 200, "default limit");
    let t: Vec<f64> = lines(&first)
        .iter()
        .map(|l| l["t_ms"].as_f64().unwrap())
        .collect();
    assert!(t.windows(2).all(|w| w[0] <= w[1]), "virtual-time order");
}

#[tokio::test]
async fn a_running_run_is_a_409_and_an_unknown_id_a_404() {
    let s = start(&config()).await;
    let r = s
        .send(
            reqwest::Method::POST,
            "/__kaanal/sim/run",
            Some(json!({ "profile": "big", "duration_s": 300, "warmup_s": 1 })),
        )
        .await;
    let v: Value = r.json().await.unwrap();
    let id = v["id"].as_str().unwrap();
    assert_eq!(log(&s, id, "").await.0, 409);
    assert_eq!(log(&s, "s999", "").await.0, 404);
    // let it finish: afterwards the same id answers
    wait(&s, id).await;
    assert_eq!(log(&s, id, "").await.0, 200);
}

#[tokio::test]
async fn a_run_past_the_line_cap_is_truncated_and_says_so() {
    // link hops on both edges make a request about nine lines: 4000 of them pass the cap
    let wide = config().replace("net: 1ms }", "net: 1ms, bandwidth: 1000000000 }")
        + "  wide:
    rps: 500
    journeys: [ { endpoint: 'GET /users/{id}', req_bytes: 1000, resp_bytes: 1000 } ]
";
    let s = start(&wide).await;
    s.put(
        "/__kaanal/chaos/db",
        json!({ "error_rate": 0.5, "error_status": 500 }),
    )
    .await;
    // near the server caps: 500 rps for 300 s
    let id = run(
        &s,
        json!({ "profile": "wide", "duration_s": 300, "warmup_s": 1 }),
    )
    .await;
    let (_, v) = log(&s, &id, "limit=1").await;
    let total = v["total"].as_u64().unwrap();
    assert!(total <= 20_000, "{total}");

    assert!(v["note"].as_str().unwrap().contains("fewer requests"));
    assert!(
        v["note"]
            .as_str()
            .unwrap()
            .contains("capped at 20000 lines")
    );
    // a small run keeps everything and says nothing of a cap
    let id2 = run(&s, json!({ "profile": "peak", "duration_s": 5 })).await;
    let (_, w) = log(&s, &id2, "limit=1").await;
    assert!(w["total"].as_u64().unwrap() <= 20_000);
    assert!(!w["note"].as_str().unwrap().contains("capped"));
}

#[tokio::test]
async fn lines_name_replicas_for_a_replicated_node() {
    let s = start(&config()).await;
    let id = flaky(&s).await;
    for r in 0..2 {
        let (_, v) = log(&s, &id, &format!("node=app&replica={r}&limit=1000")).await;
        assert!(!lines(&v).is_empty(), "replica {r}");
        assert!(
            lines(&v)
                .iter()
                .all(|l| l["replica"] == r && l["node"] == "app")
        );
    }
}

/// A db with one connection and one waiting place, so a loaded run overflows it.
fn jammed() -> String {
    format!(
        "{}traffic:
  jam:
    rps: 100
    journeys: [ {{ endpoint: 'GET /users/{{id}}' }} ]
",
        system("1ms").replace(
            "ops: { read: 1ms, write: 1ms, scan: 1ms }",
            "ops: { read: 100ms, write: 100ms, scan: 100ms }, pool: { size: 1, queue: 1 }"
        )
    )
}

#[tokio::test]
async fn a_chaos_status_equal_to_the_overflow_status_is_failed_and_a_full_queue_overflowed() {
    let s = start(&jammed()).await;
    // 503 is also the status a full queue answers with
    s.put(
        "/__kaanal/chaos/db",
        json!({ "error_rate": 0.3, "error_status": 503 }),
    )
    .await;
    let id = run(&s, json!({ "profile": "jam", "duration_s": 20 })).await;
    let errs: Vec<Value> = all(&s, &id)
        .await
        .into_iter()
        .filter(|l| l["level"] == "error")
        .collect();
    let of = |e: &str| errs.iter().filter(|l| l["event"] == e).count();
    assert!(of("overflowed") > 0, "a real full queue");
    assert!(of("failed") > 0, "chaos 503 is not a full queue");
    assert!(
        errs.iter()
            .filter(|l| l["event"] == "overflowed")
            .all(|l| l["note"] == "queue full")
    );
    assert!(
        errs.iter()
            .filter(|l| l["event"] == "failed")
            .all(|l| l["note"] == "status")
    );
}

#[tokio::test]
async fn failure_lines_carry_no_replica_and_a_replica_filter_skips_them() {
    let s = start(&config()).await;
    let id = flaky(&s).await;
    let errs = all(&s, &id)
        .await
        .into_iter()
        .filter(|l| l["level"] == "error")
        .collect::<Vec<_>>();
    assert_ne!(errs.len(), 0);
    assert!(
        errs.iter()
            .all(|l| l["replica"].is_null() && l["wait_ms"].is_null() && l["service_ms"].is_null())
    );
    let (_, v) = log(&s, &id, "replica=0&level=error").await;
    assert_eq!(lines(&v).len(), 0);
    let (_, v) = log(&s, &id, "limit=1").await;
    assert!(v["note"].as_str().unwrap().contains("no replica"));
}

#[tokio::test]
async fn request_filter_returns_one_requests_lines() {
    let s = start(&config()).await;
    let id = flaky(&s).await;
    let (_, v) = log(&s, &id, "limit=1").await;
    let n = lines(&v)[0]["request"].as_u64().unwrap();
    let (_, v) = log(
        &s,
        &id,
        &format!("endpoint=GET%20/users/%7Bid%7D&request={n}&limit=1000"),
    )
    .await;
    assert_ne!(lines(&v).len(), 0);
    assert!(lines(&v).iter().all(|l| l["request"] == n));
    let (_, v) = log(&s, &id, "request=999999999").await;
    assert_eq!(lines(&v).len(), 0);
}

#[tokio::test]
async fn filters_combine_with_and() {
    let s = start(&config()).await;
    let id = flaky(&s).await;
    let (_, v) = log(&s, &id, "limit=1").await;
    let first = &lines(&v)[0];
    let (n, node) = (
        first["request"].as_u64().unwrap(),
        first["node"].as_str().unwrap(),
    );
    let q = format!("endpoint=GET%20/users/%7Bid%7D&node={node}&request={n}&level=info&limit=1000");
    let (code, v) = log(&s, &id, &q).await;
    assert_eq!(code, 200);
    assert!(
        lines(&v)
            .iter()
            .all(|l| l["request"] == n && l["node"] == node && l["level"] == "info"),
        "{v}"
    );
    // one condition that cannot hold empties the page
    let (_, v) = log(&s, &id, &format!("{q}&replica=99")).await;
    assert_eq!(lines(&v).len(), 0);
}
