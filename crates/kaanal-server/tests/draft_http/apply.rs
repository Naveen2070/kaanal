//! `POST /__kaanal/draft/apply`: build, drain, swap.

use crate::common::*;
use reqwest::Method;
use serde_json::{Value, json};
use std::time::{Duration, Instant};

const APPLY: &str = "/__kaanal/draft/apply";
const DRAFT: &str = "/__kaanal/draft";

async fn put(s: &Srv, text: &str) {
    assert_eq!(s.put(DRAFT, json!({ "text": text })).await.status(), 200);
}

async fn apply(s: &Srv, body: Option<Value>) -> reqwest::Response {
    s.send(Method::POST, APPLY, body).await
}

async fn generation(s: &Srv) -> u64 {
    let v: Value = s.get(DRAFT).await.json().await.unwrap();
    v["generation"].as_u64().unwrap()
}

/// A mock request (no token), with its status and how long it took.
async fn mock(s: &Srv, path: &str) -> (u16, Duration) {
    let began = Instant::now();
    let resp = s
        .client
        .get(format!("{}{path}", s.base))
        .send()
        .await
        .unwrap();
    (resp.status().as_u16(), began.elapsed())
}

/// `base` plus a node `ext` with the `GET /ping` spec, wired to the gateway.
fn with_ext(base: &str) -> String {
    base.replace(
        "edges:",
        "  ext:   { type: service, spec: extra.yaml, proc: 1ms }\nedges:\n  - { from: nginx, to: ext, net: 1ms }",
    )
}

