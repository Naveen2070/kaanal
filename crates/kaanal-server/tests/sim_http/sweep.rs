//! `POST /sim/sweep`: a load sweep is a run record of kind `sweep`, under the same single slot.

use crate::common::*;
use serde_json::{Value, json};
use std::time::Duration;

/// `system()` plus profiles: `peak` is small, `big` holds the slot for a while in a debug build.
fn config() -> String {
    format!(
        "{}traffic:
  peak:
    rps: 100
    journeys: [ {{ endpoint: 'GET /users/{{id}}' }} ]
  big:
    rps: 1000
    journeys: [ {{ endpoint: 'GET /users/{{id}}' }} ]
",
        system("1ms")
    )
}

async fn post(s: &Srv, path: &str, body: Value) -> reqwest::Response {
    s.send(reqwest::Method::POST, path, Some(body)).await
}

async fn sweep(s: &Srv, body: Value) -> reqwest::Response {
    post(s, "/__kaanal/sim/sweep", body).await
}

/// Polls record `id` until it is no longer running.
async fn finished(s: &Srv, id: &str) -> Value {
    for _ in 0..2400 {
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
    panic!("record {id} did not finish");
}

#[tokio::test]
async fn sweep_route_runs_and_answers_a_record() {
    let s = start(&config()).await;
    // max_factor 1 keeps the ladder at five points: 0.25 to 1
    let r = sweep(&s, json!({ "profile": "peak", "max_factor": 1 })).await;
    assert_eq!(r.status(), 202);
    let loc = r.headers()["location"].to_str().unwrap().to_string();
    let v: Value = r.json().await.unwrap();
    let id = v["id"].as_str().unwrap();
    assert_eq!(v["state"], "running");
    assert_eq!(loc, format!("/__kaanal/sim/runs/{id}"));
    let done = finished(&s, id).await;
    assert_eq!(done["state"], "done", "{done}");
    assert_eq!(done["kind"], "sweep");
    assert_eq!(done["profile"], "peak");
    assert_eq!(done["generation"], 1);
    let sw = &done["sweep"];
    assert!(sw["verdict"]["kind"].is_string(), "{sw}");
    assert!(
        sw["label"].as_str().unwrap().contains("not a prediction"),
        "{sw}"
    );
    assert_eq!(sw["limit"], json!({ "p99_ms": 1000.0, "error_rate": 0.01 }));
    assert_ne!(sw["points"].as_array().unwrap().len(), 0, "{done}");
    assert_eq!(
        done["sweep_points"],
        json!([]),
        "spent once the sweep is done"
    );
    let p = &sw["points"][0];
    assert_eq!(p["over"], json!({ "p99": false, "errors": false }), "{p}");
    let log = s.get(&format!("/__kaanal/sim/runs/{id}/log")).await;
    assert_eq!(log.status(), 409, "a sweep has no log");
    let list: Value = s.get("/__kaanal/sim/runs").await.json().await.unwrap();
    let row = &list["runs"][0];
    assert_eq!(row["kind"], "sweep");
    assert!(row["sweep"].is_null(), "the list strips the result: {row}");
    assert_eq!(row["sweep_points"], json!([]), "and the points");
}

#[tokio::test]
async fn a_sweep_that_cannot_run_ends_failed_and_frees_the_slot() {
    let s = start(&format!(
        "{}traffic:
  huge:
    rps: 100000
    journeys: [ {{ endpoint: 'GET /users/{{id}}' }} ]
",
        system("1h")
    ))
    .await;
    // a one-hour node makes the point window ten hours: the lowest point (25 000 req/s for that
    // long) is over the hop caps, a hard error that ends the sweep `failed`
    let r = sweep(&s, json!({ "profile": "huge" })).await;
    assert_eq!(r.status(), 202);
    let id = r.json::<Value>().await.unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let done = finished(&s, &id).await;
    assert_eq!(done["state"], "failed", "{done}");
    assert_eq!(done["kind"], "sweep");
    assert!(
        done["error"].as_str().unwrap().contains("too large"),
        "{done}"
    );
    assert!(done["sweep"].is_null());
    assert_eq!(done["sweep_points"], json!([]), "nothing ran");
    // the slot is free again: the same request is refused for its size, not with a 409
    let again = sweep(&s, json!({ "profile": "huge" })).await;
    assert_eq!(again.status(), 202);
}

#[tokio::test]
async fn a_run_record_says_it_is_a_run() {
    let s = start(&config()).await;
    let v: Value = post(
        &s,
        "/__kaanal/sim/run",
        json!({ "profile": "peak", "duration_s": 5 }),
    )
    .await
    .json()
    .await
    .unwrap();
    let done = finished(&s, v["id"].as_str().unwrap()).await;
    assert_eq!(done["kind"], "run");
    assert!(done["sweep"].is_null());
}

#[tokio::test]
async fn a_sweep_while_a_run_runs_is_a_409_with_the_id() {
    let s = start(&config()).await;
    let v: Value = post(
        &s,
        "/__kaanal/sim/run",
        json!({ "profile": "big", "duration_s": 300, "warmup_s": 1 }),
    )
    .await
    .json()
    .await
    .unwrap();
    let id = v["id"].as_str().expect("202 with an id").to_string();
    let r = sweep(&s, json!({ "profile": "peak" })).await;
    assert_eq!(r.status(), 409);
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["id"], id.as_str());
    assert_eq!(finished(&s, &id).await["state"], "done");
}

#[tokio::test]
async fn draft_source_and_duration_are_refused() {
    let s = start(&config()).await;
    for extra in [
        json!({ "source": "draft" }),
        json!({ "source": "applied" }),
        json!({ "duration_s": 10 }),
        json!({ "warmup_s": 1 }),
    ] {
        let mut body = json!({ "profile": "peak" });
        body.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        let r = sweep(&s, body.clone()).await;
        assert_eq!(r.status(), 400, "{body}");
        let v: Value = r.json().await.unwrap();
        assert!(
            v["error"].as_str().unwrap().contains("unknown field"),
            "{v}"
        );
    }
    let runs: Value = s.get("/__kaanal/sim/runs").await.json().await.unwrap();
    assert_eq!(runs["runs"], json!([]), "a refused request starts nothing");
}

#[tokio::test]
async fn bad_limits_are_400() {
    let s = start(&config()).await;
    for bad in [
        json!({ "p99_ms": 0 }),
        json!({ "p99_ms": -5 }),
        json!({ "error_pct": 101 }),
        json!({ "error_pct": -1 }),
        json!({ "max_factor": 0.1 }),
        json!({ "max_factor": 0.99 }),
        json!({ "max_factor": 65 }),
    ] {
        let mut body = json!({ "profile": "peak" });
        body.as_object_mut()
            .unwrap()
            .extend(bad.as_object().unwrap().clone());
        let r = sweep(&s, body.clone()).await;
        assert_eq!(r.status(), 400, "{body}");
    }
    assert_eq!(sweep(&s, json!({ "profile": "nope" })).await.status(), 404);
    let unknown = sweep(&s, json!({ "profile": "peak", "scenario": "nope" })).await;
    assert_eq!(unknown.status(), 404);
    let runs: Value = s.get("/__kaanal/sim/runs").await.json().await.unwrap();
    assert_eq!(runs["runs"], json!([]), "a refused request starts nothing");
}
