//! The storage node in the live mock: a service calling an object store, its trace, chaos on it and
//! its rate limit answering 503.

use crate::common::*;
use serde_json::{Value, json};

/// Gateway → fixture service → the store `assets`; `store` is appended to the store's fields.
fn with_store(store: &str) -> String {
    format!(
        "version: 1\nnodes:\n  nginx: {{ type: gateway, proc: 1ms }}\n  app: {{ type: service, spec: openapi.yaml, proc: 1ms }}\n  assets: {{ type: storage, flavor: s3, first_byte: 5ms, throughput: 80MB/s{store} }}\nedges:\n  - {{ from: nginx, to: app, net: 1ms }}\n  - {{ from: app, to: assets, net: 1ms }}\nendpoints:\n  \"GET /users/{{id}}\": {{ chain: [ {{ call: assets, op: read }} ] }}\n"
    )
}

#[tokio::test]
async fn a_request_through_a_service_shows_the_store_hop_in_the_trace() {
    let s = start_yaml(&with_store("")).await;
    let r = s.get("/users/42").await;
    assert_eq!(r.status(), 200);
    let trace = trace_of(&r);
    // the store is a leaf with one service-time hop, like any other node, after the net hop to it
    assert!(
        trace.starts_with("nginx.proc=")
            && trace.contains(",app.proc=")
            && trace.ends_with(",net=1.0ms,assets.proc=5.0ms"),
        "{trace}"
    );
    assert!(
        !trace.contains("assets.read="),
        "no op hop at a store: {trace}"
    );
}

#[tokio::test]
async fn chaos_on_the_store_through_the_control_api_fails_the_request() {
    let s = start_yaml(&with_store("")).await;
    let r = s
        .put_chaos("assets", json!({ "error_rate": 1.0, "error_status": 503 }))
        .await;
    assert_eq!(r.status(), 200);
    let failed = s.get("/users/42").await;
    assert_eq!(failed.status(), 503);
    assert_eq!(
        failed.json::<Value>().await.unwrap(),
        json!({ "error": "chaos", "node": "assets" })
    );
    let r = s.put_chaos("assets", json!({ "down": true })).await;
    assert_eq!(r.status(), 200);
    assert_eq!(s.get("/users/42").await.status(), 503);
}

#[tokio::test]
async fn the_rate_limit_answers_503_not_429() {
    let s = start_yaml(&with_store(", rate_limit: { rps: 1, burst: 1 }")).await;
    let mut got = Vec::new();
    for _ in 0..4 {
        let r = s.get("/users/42").await;
        got.push(r.status().as_u16());
    }
    assert_eq!(got[0], 200, "{got:?}");
    assert!(got.iter().skip(1).any(|g| *g == 503), "{got:?}");
    assert!(
        got.iter().all(|g| *g == 200 || *g == 503),
        "never 429: {got:?}"
    );
}
