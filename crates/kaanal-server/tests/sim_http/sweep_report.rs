//! `GET /sim/runs/{id}/report`: a finished sweep as Markdown, or its JSON alone.

use crate::common::*;
use serde_json::{Value, json};
use std::time::Duration;

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

/// Starts a sweep and returns its id.
async fn start_sweep(s: &Srv, body: Value) -> String {
    let r = s
        .send(reqwest::Method::POST, "/__kaanal/sim/sweep", Some(body))
        .await;
    assert_eq!(r.status(), 202);
    r.json::<Value>().await.unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string()
}

/// Polls record `id` until it is no longer running.
async fn finished(s: &Srv, id: &str) {
    for _ in 0..2400 {
        let v: Value = s
            .get(&format!("/__kaanal/sim/runs/{id}"))
            .await
            .json()
            .await
            .unwrap();
        if v["state"] != "running" {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("record {id} did not finish");
}

async fn report(s: &Srv, id: &str, query: &str) -> reqwest::Response {
    s.get(&format!("/__kaanal/sim/runs/{id}/report{query}"))
        .await
}

async fn done_sweep(s: &Srv) -> String {
    let id = start_sweep(s, json!({ "profile": "peak", "max_factor": 1 })).await;
    finished(s, &id).await;
    id
}

#[tokio::test]
async fn a_finished_sweep_has_a_report_in_both_forms() {
    let s = start(&config()).await;
    let id = done_sweep(&s).await;
    let md = report(&s, &id, "").await;
    assert_eq!(md.status(), 200);
    assert_eq!(md.headers()["content-type"], "text/markdown; charset=utf-8");
    let md = md.text().await.unwrap();
    let data = md
        .split("## Data\n\n```json\n")
        .nth(1)
        .expect("a Data fence");
    let fenced: Value = serde_json::from_str(data.split("\n```").next().unwrap()).unwrap();
    let j = report(&s, &id, "?format=json").await;
    assert_eq!(j.status(), 200);
    assert_eq!(j.headers()["content-type"], "application/json");
    assert_eq!(j.json::<Value>().await.unwrap(), fenced);
    assert_eq!(
        report(&s, &id, "?format=markdown").await.status(),
        200,
        "markdown is the named default"
    );
}

#[tokio::test]
async fn the_bytes_are_stable_across_two_fetches() {
    let s = start(&config()).await;
    let id = done_sweep(&s).await;
    for q in ["", "?format=json"] {
        let a = report(&s, &id, q).await.bytes().await.unwrap();
        let b = report(&s, &id, q).await.bytes().await.unwrap();
        assert_eq!(a, b, "{q}");
    }
}

#[tokio::test]
async fn a_running_sweep_is_409() {
    let s = start(&config()).await;
    let id = start_sweep(&s, json!({ "profile": "big" })).await;
    let r = report(&s, &id, "").await;
    assert_eq!(r.status(), 409);
    let v: Value = r.json().await.unwrap();
    assert!(v["error"].as_str().unwrap().contains("not finished"), "{v}");
}

#[tokio::test]
async fn a_failed_sweep_is_409_with_its_error() {
    let s = start(&format!(
        "{}traffic:
  huge:
    rps: 100000
    journeys: [ {{ endpoint: 'GET /users/{{id}}' }} ]
",
        system("1h")
    ))
    .await;
    let id = start_sweep(&s, json!({ "profile": "huge" })).await;
    finished(&s, &id).await;
    let r = report(&s, &id, "").await;
    assert_eq!(r.status(), 409);
    let v: Value = r.json().await.unwrap();
    assert!(v["error"].as_str().unwrap().contains("too large"), "{v}");
}

#[tokio::test]
async fn a_run_is_409_and_an_unknown_id_is_404() {
    let s = start(&config()).await;
    let v: Value = s
        .send(
            reqwest::Method::POST,
            "/__kaanal/sim/run",
            Some(json!({ "profile": "peak", "duration_s": 5 })),
        )
        .await
        .json()
        .await
        .unwrap();
    let id = v["id"].as_str().unwrap();
    finished(&s, id).await;
    let r = report(&s, id, "").await;
    assert_eq!(r.status(), 409);
    let v: Value = r.json().await.unwrap();
    assert!(v["error"].as_str().unwrap().contains("Studio"), "{v}");
    assert_eq!(report(&s, "nope", "").await.status(), 404);
}

#[tokio::test]
async fn a_bad_format_value_is_400() {
    let s = start(&config()).await;
    let id = done_sweep(&s).await;
    for q in ["?format=xml", "?format=", "?format=JSON", "?bogus=1"] {
        assert_eq!(report(&s, &id, q).await.status(), 400, "{q}");
    }
}
