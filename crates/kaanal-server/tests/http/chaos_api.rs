//! The chaos control routes: set, patch, clear, validate, and per-endpoint and node-at-endpoint keys.

use crate::common::*;
use serde_json::{Value, json};

#[tokio::test]
async fn chaos_on_db_fails_requests_then_clears() {
    let s = start().await;
    let r = s
        .put_chaos("db", json!({ "error_rate": 1.0, "error_status": 503 }))
        .await;
    assert_eq!(r.status(), 200);
    assert_eq!(r.json::<Value>().await.unwrap()["error_rate"], 1.0);

    let failed = s.get("/users/42").await;
    assert_eq!(failed.status(), 503);
    assert_eq!(
        failed.json::<Value>().await.unwrap(),
        json!({ "error": "chaos", "node": "db" })
    );

    let cleared = s
        .auth(s.client.delete(format!("{}/__kaanal/chaos/db", s.base)))
        .send()
        .await
        .unwrap();
    assert_eq!(cleared.status(), 204);
    assert_eq!(s.get("/users/42").await.status(), 200);
}

#[tokio::test]
async fn chaos_patch_merges_and_lists() {
    let s = start().await;
    s.put_chaos("app", json!({ "latency_ms": 50 })).await;
    s.put_chaos("app", json!({ "down": true })).await;
    let all: Value = s.get_authed("/__kaanal/chaos").await.json().await.unwrap();
    assert_eq!(
        all["nodes"]["app"],
        json!({ "latency_ms": 50, "error_rate": 0.0, "error_kind": "status", "error_status": 503, "down": true })
    );
    let wiped = s
        .auth(s.client.delete(format!("{}/__kaanal/chaos", s.base)))
        .send()
        .await
        .unwrap();
    assert_eq!(wiped.status(), 204);
    let mut all: Value = s.get_authed("/__kaanal/chaos").await.json().await.unwrap();
    // `effective` (the compiled view) is checked in scenarios_http.rs; this is the live layer.
    all.as_object_mut().unwrap().remove("effective");
    assert_eq!(all, json!({ "nodes": {}, "endpoints": {}, "node_at": [] }));
}

#[tokio::test]
async fn latency_chaos_slows_requests() {
    let s = start().await;
    s.put_chaos("db", json!({ "latency_ms": 200 })).await;
    let t = std::time::Instant::now();
    assert_eq!(s.get("/users/1").await.status(), 200);
    assert!(t.elapsed() >= std::time::Duration::from_millis(200));
}

#[tokio::test]
async fn unknown_node_is_404_with_message() {
    let s = start().await;
    let r = s.put_chaos("redis", json!({ "down": true })).await;
    assert_eq!(r.status(), 404);
    assert_eq!(
        r.json::<Value>().await.unwrap()["error"],
        "unknown node 'redis'"
    );
}

#[tokio::test]
async fn a_malformed_endpoint_is_400_on_set_and_clear() {
    let s = start().await;
    let r = s
        .auth(
            s.client
                .put(format!("{}/__kaanal/chaos", s.base))
                .json(&json!({ "endpoint": "GET", "down": true })),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 400);
    assert!(r.json::<Value>().await.unwrap()["error"].is_string());
    let r = s
        .auth(
            s.client
                .delete(format!("{}/__kaanal/chaos?endpoint=nope", s.base)),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 400);
}

#[tokio::test]
async fn bad_chaos_values_are_4xx_with_json_errors() {
    let s = start().await;
    for bad in [
        json!({ "error_rate": 1.5 }),
        json!({ "error_status": 418 }),
        json!({ "latency_ms": 300_001 }),
        json!({ "latency_ms": -5 }),
        json!({ "bogus": 1 }),
    ] {
        let r = s.put_chaos("db", bad.clone()).await;
        assert!(r.status().is_client_error(), "{bad}: {}", r.status());
        assert!(
            r.json::<Value>().await.unwrap()["error"].is_string(),
            "{bad}"
        );
    }
    let rate = s.put_chaos("db", json!({ "error_rate": 1.5 })).await;
    assert_eq!(rate.status(), 400);
    let slow = s.put_chaos("db", json!({ "latency_ms": 300_001 })).await;
    assert_eq!(slow.status(), 400);
    // A rejected patch changes nothing.
    let mut all: Value = s.get_authed("/__kaanal/chaos").await.json().await.unwrap();
    // `effective` (the compiled view) is checked in scenarios_http.rs; this is the live layer.
    all.as_object_mut().unwrap().remove("effective");
    assert_eq!(all, json!({ "nodes": {}, "endpoints": {}, "node_at": [] }));
    let cap = s.put_chaos("db", json!({ "latency_ms": 300_000 })).await;
    assert_eq!(cap.status(), 200);
}
