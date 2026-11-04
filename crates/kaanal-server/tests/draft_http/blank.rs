//! A blank start: no system until the first Apply; `add_node` gives a new node a default capacity.

use crate::common::*;
use reqwest::Method;
use serde_json::{Value, json};

const URL: &str = "/__kaanal/draft";

async fn patch(s: &Srv, ops: Value) -> Value {
    let r = s.put(URL, json!({ "patch": ops })).await;
    let status = r.status();
    let v: Value = r.json().await.unwrap();
    assert_eq!(status, 200, "{v}");
    v
}

async fn apply(s: &Srv) -> reqwest::Response {
    s.send(Method::POST, "/__kaanal/draft/apply", None).await
}

async fn validate(s: &Srv) -> Value {
    let r = s.send(Method::POST, "/__kaanal/draft/validate", None).await;
    r.json().await.unwrap()
}

#[tokio::test]
async fn a_blank_server_serves_only_the_control_api() {
    let s = start_blank().await;
    let r = s.get("/users/1").await;
    assert_eq!(r.status(), 404);
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["error"], "no system applied yet");
    assert_eq!(s.get("/__kaanal/health").await.status(), 200);
    assert_eq!(s.get("/__kaanal/ui").await.status(), 200);
    // what needs a system says so
    for path in ["/__kaanal/topology", "/__kaanal/chaos", "/__kaanal/pins"] {
        let r = s.get(path).await;
        let status = r.status();
        let v: Value = r.json().await.unwrap();
        assert!(status == 409 || status == 200, "{path}: {status} {v}");
    }
    assert_eq!(s.get("/__kaanal/topology").await.status(), 409);
    // the draft is the blank text, nothing applied
    let v: Value = s.get(URL).await.json().await.unwrap();
    assert_eq!(v["text"], kaanal_server::BLANK_TEXT);
    assert_eq!((&v["generation"], &v["dirty"]), (&json!(0), &json!(true)));
}

#[tokio::test]
async fn a_blank_draft_cannot_apply_without_an_entry_node() {
    let s = start_blank().await;
    let r = apply(&s).await;
    assert_eq!(r.status(), 400);
    let v: Value = r.json().await.unwrap();
    assert!(v["diagnostics"][0]["message"].is_string(), "{v}");
    assert_eq!(s.get("/users/1").await.status(), 404, "still no system");
    let v: Value = s.get(URL).await.json().await.unwrap();
    assert_eq!(v["generation"], 0);
}

#[tokio::test]
async fn the_first_apply_creates_the_first_generation() {
    let s = start_blank().await;
    patch(
        &s,
        json!([
            { "op": "add_node", "name": "nginx", "kind": "gateway" },
            { "op": "add_node", "name": "app", "kind": "service" },
            { "op": "insert", "path": ["nodes", "app", "spec"], "value": "openapi.yaml" },
            { "op": "insert", "path": ["edges"], "value": [{ "from": "nginx", "to": "app", "net": "1ms" }] },
        ]),
    )
    .await;
    let r = apply(&s).await;
    assert_eq!(r.status(), 200, "{:?}", r.text().await);
    let v: Value = s.get(URL).await.json().await.unwrap();
    assert_eq!((&v["generation"], &v["dirty"]), (&json!(1), &json!(false)));
    assert_eq!(s.get("/__kaanal/topology").await.status(), 200);
    let r = s.get("/users").await;
    assert_ne!(r.status(), 404, "the mock now serves the spec's routes");
    // and a second Apply is generation 2, as after any start
    let r = apply(&s).await;
    assert_eq!(r.status(), 200);
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["generation"], 2);
}

#[tokio::test]
async fn add_node_gets_a_default_capacity_and_validates() {
    // (kind, what the new node's line must contain)
    let table = [
        ("gateway", "timeout: 30s"),
        ("gateway", "queue: 100"),
        ("service", "queue: 100"),
        ("monolith", "proc: 40ms"),
        ("database", "size: 20"),
        ("cache", "flavor: redis"),
        ("external-api", "proc: 100ms"),
        ("auth", "proc: 10ms"),
        ("storage", "throughput: 80MB/s"),
    ];
    for (kind, has) in table {
        let s = start_blank().await;
        let mut ops = vec![json!({ "op": "add_node", "name": "nginx", "kind": "gateway" })];
        if kind != "gateway" {
            ops.push(json!({ "op": "add_node", "name": "app", "kind": "service" }));
            ops.push(json!({ "op": "add_node", "name": "x", "kind": kind }));
            ops.push(json!({ "op": "insert", "path": ["edges"], "value": [
                { "from": "nginx", "to": "app" }, { "from": "app", "to": "x" },
            ] }));
        }
        let v = patch(&s, json!(ops)).await;
        let text = v["text"].as_str().unwrap();
        let line = text
            .lines()
            .find(|l| l.contains(&format!("type: {kind}")))
            .unwrap_or_else(|| panic!("{kind}: no line in {text}"));
        assert!(line.contains(has), "{kind}: {line}");
        let v = validate(&s).await;
        assert_eq!(v["ok"], true, "{kind}: {v}");
    }
}

#[tokio::test]
async fn add_node_refuses_what_it_cannot_do() {
    let s = start_blank().await;
    for body in [
        json!([{ "op": "add_node", "name": "a", "kind": "toaster" }]),
        json!([{ "op": "add_node", "kind": "service" }]),
        json!([{ "op": "add_node", "name": "a" }]),
        json!([
            { "op": "add_node", "name": "a", "kind": "service" },
            { "op": "add_node", "name": "a", "kind": "service" },
        ]),
    ] {
        let r = s.put(URL, json!({ "patch": body })).await;
        assert_eq!(r.status(), 400, "{body}");
    }
    let v: Value = s.get(URL).await.json().await.unwrap();
    assert_eq!(v["text"], kaanal_server::BLANK_TEXT, "nothing changed");
}

#[tokio::test]
async fn a_blank_start_saves_a_new_file_and_refuses_if_one_appeared() {
    let s = start_blank().await;
    let file = s.dir.join("kaanal.yaml");
    assert!(!file.exists());
    patch(
        &s,
        json!([{ "op": "add_node", "name": "nginx", "kind": "gateway" }]),
    )
    .await;
    let r = s.send(Method::POST, "/__kaanal/draft/save", None).await;
    assert_eq!(r.status(), 200);
    assert!(std::fs::read_to_string(&file).unwrap().contains("nginx"));
    // a second blank server over a folder whose kaanal.yaml appears after the start
    let s = start_blank().await;
    std::fs::write(s.dir.join("kaanal.yaml"), "version: 1\n").unwrap();
    let r = s.send(Method::POST, "/__kaanal/draft/save", None).await;
    assert_eq!(
        r.status(),
        409,
        "never overwrite a file the draft was not started from"
    );
}

#[tokio::test]
async fn saved_is_false_on_a_blank_start_until_first_save() {
    let s = start_blank().await;
    let v: Value = s.get(URL).await.json().await.unwrap();
    assert_eq!(v["saved"], false, "nothing is on disk yet");
    let r = s.send(Method::POST, "/__kaanal/draft/save", None).await;
    assert_eq!(r.status(), 200, "{}", r.text().await.unwrap());
    let v: Value = s.get(URL).await.json().await.unwrap();
    assert_eq!(v["saved"], true);
}
