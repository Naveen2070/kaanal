//! A parameter edit goes straight into the running engine while a page follows the live run
//! (step 8L): it changes what real mock requests see, structure edits wait for Apply, and the
//! file changes only on Save.

use crate::common::*;
use reqwest::Method;
use serde_json::{Value, json};
use std::time::{Duration, Instant};

const LIVE: &str = "version: 1
seed: 42
nodes:
  nginx: { type: gateway, proc: 1ms }
  app: { type: service, spec: openapi.yaml, proc: 40ms }
  db: { type: database, ops: { read: 1ms, write: 1ms, scan: 1ms } }
edges:
  - { from: nginx, to: app }
  - { from: app, to: db }
traffic:
  peak:
    rps: 5
    journeys:
      - { endpoint: 'GET /users', weight: 1 }
";

/// A page following the live run: the stream stays open for as long as this is held.
async fn follow(s: &Srv) -> reqwest::Response {
    let r = s
        .client
        .get(format!(
            "{}/__kaanal/live?fresh=1&token={}",
            s.base, s.token
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    r
}

async fn patch(s: &Srv, ops: Value) -> Value {
    s.put("/__kaanal/draft", json!({ "patch": ops }))
        .await
        .json()
        .await
        .unwrap()
}

/// How long two simultaneous `GET /users` take, the slower of them.
async fn two_at_once(s: &Srv) -> Duration {
    let start = Instant::now();
    let call = || async {
        let r = s
            .client
            .get(format!("{}/users", s.base))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        start.elapsed()
    };
    let (a, b) = tokio::join!(call(), call());
    a.max(b)
}

#[tokio::test]
async fn editing_cores_over_http_changes_a_running_mocks_latency() {
    let s = start(LIVE).await;
    let _page = follow(&s).await;
    let before = two_at_once(&s).await;
    assert!(before < Duration::from_millis(70), "{before:?}");
    let v = patch(
        &s,
        json!([{ "op": "insert", "path": ["nodes", "app", "cpu"], "value": { "cores": 1 } }]),
    )
    .await;
    assert_eq!(v["live"]["live"], "applied", "{v}");
    assert_eq!(v["live"]["changed"], json!(["app"]));
    // two requests share one core now: each takes about twice its 40 ms of work
    let after = two_at_once(&s).await;
    assert!(after >= Duration::from_millis(75), "{after:?}");
}

#[tokio::test]
async fn a_mixed_patch_applies_its_parameter_part_and_leaves_the_structure_part() {
    let s = start(LIVE).await;
    let _page = follow(&s).await;
    let v = patch(
        &s,
        json!([
            { "op": "set", "path": ["nodes", "app", "proc"], "value": "90ms" },
            { "op": "insert", "path": ["nodes", "extra"], "value": { "type": "service", "proc": "1ms" } }
        ]),
    )
    .await;
    assert_eq!(v["live"]["live"], "applied", "{v}");
    assert_eq!(v["live"]["structure_pending"], true);
    // the new component is in the draft but not in the running system
    assert!(v["text"].as_str().unwrap().contains("extra"));
    let topo: Value = s.get("/__kaanal/topology").await.json().await.unwrap();
    let names: Vec<&str> = topo["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["name"].as_str().unwrap())
        .collect();
    assert!(!names.contains(&"extra"), "{names:?}");
    // and the parameter part runs: a request takes the new 90 ms
    let start_t = Instant::now();
    let r = s
        .client
        .get(format!("{}/users", s.base))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    assert!(start_t.elapsed() >= Duration::from_millis(85));
}

#[tokio::test]
async fn a_pending_structure_edit_does_not_block_a_parameter_edit_to_an_existing_node() {
    let s = start(LIVE).await;
    let _page = follow(&s).await;
    let first = patch(
        &s,
        json!([{ "op": "insert", "path": ["nodes", "extra"], "value": { "type": "service" } }]),
    )
    .await;
    assert_eq!(first["live"]["live"], "unchanged", "{first}");
    assert_eq!(first["live"]["structure_pending"], true);
    let second = patch(
        &s,
        json!([{ "op": "set", "path": ["nodes", "app", "proc"], "value": "60ms" }]),
    )
    .await;
    assert_eq!(second["live"]["live"], "applied", "{second}");
    assert_eq!(second["live"]["changed"], json!(["app"]));
}

#[tokio::test]
async fn a_parameter_edit_to_a_draft_only_node_waits_for_apply() {
    let s = start(LIVE).await;
    let _page = follow(&s).await;
    patch(
        &s,
        json!([{ "op": "insert", "path": ["nodes", "extra"], "value": { "type": "service" } }]),
    )
    .await;
    let v = patch(
        &s,
        json!([{ "op": "insert", "path": ["nodes", "extra", "proc"], "value": "5ms" }]),
    )
    .await;
    assert_eq!(v["live"]["live"], "unchanged", "{v}");
    assert_eq!(v["live"]["structure_pending"], true);
}

#[tokio::test]
async fn the_file_on_disk_changes_only_on_save() {
    let s = start(LIVE).await;
    let _page = follow(&s).await;
    let file = s.dir.join("kaanal.yaml");
    patch(
        &s,
        json!([{ "op": "set", "path": ["nodes", "app", "proc"], "value": "55ms" }]),
    )
    .await;
    assert_eq!(
        std::fs::read_to_string(&file).unwrap(),
        LIVE,
        "an edit never writes the file"
    );
    let saved = s
        .send(Method::POST, "/__kaanal/draft/save", Some(json!({})))
        .await;
    assert_eq!(saved.status(), 200);
    assert!(std::fs::read_to_string(&file).unwrap().contains("55ms"));
}

#[tokio::test]
async fn a_refused_value_never_reaches_the_engine() {
    let s = start(LIVE).await;
    let _page = follow(&s).await;
    let v = patch(
        &s,
        json!([{ "op": "insert", "path": ["nodes", "app", "cpu"], "value": { "cores": 0 } }]),
    )
    .await;
    assert_eq!(v["live"]["live"], "invalid", "{v}");
    assert!(
        v["live"]["message"].as_str().unwrap().contains("cores"),
        "{v}"
    );
    // the running system is as it was: two requests do not share a core
    assert!(two_at_once(&s).await < Duration::from_millis(70));
}

#[tokio::test]
async fn without_a_page_connected_a_draft_edit_waits_for_apply() {
    let s = start(LIVE).await;
    let v = patch(
        &s,
        json!([{ "op": "set", "path": ["nodes", "app", "proc"], "value": "70ms" }]),
    )
    .await;
    assert_eq!(v["live"]["live"], "none", "{v}");
}

#[tokio::test]
async fn a_parameter_edit_that_is_running_leaves_the_draft_clean_and_pending_structure_does_not() {
    let s = start(LIVE).await;
    let _page = follow(&s).await;
    let v = patch(
        &s,
        json!([{ "op": "set", "path": ["nodes", "app", "proc"], "value": "55ms" }]),
    )
    .await;
    assert_eq!(v["live"]["live"], "applied", "{v}");
    assert_eq!(
        v["dirty"], false,
        "it is running, so nothing waits for Apply: {v}"
    );
    let v = patch(
        &s,
        json!([{ "op": "insert", "path": ["nodes", "extra"], "value": { "type": "service" } }]),
    )
    .await;
    assert_eq!(
        v["dirty"], true,
        "a pending structure edit waits for Apply: {v}"
    );
    // a further parameter edit applies and the structure still waits
    let v = patch(
        &s,
        json!([{ "op": "set", "path": ["nodes", "app", "proc"], "value": "65ms" }]),
    )
    .await;
    assert_eq!(v["live"]["structure_pending"], true);
    assert_eq!(v["dirty"], true);
}

/// Review finding 1: an Apply restarts the live session, and the new session must run on what was
/// just applied, so a parameter edit after it is a plain edit and not a structure mismatch.
#[tokio::test]
async fn a_parameter_edit_after_an_apply_that_added_a_component_reaches_the_new_engine() {
    let s = start(LIVE).await;
    let _page = follow(&s).await;
    patch(
        &s,
        json!([{ "op": "insert", "path": ["nodes", "extra"], "value": { "type": "service" } }]),
    )
    .await;
    let r = s.send(Method::POST, "/__kaanal/draft/apply", None).await;
    assert_eq!(r.status(), 200);
    let v = patch(
        &s,
        json!([{ "op": "set", "path": ["nodes", "app", "proc"], "value": "90ms" }]),
    )
    .await;
    assert_eq!(v["live"]["live"], "applied", "{v}");
    assert_eq!(v["live"]["structure_pending"], false, "{v}");
    let start_t = Instant::now();
    let r = s
        .client
        .get(format!("{}/users", s.base))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    assert!(start_t.elapsed() >= Duration::from_millis(85));
}

/// Review finding 2: a draft that only reorders the components is a structure edit (node numbers
/// change), so it waits for Apply and the running system is left alone.
#[tokio::test]
async fn reordering_components_waits_for_apply_and_leaves_the_engine_alone() {
    let s = start(LIVE).await;
    let _page = follow(&s).await;
    let reordered = LIVE.replace(
        "  nginx: { type: gateway, proc: 1ms }\n  app: { type: service, spec: openapi.yaml, proc: 40ms }\n",
        "  app: { type: service, spec: openapi.yaml, proc: 40ms }\n  nginx: { type: gateway, proc: 1ms }\n",
    );
    assert_ne!(reordered, LIVE);
    let v = s
        .put("/__kaanal/draft", json!({ "text": reordered }))
        .await
        .json::<Value>()
        .await
        .unwrap();
    assert_eq!(v["live"]["structure_pending"], true, "{v}");
    assert_ne!(v["live"]["live"], "invalid", "{v}");
}

#[tokio::test]
async fn live_reset_rebuilds_the_applied_system_and_leaves_the_draft_alone() {
    const DRAFT: &str = "/__kaanal/draft";
    let s = start(&system("1ms")).await;
    let before: Value = s.get(DRAFT).await.json().await.unwrap();
    let g0 = before["generation"].as_u64().unwrap();
    // an unapplied edit: a reset must not apply it
    let edited = system("2ms");
    assert_eq!(s.put(DRAFT, json!({ "text": edited })).await.status(), 200);
    let r = s
        .send(reqwest::Method::POST, "/__kaanal/live/reset", None)
        .await;
    assert_eq!(r.status(), 200);
    let body: Value = r.json().await.unwrap();
    assert_eq!(body["generation"].as_u64().unwrap(), g0 + 1);
    let after: Value = s.get(DRAFT).await.json().await.unwrap();
    assert_eq!(after["text"], json!(edited), "the draft is untouched");
    assert_eq!(after["dirty"], json!(true), "and still not applied");
}
