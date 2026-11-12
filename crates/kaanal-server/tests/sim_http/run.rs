//! `POST /sim/run`: one run at a time, caps, the generation a report belongs to, chaos source.

use crate::common::*;
use serde_json::{Value, json};
use std::time::Duration;

/// `system()` plus profiles: `peak` is small, `big` is sized to take a while in a debug build.
fn config() -> String {
    format!(
        "{}traffic:
  peak:
    rps: 100
    journeys: [ {{ endpoint: 'GET /users/{{id}}' }} ]
  big:
    rps: 1000
    journeys: [ {{ endpoint: 'GET /users/{{id}}' }} ]
scenarios:
  slow: {{ chaos: [ {{ node: app, latency_ms: 50 }} ] }}
",
        system("1ms")
    )
}

async fn post(s: &Srv, body: Value) -> reqwest::Response {
    s.send(reqwest::Method::POST, "/__kaanal/sim/run", Some(body))
        .await
}

/// Polls run `id` until it is no longer running.
async fn finished(s: &Srv, id: &str) -> Value {
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

/// Starts a run that holds the slot for a while.
async fn start_big(s: &Srv) -> String {
    let v: Value = post(
        s,
        json!({ "profile": "big", "duration_s": 300, "warmup_s": 1 }),
    )
    .await
    .json()
    .await
    .unwrap();
    v["id"].as_str().expect("202 with an id").to_string()
}

#[tokio::test]
async fn a_run_finishes_and_reports_percentiles() {
    let s = start(&config()).await;
    let r = post(&s, json!({ "profile": "peak", "duration_s": 20 })).await;
    assert_eq!(r.status(), 202);
    let loc = r.headers()["location"].to_str().unwrap().to_string();
    let v: Value = r.json().await.unwrap();
    let id = v["id"].as_str().unwrap();
    assert_eq!(loc, format!("/__kaanal/sim/runs/{id}"));
    let done = finished(&s, id).await;
    assert_eq!(done["state"], "done", "{done}");
    assert_eq!(done["generation"], 1);
    let sum = &done["summary"];
    assert!(sum["requests"].as_u64().unwrap() > 1000, "{sum}");
    let l = &sum["latency"];
    let (p50, p95, p99) = (
        l["p50_ms"].as_f64().unwrap(),
        l["p95_ms"].as_f64().unwrap(),
        l["p99_ms"].as_f64().unwrap(),
    );
    assert!(p50 > 0.0 && p50 <= p95 && p95 <= p99, "{p50} {p95} {p99}");
    assert!(sum["label"].as_str().unwrap().contains("not a prediction"));
}

/// B4: a finished run carries the estimated cost: the sums of the cost card, here one t3.micro (EC2
/// list price $0.0104 an hour in us-east-1) with two nodes that name no instance left out, `why`
/// in each; the runs list leaves it out like the summary.
#[tokio::test]
async fn a_finished_run_carries_the_estimated_cost() {
    let yaml = config().replace(
        "spec: openapi.yaml, proc: 1ms }",
        "spec: openapi.yaml, proc: 1ms, instance: aws/t3.micro }",
    );
    let s = start(&yaml).await;
    let v: Value = post(&s, json!({ "profile": "peak", "duration_s": 20 }))
        .await
        .json()
        .await
        .unwrap();
    let id = v["id"].as_str().unwrap().to_string();
    let done = finished(&s, &id).await;
    assert_eq!(done["state"], "done", "{done}");
    let c = &done["cost"];
    assert_eq!(c["total_micro_usd_hr"], 10_400, "{c}");
    assert_eq!(
        (c["priced"].as_u64(), c["unpriced"].as_u64()),
        (Some(1), Some(2))
    );
    assert_eq!(c["regions"]["aws"], "us-east-1");
    let app = c["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["name"] == "app")
        .unwrap();
    assert_eq!(app["instance"], "aws/t3.micro");
    assert!(
        c["nodes"][0]["why"]
            .as_str()
            .unwrap()
            .contains("no instance"),
        "{c}"
    );
    let list: Value = s.get("/__kaanal/sim/runs").await.json().await.unwrap();
    assert!(
        list["runs"][0]["cost"].is_null(),
        "the list leaves the cost out"
    );
}

#[tokio::test]
async fn a_second_run_while_one_runs_is_409() {
    let s = start(&config()).await;
    let id = start_big(&s).await;
    let r = post(&s, json!({ "profile": "peak" })).await;
    assert_eq!(r.status(), 409);
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["id"], id.as_str());
    // the first is untouched and finishes, and then the slot is free
    assert_eq!(finished(&s, &id).await["state"], "done");
    let again = post(&s, json!({ "profile": "peak", "duration_s": 5 })).await;
    assert_eq!(again.status(), 202);
}

#[tokio::test]
async fn a_run_over_the_server_caps_is_400_with_the_library_message() {
    let s = start(&config()).await;
    let r = post(&s, json!({ "profile": "big", "duration_s": 3000 })).await;
    assert_eq!(r.status(), 400);
    let v: Value = r.json().await.unwrap();
    assert!(v["error"].as_str().unwrap().contains("too large"), "{v}");
    let runs: Value = s.get("/__kaanal/sim/runs").await.json().await.unwrap();
    assert_eq!(runs["runs"], json!([]), "a refused request starts no run");
}

#[tokio::test]
async fn an_apply_during_a_run_leaves_the_report_on_its_generation() {
    let s = start(&config()).await;
    let id = start_big(&s).await;
    let changed = config().replace("proc: 1ms }\n  db", "proc: 2ms }\n  db");
    assert_ne!(changed, config());
    s.put("/__kaanal/draft", json!({ "text": changed })).await;
    let a = s
        .send(reqwest::Method::POST, "/__kaanal/draft/apply", None)
        .await;
    assert_eq!(a.status(), 200);
    let done = finished(&s, &id).await;
    assert_eq!(done["state"], "done", "{done}");
    assert_eq!(
        done["generation"], 1,
        "the run keeps the generation it began on"
    );
    // a run started now is on the new one
    let next: Value = post(&s, json!({ "profile": "peak", "duration_s": 5 }))
        .await
        .json()
        .await
        .unwrap();
    let next = finished(&s, next["id"].as_str().unwrap()).await;
    assert_eq!(next["generation"], 2);
}

#[tokio::test]
async fn a_named_scenario_changes_the_result() {
    let s = start(&config()).await;
    let mut p50s = Vec::new();
    for extra in [json!({}), json!({ "scenario": "slow" })] {
        let mut body = json!({ "profile": "peak", "duration_s": 20 });
        body.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        let v: Value = post(&s, body).await.json().await.unwrap();
        let done = finished(&s, v["id"].as_str().unwrap()).await;
        p50s.push(done["summary"]["latency"]["p50_ms"].as_f64().unwrap());
    }
    assert!(
        p50s[1] > p50s[0] + 40.0,
        "plain {} ms, slow {} ms",
        p50s[0],
        p50s[1]
    );
}

#[tokio::test]
async fn bad_requests_are_told_apart() {
    let s = start(&config()).await;
    assert_eq!(post(&s, json!({ "profile": "nope" })).await.status(), 404);
    let unknown = post(&s, json!({ "profile": "peak", "scenario": "nope" })).await;
    assert_eq!(unknown.status(), 404);
    assert_eq!(post(&s, json!({ "nonsense": 1 })).await.status(), 400);
    let negative = post(&s, json!({ "profile": "peak", "duration_s": -1 })).await;
    assert_eq!(negative.status(), 400);
    let inline = json!({ "profile": { "rps": 5, "journeys": [ { "endpoint": "GET /nope" } ] } });
    let r = post(&s, inline).await;
    assert_eq!(r.status(), 400);
    assert!(r.text().await.unwrap().contains("unknown endpoint"));
    assert_eq!(s.get("/__kaanal/sim/runs/s99").await.status(), 404);
}

#[tokio::test]
async fn an_inline_profile_error_says_what_is_wrong() {
    let s = start(&config()).await;
    let bad =
        json!({ "profile": { "rps": 5_000_000, "journeys": [ { "endpoint": "GET /users" } ] } });
    let r = post(&s, bad).await;
    assert_eq!(r.status(), 400);
    assert!(r.text().await.unwrap().contains("rps must be between"));
    let unknown = json!({ "profile": { "rps": 5, "users": 1, "journeys": [ { "endpoint": "GET /users" } ] } });
    assert!(
        post(&s, unknown)
            .await
            .text()
            .await
            .unwrap()
            .contains("users")
    );
    assert_eq!(post(&s, json!({ "profile": 7 })).await.status(), 400);
}

#[tokio::test]
async fn the_request_seed_changes_the_run() {
    let s = start(&config()).await;
    let mut sums = Vec::new();
    for seed in [1, 2, 1] {
        let v: Value = post(
            &s,
            json!({ "profile": "peak", "duration_s": 20, "seed": seed }),
        )
        .await
        .json()
        .await
        .unwrap();
        sums.push(finished(&s, v["id"].as_str().unwrap()).await["summary"].clone());
    }
    assert_ne!(sums[0], sums[1], "another seed, another run");
    assert_eq!(sums[0], sums[2], "the same seed, the same run");
}

#[tokio::test]
async fn a_blank_start_answers_409_no_system() {
    let s = start_blank().await;
    assert_eq!(post(&s, json!({ "profile": "peak" })).await.status(), 409);
    let sample = s
        .send(
            reqwest::Method::POST,
            "/__kaanal/sample",
            Some(json!({ "endpoint": "GET /users" })),
        )
        .await;
    assert_eq!(sample.status(), 409);
}

/// p99 of `v` in milliseconds.
fn p99(mut v: Vec<f64>) -> f64 {
    v.sort_by(f64::total_cmp);
    v[(v.len() * 99 / 100).min(v.len() - 1)]
}

/// Measures how much a run slows the mocks (the trigger in
/// `internal/docs/suggestions/2026-10-28-process-isolation.md`: mock p99 up by more than 50 ms).
/// By hand, in release: `cargo test --release -p kaanal-server --test sim_http mocks_stay -- --ignored --nocapture`.
#[tokio::test]
#[ignore = "timing; run by hand in release"]
async fn mocks_stay_responsive_during_a_run() {
    let s = start(&config()).await;
    let url = format!("{}/users/1", s.base);
    let one = || async {
        let t = std::time::Instant::now();
        let r = s.client.get(&url).send().await.unwrap();
        assert_eq!(r.status(), 200);
        t.elapsed().as_secs_f64() * 1000.0
    };
    let mut idle = Vec::new();
    for _ in 0..300 {
        idle.push(one().await);
    }
    let id = start_big(&s).await;
    let mut during = Vec::new();
    let t0 = std::time::Instant::now();
    while s
        .get(&format!("/__kaanal/sim/runs/{id}"))
        .await
        .json::<Value>()
        .await
        .unwrap()["state"]
        == "running"
    {
        during.push(one().await);
    }
    let (a, b) = (p99(idle), p99(during.clone()));
    println!(
        "run took {:?}; {} mock requests during it; p99 idle {a:.2} ms, during {b:.2} ms, max during {:.2} ms",
        t0.elapsed(),
        during.len(),
        during.iter().copied().fold(0.0, f64::max)
    );
    assert!(b < a + 50.0, "mock p99 rose from {a:.2} to {b:.2} ms");
}
