//! `rename_node`: every reference in the draft changes together, or nothing does.

use crate::common::*;
use reqwest::Method;
use serde_json::{Value, json};

const DRAFT: &str = "/__kaanal/draft";

/// `db` is named in: its key, an edge, a node chain (bare and `call`), an endpoint chain, an
/// `error_map` key and an inline scenario (a node and an edge end).
const SYSTEM_WITH_REFS: &str = "version: 1
seed: 42
nodes:
  nginx: { type: gateway }
  # the application
  app:   { type: service, spec: openapi.yaml, chains: { nightly: [ db, { call: db, op: read } ] } }
  db:    { type: database, flavor: postgres }
edges:
  - { from: nginx, to: app }
  - { from: app, to: db }
endpoints:
  GET /users: { chain: [ db, { call: db } ], error_map: { db: 503 } }
scenarios:
  db-down: { chaos: [ { node: db, down: true }, { edge: { from: app, to: db }, latency_ms: 5 } ] }
";

async fn rename(s: &Srv, from: &str, to: &str) -> reqwest::Response {
    let op = json!({ "op": "rename_node", "from": from, "to": to });
    s.put(DRAFT, json!({ "patch": [op] })).await
}

async fn validate(s: &Srv) -> Value {
    let r = s.send(Method::POST, "/__kaanal/draft/validate", None).await;
    r.json().await.unwrap()
}

fn yaml(text: &str) -> serde_yaml_ng::Value {
    serde_yaml_ng::from_str(text).unwrap()
}

#[tokio::test]
async fn rename_node_rewrites_every_reference() {
    let s = start(SYSTEM_WITH_REFS).await;
    assert_eq!(
        validate(&s).await["ok"],
        true,
        "the starting point is a good system"
    );
    let r = rename(&s, "db", "store").await;
    assert_eq!(r.status(), 200);
    let v: Value = r.json().await.unwrap();
    let text = v["text"].as_str().unwrap();
    // one entry for each place that changed
    let mut touched: Vec<_> = v["touched"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t.as_str().unwrap())
        .collect();
    touched.sort_unstable();
    let mut want = vec![
        "nodes.db",
        "edges[1].to",
        "nodes.app.chains.nightly[0]",
        "nodes.app.chains.nightly[1].call",
        "endpoints.GET /users.chain[0]",
        "endpoints.GET /users.chain[1].call",
        "endpoints.GET /users.error_map.db",
        "scenarios.db-down.chaos[0].node",
        "scenarios.db-down.chaos[1].edge.to",
    ];
    want.sort_unstable();
    assert_eq!(touched, want, "{v}");
    // the text: no `db` node left anywhere, the comment and the scenario's own name stay
    let y = yaml(text);
    assert!(
        y["nodes"].get("db").is_none() && y["nodes"].get("store").is_some(),
        "{text}"
    );
    assert_eq!(y["edges"][1]["to"], "store");
    assert_eq!(y["nodes"]["app"]["chains"]["nightly"][0], "store");
    assert_eq!(y["nodes"]["app"]["chains"]["nightly"][1]["call"], "store");
    assert_eq!(y["endpoints"]["GET /users"]["chain"][0], "store");
    assert_eq!(y["endpoints"]["GET /users"]["chain"][1]["call"], "store");
    assert_eq!(y["endpoints"]["GET /users"]["error_map"]["store"], 503);
    assert_eq!(y["scenarios"]["db-down"]["chaos"][0]["node"], "store");
    assert_eq!(y["scenarios"]["db-down"]["chaos"][1]["edge"]["to"], "store");
    assert_eq!(
        y["edges"][1]["from"], "app",
        "an edge's other end is not touched"
    );
    assert!(text.contains("# the application"), "{text}");
    // and it is still a system the loader accepts
    assert_eq!(validate(&s).await["ok"], true, "{}", validate(&s).await);
}

#[tokio::test]
async fn rename_changes_only_the_bytes_of_the_names() {
    let s = start(SYSTEM_WITH_REFS).await;
    let v: Value = rename(&s, "db", "store").await.json().await.unwrap();
    let text = v["text"].as_str().unwrap();
    // the same text with each reference replaced: nothing else moved (no re-serialization)
    let want = SYSTEM_WITH_REFS
        .replace("  db:    {", "  store:    {")
        .replace("to: db }", "to: store }")
        .replace("[ db, {", "[ store, {")
        .replace("call: db", "call: store")
        .replace("error_map: { db:", "error_map: { store:")
        .replace("node: db,", "node: store,");
    assert_eq!(text, want);
}