#[tokio::test]
async fn apply_swaps_the_routes() {
    let fast = system("1ms");
    let s = start(&fast).await;
    assert_eq!(mock(&s, "/ping").await.0, 404);
    put(&s, &with_ext(&fast)).await;
    let resp = apply(&s, None).await;
    assert_eq!(resp.status(), 200);
    let v: Value = resp.json().await.unwrap();
    assert_eq!(
        (v["ok"].as_bool(), v["generation"].as_u64()),
        (Some(true), Some(2))
    );
    assert_eq!(mock(&s, "/ping").await.0, 200, "the new route serves");
    assert_eq!(mock(&s, "/users/1").await.0, 200, "the old one still does");
    let dr: Value = s.get(DRAFT).await.json().await.unwrap();
    assert_eq!(
        (dr["dirty"].as_bool(), dr["generation"].as_u64()),
        (Some(false), Some(2))
    );
    let topo: Value = s.get("/__kaanal/topology").await.json().await.unwrap();
    assert!(
        topo["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .any(|n| n["name"] == "ext")
    );
}

#[tokio::test]
async fn a_request_in_flight_finishes_on_the_old_generation() {
    let slow = system("800ms");
    let s = start(&slow).await;
    let (base, client) = (s.base.clone(), s.client.clone());
    let inflight = tokio::spawn(async move {
        let began = Instant::now();
        let resp = client.get(format!("{base}/users/1")).send().await.unwrap();
        (resp.status().as_u16(), began.elapsed())
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    put(&s, &system("1ms")).await;
    let began = Instant::now();
    let resp = apply(&s, Some(json!({ "timeout_ms": 5000 }))).await;
    assert_eq!(resp.status(), 200);
    assert!(
        began.elapsed() > Duration::from_millis(400),
        "apply waited for the request to finish"
    );
    let (status, took) = inflight.await.unwrap();
    assert_eq!(status, 200);
    assert!(
        took > Duration::from_millis(700),
        "it ran its old 800 ms chain: {took:?}"
    );
    let (status, took) = mock(&s, "/users/1").await;
    assert_eq!(status, 200);
    assert!(
        took < Duration::from_millis(400),
        "the new system is the fast one: {took:?}"
    );
}

#[tokio::test]
async fn an_apply_that_cannot_drain_keeps_the_old_system() {
    let slow = system("1500ms");
    let s = start(&slow).await;
    let (base, client) = (s.base.clone(), s.client.clone());
    let inflight = tokio::spawn(async move {
        client
            .get(format!("{base}/users/1"))
            .send()
            .await
            .unwrap()
            .status()
            .as_u16()
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    let fast = system("1ms");
    put(&s, &fast).await;
    let resp = apply(&s, Some(json!({ "timeout_ms": 100 }))).await;
    assert_eq!(resp.status(), 409);
    let v: Value = resp.json().await.unwrap();
    assert!(
        v["error"].as_str().unwrap().contains("drain timed out"),
        "{v}"
    );
    let dr: Value = s.get(DRAFT).await.json().await.unwrap();
    assert_eq!(
        (dr["generation"].as_u64(), dr["dirty"].as_bool()),
        (Some(1), Some(true))
    );
    assert_eq!(dr["text"], fast, "the draft is kept");
    let began = Instant::now();
    assert_eq!(mock(&s, "/nope").await.0, 404);
    assert!(
        began.elapsed() < Duration::from_millis(500),
        "admission resumed after the timeout"
    );
    assert_eq!(
        inflight.await.unwrap(),
        200,
        "the request in flight is untouched"
    );
    // and the same Apply goes through once nothing is in flight
    assert_eq!(apply(&s, None).await.status(), 200);
    assert_eq!(generation(&s).await, 2);
}

#[tokio::test]
async fn a_request_during_the_pause_waits_and_then_uses_the_new_generation() {
    let slow = system("800ms");
    let s = start(&slow).await;
    let (base, client) = (s.base.clone(), s.client.clone());
    let inflight = tokio::spawn(async move {
        client
            .get(format!("{base}/users/1"))
            .send()
            .await
            .unwrap()
            .status()
            .as_u16()
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    // the new system has no spec on `app`, so `/users/{id}` is gone from it
    put(&s, &slow.replace("spec: openapi.yaml, ", "")).await;
    let (base, token, client) = (s.base.clone(), s.token.clone(), s.client.clone());
    let applier = tokio::spawn(async move {
        client
            .post(format!("{base}{APPLY}"))
            .bearer_auth(token)
            .send()
            .await
            .unwrap()
            .status()
            .as_u16()
    });
    tokio::time::sleep(Duration::from_millis(150)).await;
    let (status, took) = mock(&s, "/users/2").await;
    assert_eq!(
        status, 404,
        "it ran on the new system, which has no such route"
    );
    assert!(
        took > Duration::from_millis(400),
        "and it waited for the swap: {took:?}"
    );
    assert_eq!(
        inflight.await.unwrap(),
        200,
        "the request already inside answered from the old one"
    );
    assert_eq!(applier.await.unwrap(), 200);
}

#[tokio::test]
async fn an_invalid_draft_is_400_with_diagnostics_and_changes_nothing() {
    let fast = system("1ms");
    let s = start(&fast).await;
    put(&s, &fast.replace("to: db", "to: nope")).await;
    let resp = apply(&s, None).await;
    assert_eq!(resp.status(), 400);
    let v: Value = resp.json().await.unwrap();
    assert!(v["error"].as_str().unwrap().contains("nope"), "{v}");
    assert_eq!(v["diagnostics"][0]["path"], json!(["edges", 1, "to"]));
    assert_eq!(generation(&s).await, 1);
    assert_eq!(
        mock(&s, "/users/1").await.0,
        200,
        "the old system serves on"
    );
    assert_eq!(
        apply(&s, Some(json!({ "timeout_ms": 60_001 })))
            .await
            .status(),
        400
    );
}

#[tokio::test]
async fn apply_keeps_chaos_by_name_and_loads_the_new_drafts_scenarios() {
    let base = format!(
        "{}scenarios:\n  s1: {{ chaos: [ {{ node: db, latency_ms: 5 }} ] }}\n",
        system("1ms")
    );
    let s = start(&base).await;
    assert_eq!(
        s.put("/__kaanal/chaos/db", json!({ "latency_ms": 7 }))
            .await
            .status(),
        200
    );
    assert_eq!(
        s.send(Method::POST, "/__kaanal/scenarios/s1/activate", None)
            .await
            .status(),
        200
    );
    let next = format!("{base}  s2: {{ chaos: [ {{ node: app, latency_ms: 3 }} ] }}\n");
    put(&s, &next).await;
    let v: Value = apply(&s, None).await.json().await.unwrap();
    assert_eq!(v["kept"]["live_chaos"], 1, "db is still there: {v}");
    assert_eq!(v["kept"]["active_scenarios"], json!(["s1"]));
    assert_eq!(v["dropped"]["live_chaos"], json!([]));
    let chaos: Value = s.get("/__kaanal/chaos").await.json().await.unwrap();
    assert_eq!(
        chaos["nodes"]["db"]["latency_ms"], 7,
        "the live chaos survived: {chaos}"
    );
    let sc: Value = s.get("/__kaanal/scenarios").await.json().await.unwrap();
    let names: Vec<&str> = sc
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["s1", "s2"], "the new draft's scenarios are loaded");
    let active: Value = s
        .get("/__kaanal/scenarios/active")
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(active["active"], json!(["s1"]));
}
