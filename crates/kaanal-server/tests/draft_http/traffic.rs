//! `traffic:` in a draft: a patch can add a profile, and a bad one is a positioned diagnostic.

use crate::common::*;
use serde_json::{Value, json};

async fn validate(s: &Srv) -> Value {
    s.send(reqwest::Method::POST, "/__kaanal/draft/validate", None)
        .await
        .json()
        .await
        .unwrap()
}

#[tokio::test]
async fn a_patch_can_set_a_traffic_profile() {
    let s = start(BASE).await;
    let body = r#"{ "patch": [
        { "op": "insert", "path": ["traffic"], "value": { "peak": { "rps": 50, "journeys": [ { "endpoint": "GET /users", "weight": 3 } ] } } }
    ] }"#;
    let r = s
        .client
        .put(format!("{}/__kaanal/draft", s.base))
        .bearer_auth(&s.token)
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let v: Value = r.json().await.unwrap();
    assert!(v["text"].as_str().unwrap().contains("peak"), "{v}");
    assert_eq!(validate(&s).await["ok"], true);
}

#[tokio::test]
async fn a_bad_profile_is_a_diagnostic_at_its_key() {
    let s = start(BASE).await;
    let text = format!(
        "{BASE}traffic:\n  peak:\n    rps: 0\n    journeys: [ {{ endpoint: 'GET /users' }} ]\n"
    );
    s.put("/__kaanal/draft", json!({ "text": text })).await;
    let v = validate(&s).await;
    assert_eq!(v["ok"], false, "{v}");
    let d = &v["diagnostics"][0];
    assert!(
        d["message"]
            .as_str()
            .unwrap()
            .contains("rps must be between"),
        "{d}"
    );
    // a serde error is placed by line and column, as for every other bad value
    assert!(d["line"].is_number() && d["col"].is_number(), "{d}");
}