#[tokio::test]
async fn rename_refuses_when_a_reference_is_in_an_alias() {
    let anchored = SYSTEM_WITH_REFS.replace("edges:", "edges: &es");
    let s = start(SYSTEM_WITH_REFS).await;
    assert_eq!(
        s.put(DRAFT, json!({ "text": anchored })).await.status(),
        200
    );
    let r = rename(&s, "db", "store").await;
    assert_eq!(r.status(), 400);
    let v: Value = r.json().await.unwrap();
    assert!(
        v["error"].as_str().unwrap().contains("nothing was changed"),
        "{v}"
    );
    let v: Value = s.get(DRAFT).await.json().await.unwrap();
    assert_eq!(v["text"], anchored, "not half renamed");
}

#[tokio::test]
async fn rename_refuses_when_a_reference_is_itself_anchored() {
    let anchored = SYSTEM_WITH_REFS.replace("{ from: app, to: db }", "{ from: app, to: &d db }");
    assert_ne!(anchored, SYSTEM_WITH_REFS);
    let s = start(SYSTEM_WITH_REFS).await;
    assert_eq!(
        s.put(DRAFT, json!({ "text": anchored })).await.status(),
        200
    );
    let r = rename(&s, "db", "store").await;
    assert_eq!(r.status(), 400, "an anchored reference must not be skipped");
    let v: Value = s.get(DRAFT).await.json().await.unwrap();
    assert_eq!(v["text"], anchored, "not half renamed");
}

#[tokio::test]
async fn rename_refuses_what_makes_no_sense() {
    let s = start(SYSTEM_WITH_REFS).await;
    for (from, to) in [("ghost", "x"), ("db", "app"), ("db", "db"), ("db", "")] {
        assert_eq!(rename(&s, from, to).await.status(), 400, "{from} -> {to}");
    }
    let r = s
        .put(
            DRAFT,
            json!({ "patch": [{ "op": "rename_node", "from": "db" }] }),
        )
        .await;
    assert_eq!(r.status(), 400);
    let v: Value = s.get(DRAFT).await.json().await.unwrap();
    assert_eq!(
        (&v["text"], &v["dirty"]),
        (&json!(SYSTEM_WITH_REFS), &json!(false))
    );
}

#[tokio::test]
async fn an_endpoints_file_is_reported_not_silently_left() {
    let text = "version: 1\nnodes:\n  gw: { type: gateway }\n  app: { type: service }\nedges:\n  - { from: gw, to: app }\nendpoints: kaanal.endpoints.yaml\n";
    let s = start_with(SYSTEM_WITH_REFS, Some(text)).await;
    let r = rename(&s, "app", "web").await;
    assert_eq!(r.status(), 200);
    let v: Value = r.json().await.unwrap();
    let notes = v["notes"].as_array().unwrap();
    assert_eq!(notes.len(), 1, "{v}");
    assert!(notes[0].as_str().unwrap().contains("'app' to 'web'"), "{v}");
    assert!(
        v["text"]
            .as_str()
            .unwrap()
            .contains("endpoints: kaanal.endpoints.yaml")
    );
}

#[tokio::test]
async fn a_renamed_node_loses_live_chaos_but_inline_scenarios_still_load() {
    let s = start(SYSTEM_WITH_REFS).await;
    // a scenario file is outside kaanal.yaml: the rename does not rewrite it, Apply flags it
    std::fs::create_dir_all(s.dir.join("scenarios")).unwrap();
    std::fs::write(
        s.dir.join("scenarios/f.yaml"),
        "chaos:\n  - { node: db, down: true }\n",
    )
    .unwrap();
    assert_eq!(
        s.put("/__kaanal/chaos/db", json!({ "latency_ms": 50 }))
            .await
            .status(),
        200
    );
    assert_eq!(rename(&s, "db", "store").await.status(), 200);
    let r = s.send(Method::POST, "/__kaanal/draft/apply", None).await;
    assert_eq!(r.status(), 200);
    let v: Value = r.json().await.unwrap();
    // a rename is a removal plus an addition (spec section 5): the chaos on `db` does not follow
    assert_eq!(v["kept"]["live_chaos"], 0, "{v}");
    let dropped = v["dropped"]["live_chaos"].as_array().unwrap();
    assert!(
        dropped.iter().any(|d| d.as_str().unwrap().contains("db")),
        "{v}"
    );
    // the scenario written in kaanal.yaml was rewritten with the rename, so it loads
    let sc: Value = s.get("/__kaanal/scenarios").await.json().await.unwrap();
    let names: Vec<_> = sc
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["name"].as_str().unwrap())
        .collect();
    assert!(names.contains(&"db-down"), "{sc}");
    // the file is flagged, not lost: it still names `db`
    let flagged = v["flagged"].as_array().unwrap();
    assert_eq!(flagged.len(), 1, "{v}");
    assert_eq!(flagged[0]["scenario"], "f");
}
