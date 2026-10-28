//! `POST /__kaanal/draft/validate`: positioned diagnostics, and nothing running changes.

use crate::common::*;
use reqwest::Method;
use serde_json::{Value, json};

const VALIDATE: &str = "/__kaanal/draft/validate";

async fn validate(s: &Srv, body: Option<Value>) -> Value {
    let r = s.send(Method::POST, VALIDATE, body).await;
    assert_eq!(r.status(), 200, "validation is data, not an HTTP error");
    r.json().await.unwrap()
}

async fn put(s: &Srv, text: &str) {
    assert_eq!(
        s.put("/__kaanal/draft", json!({ "text": text }))
            .await
            .status(),
        200
    );
}

/// 1-based line of the first line of `text` containing `needle`.
fn line_of(text: &str, needle: &str) -> u64 {
    text.lines().position(|l| l.contains(needle)).unwrap() as u64 + 1
}

#[tokio::test]
async fn a_valid_draft_returns_ok_with_no_diagnostics() {
    let s = start(BASE).await;
    let v = validate(&s, None).await;
    assert_eq!(v["ok"], true, "{v}");
    assert_eq!(v["diagnostics"], json!([]));
    assert!(v["warnings"].is_array());
}

#[tokio::test]
async fn a_bad_edge_is_reported_with_its_position() {
    let s = start(BASE).await;
    let bad = BASE.replace("{ from: app, to: db,", "{ from: app, to: nope,");
    put(&s, &bad).await;
    let v = validate(&s, None).await;
    assert_eq!(v["ok"], false);
    let d = &v["diagnostics"][0];
    assert_eq!(d["line"], line_of(&bad, "to: nope"), "{d}");
    assert!(d["message"].as_str().unwrap().contains("nope"), "{d}");
    assert_eq!(d["path"], json!(["edges", 1, "to"]));
}

#[tokio::test]
async fn a_syntax_error_and_an_empty_draft_are_reported() {
    let s = start(BASE).await;
    put(&s, "version: 1\nnodes: [\n").await;
    let v = validate(&s, None).await;
    assert_eq!(v["ok"], false);
    assert!(v["diagnostics"][0]["line"].is_u64(), "{v}");
    put(&s, "").await;
    let v = validate(&s, None).await;
    assert_eq!(v["ok"], false, "an empty draft is not a system: {v}");
    assert_ne!(v["diagnostics"][0]["message"].as_str().unwrap(), "");
}

#[tokio::test]
async fn a_body_text_is_validated_without_being_stored() {
    let s = start(BASE).await;
    let v = validate(&s, Some(json!({ "text": "version: 1\nnodes: {}\n" }))).await;
    assert_eq!(v["ok"], false);
    let d: Value = s.get("/__kaanal/draft").await.json().await.unwrap();
    assert_eq!(d["text"], BASE, "the draft is untouched");
}

#[tokio::test]
async fn validate_does_not_change_the_running_system() {
    let s = start(BASE).await;
    let before: Value = s.get("/__kaanal/topology").await.json().await.unwrap();
    // a valid draft with an extra node: validate it, the running topology must not move
    let more = BASE.replace(
        "edges:",
        "  cache: { type: cache, hit_ratio: 0.5, hit: 1ms, miss: 2ms }\nedges:",
    );
    put(&s, &more).await;
    assert_eq!(validate(&s, None).await["ok"], true);
    let after: Value = s.get("/__kaanal/topology").await.json().await.unwrap();
    assert_eq!(before, after);
    let d: Value = s.get("/__kaanal/draft").await.json().await.unwrap();
    assert_eq!(d["generation"], 1);
}
