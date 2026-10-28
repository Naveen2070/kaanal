//! `GET` and `PUT /__kaanal/draft`: read the draft text, replace it.

use crate::common::*;
use serde_json::{Value, json};

const URL: &str = "/__kaanal/draft";

#[tokio::test]
async fn get_draft_returns_the_file_and_put_replaces_it() {
    let s = start(BASE).await;
    let v: Value = s.get(URL).await.json().await.unwrap();
    assert_eq!(
        v["text"], BASE,
        "the draft starts as the file text, comments and all"
    );
    assert_eq!(v["dirty"], false);
    assert_eq!(v["generation"], 1);

    let next = "version: 1\nnodes: {}\n";
    let r = s.put(URL, json!({ "text": next })).await;
    assert_eq!(r.status(), 200);
    let v: Value = r.json().await.unwrap();
    assert_eq!(
        (v["text"].as_str(), v["dirty"].as_bool()),
        (Some(next), Some(true))
    );

    let v: Value = s.get(URL).await.json().await.unwrap();
    assert_eq!(v["text"], next, "the replacement sticks");
    assert_eq!(v["dirty"], true);
    // putting the applied text back is clean again
    let v: Value = s
        .put(URL, json!({ "text": BASE }))
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(v["dirty"], false);
}

#[tokio::test]
async fn a_put_over_the_size_limit_is_400_and_changes_nothing() {
    let s = start(BASE).await;
    let big = "#".repeat(1024 * 1024 + 1);
    let r = s.put(URL, json!({ "text": big })).await;
    assert_eq!(r.status(), 400);
    let v: Value = r.json().await.unwrap();
    assert!(v["error"].as_str().unwrap().contains("1048576"), "{v}");
    let v: Value = s.get(URL).await.json().await.unwrap();
    assert_eq!(v["text"], BASE);
    // exactly at the limit is accepted
    let ok = "#".repeat(1024 * 1024);
    assert_eq!(s.put(URL, json!({ "text": ok })).await.status(), 200);
}

#[tokio::test]
async fn a_put_without_text_is_rejected_and_a_server_without_a_source_starts_empty() {
    let s = start_with(BASE, None).await;
    assert!(
        s.put(URL, json!({ "nope": 1 }))
            .await
            .status()
            .is_client_error()
    );
    let v: Value = s.get(URL).await.json().await.unwrap();
    assert_eq!(v["text"], "", "no draft source means an empty draft");
}

#[tokio::test]
async fn the_draft_routes_need_the_token() {
    let s = start(BASE).await;
    let r = s
        .client
        .get(format!("{}{URL}", s.base))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 401);
}
