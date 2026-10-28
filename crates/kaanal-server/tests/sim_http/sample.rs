//! `POST /sample`: the body the mock would generate, and its size.

use crate::common::*;
use serde_json::{Value, json};

async fn sample(s: &Srv, body: Value) -> reqwest::Response {
    s.send(reqwest::Method::POST, "/__kaanal/sample", Some(body))
        .await
}

#[tokio::test]
async fn a_route_with_a_schema_returns_a_body_and_its_size() {
    let s = start(&system("1ms")).await;
    let v: Value = sample(&s, json!({ "endpoint": "get /users/{id}" }))
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(v["endpoint"], "GET /users/{id}");
    assert_eq!(v["status"], 200);
    assert!(v["body"]["email"].is_string(), "{v}");
    assert_eq!(
        v["bytes"].as_u64().unwrap(),
        serde_json::to_vec(&v["body"]).unwrap().len() as u64
    );
}

#[tokio::test]
async fn the_same_seed_gives_the_same_body_and_another_seed_another() {
    let s = start(&system("1ms")).await;
    let get = |seed: u64| {
        let s = &s;
        async move {
            let v: Value = sample(s, json!({ "endpoint": "GET /users/{id}", "seed": seed }))
                .await
                .json()
                .await
                .unwrap();
            v["body"].clone()
        }
    };
    assert_eq!(get(1).await, get(1).await);
    assert_ne!(get(1).await, get(2).await);
}

#[tokio::test]
async fn bad_requests_are_told_apart() {
    let s = start(&system("1ms")).await;
    assert_eq!(
        sample(&s, json!({ "endpoint": "GET /nope" }))
            .await
            .status(),
        404
    );
    assert_eq!(
        sample(&s, json!({ "endpoint": "nonsense" })).await.status(),
        400
    );
    assert_eq!(sample(&s, json!({})).await.status(), 400);
}
