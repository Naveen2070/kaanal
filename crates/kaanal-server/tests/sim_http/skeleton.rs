//! `GET /openapi/skeleton`: a loadable spec for the draft's endpoints, before any Apply.

use crate::common::*;
use serde_json::{Value, json};

const URL: &str = "/__kaanal/openapi/skeleton";

const NO_SPEC: &str = "version: 1
nodes:
  gw: { type: gateway }
  app: { type: service }
edges:
  - { from: gw, to: app }
endpoints:
  'GET /users': { chain: [] }
  'GET /users/{id}': { chain: [] }
  'POST /orders': { override: { status: 201, body: {} } }
";

#[tokio::test]
async fn the_skeleton_comes_from_the_draft_and_loads_as_a_spec() {
    let s = start_blank().await;
    s.put("/__kaanal/draft", json!({ "text": NO_SPEC })).await;
    let r = s.get(URL).await;
    assert_eq!(r.status(), 200);
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["endpoints"], 3);
    let model = kaanal_data::ApiModel::from_openapi(v["text"].as_str().unwrap()).unwrap();
    let mut keys: Vec<String> = model
        .endpoints
        .iter()
        .map(|e| format!("{} {}", e.method, e.path))
        .collect();
    keys.sort();
    assert_eq!(keys, ["GET /users", "GET /users/{id}", "POST /orders"]);
    let post = model.endpoints.iter().find(|e| e.method == "POST").unwrap();
    assert_eq!(post.status, 201);
}

#[tokio::test]
async fn a_draft_with_no_endpoints_is_409_and_a_bad_draft_is_400() {
    let s = start(&system("1ms")).await;
    let r = s.get(URL).await;
    assert_eq!(r.status(), 409);
    assert!(r.text().await.unwrap().contains("no endpoints"));
    s.put("/__kaanal/draft", json!({ "text": "nodes: [" }))
        .await;
    assert_eq!(s.get(URL).await.status(), 400);
}

#[tokio::test]
async fn an_endpoints_file_outside_the_project_is_refused() {
    let s = start_blank().await;
    let text = "version: 1\nnodes: {}\nendpoints: ../elsewhere.yaml\n";
    s.put("/__kaanal/draft", json!({ "text": text })).await;
    let r = s.get(URL).await;
    assert_eq!(r.status(), 400);
    assert!(
        r.text()
            .await
            .unwrap()
            .contains("inside the project folder")
    );
}

#[tokio::test]
async fn an_endpoints_file_inside_the_project_is_read() {
    let s = start_blank().await;
    std::fs::write(s.dir.join("eps.yaml"), "'GET /ping': {}\n").unwrap();
    let text = "version: 1\nnodes: {}\nendpoints: eps.yaml\n";
    s.put("/__kaanal/draft", json!({ "text": text })).await;
    let v: Value = s.get(URL).await.json().await.unwrap();
    assert_eq!(v["endpoints"], 1);
}

#[tokio::test]
async fn a_catch_all_stays_a_catch_all_in_the_spec() {
    let s = start_blank().await;
    let text = "version: 1\nnodes: {}\nendpoints:\n  'GET /files/{*rest}': {}\n";
    s.put("/__kaanal/draft", json!({ "text": text })).await;
    let v: Value = s.get(URL).await.json().await.unwrap();
    let model = kaanal_data::ApiModel::from_openapi(v["text"].as_str().unwrap()).unwrap();
    assert_eq!(model.endpoints[0].path, "/files/{*rest}");
}

#[tokio::test]
async fn a_bad_key_in_an_endpoints_file_is_not_echoed() {
    let s = start_blank().await;
    std::fs::write(s.dir.join("notes.yaml"), "hunter2_secret_key: {}\n").unwrap();
    let text = "version: 1\nnodes: {}\nendpoints: notes.yaml\n";
    s.put("/__kaanal/draft", json!({ "text": text })).await;
    let r = s.get(URL).await;
    assert_eq!(r.status(), 409);
    assert!(!r.text().await.unwrap().contains("hunter2"));
}
