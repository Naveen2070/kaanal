//! What an Apply keeps, by name: live chaos, leases, active scenarios, pins; and what it flags.

use crate::common::*;
use reqwest::Method;
use serde_json::{Value, json};
use std::fmt::Write as _;
use std::time::{Duration, Instant};

const DRAFT: &str = "/__kaanal/draft";

async fn put_draft(s: &Srv, text: &str) {
    assert_eq!(s.put(DRAFT, json!({ "text": text })).await.status(), 200);
}

async fn apply(s: &Srv) -> Value {
    let r = s.send(Method::POST, "/__kaanal/draft/apply", None).await;
    assert_eq!(r.status(), 200);
    r.json().await.unwrap()
}

async fn chaos(s: &Srv) -> Value {
    s.get("/__kaanal/chaos").await.json().await.unwrap()
}

/// The `system()` nodes in the given order, so a node's id changes with the order.
fn ordered(order: &[&str]) -> String {
    let defs = [
        ("nginx", "  nginx: { type: gateway, proc: 1ms }\n"),
        (
            "app",
            "  app:   { type: service, spec: openapi.yaml, proc: 1ms }\n",
        ),
        (
            "db",
            "  db:    { type: database, ops: { read: 1ms, write: 1ms, scan: 1ms } }\n",
        ),
    ];
    let mut out = String::from("version: 1\nseed: 42\nnodes:\n");
    for name in order {
        out += defs.iter().find(|(n, _)| n == name).unwrap().1;
    }
    out += "edges:\n  - { from: nginx, to: app, net: 1ms }\n";
    if order.contains(&"db") {
        out += "  - { from: app, to: db, net: 1ms }\n";
    }
    out
}

#[tokio::test]
async fn chaos_survives_when_the_node_order_changes() {
    let s = start(&ordered(&["nginx", "app", "db"])).await;
    let r = s
        .put("/__kaanal/chaos/db", json!({ "latency_ms": 120 }))
        .await;
    assert_eq!(r.status(), 200);
    // db was node 2; in the new draft it is node 0
    put_draft(&s, &ordered(&["db", "app", "nginx"])).await;
    let v = apply(&s).await;
    assert_eq!(v["kept"]["live_chaos"], 1, "{v}");
    let c = chaos(&s).await;
    assert_eq!(
        c["nodes"]["db"]["latency_ms"], 120,
        "it still points at db: {c}"
    );
    assert_eq!(
        c["nodes"].as_object().unwrap().len(),
        1,
        "and at nothing else: {c}"
    );
    let began = Instant::now();
    let status = s
        .client
        .get(format!("{}/users/1", s.base))
        .send()
        .await
        .unwrap()
        .status();
    assert_eq!(status, 200);
    assert!(
        began.elapsed() >= Duration::from_millis(100),
        "the chaos really hits db in the new system: {:?}",
        began.elapsed()
    );
}

#[tokio::test]
async fn chaos_on_a_removed_node_is_dropped_and_reported() {
    let s = start(&ordered(&["nginx", "app", "db"])).await;
    s.put("/__kaanal/chaos/db", json!({ "latency_ms": 50 }))
        .await;
    put_draft(&s, &ordered(&["nginx", "app"])).await;
    let v = apply(&s).await;
    assert_eq!(v["dropped"]["live_chaos"], json!(["node 'db'"]), "{v}");
    assert_eq!(v["kept"]["live_chaos"], 0);
    assert_eq!(chaos(&s).await["nodes"], json!({}));
}

#[tokio::test]
async fn a_replica_beyond_the_new_count_is_dropped() {
    let three = ordered(&["nginx", "app", "db"]).replace("ops: {", "replicas: 3, ops: {");
    let s = start(&three).await;
    let body = json!({ "node": "db", "replica": 2, "down": true });
    assert_eq!(s.put("/__kaanal/chaos", body).await.status(), 200);
    // same count: kept
    let v = apply(&s).await;
    assert_eq!(v["kept"]["live_chaos"], 1, "{v}");
    let n = |c: &Value| c["replicas"].as_array().map_or(0, Vec::len);
    assert_eq!(n(&chaos(&s).await), 1);
    // two replicas now: replica 2 does not exist
    put_draft(&s, &three.replace("replicas: 3", "replicas: 2")).await;
    let v = apply(&s).await;
    assert_eq!(
        v["dropped"]["live_chaos"],
        json!(["node 'db' replica 2"]),
        "{v}"
    );
    assert_eq!(n(&chaos(&s).await), 0);
}

#[tokio::test]
async fn a_leased_unit_keeps_its_lease_or_loses_it_with_its_key() {
    let full = ordered(&["nginx", "app", "db"]);
    let s = start(&full).await;
    let r = s
        .put(
            "/__kaanal/chaos/db",
            json!({ "latency_ms": 50, "ttl": "60s" }),
        )
        .await;
    assert_eq!(r.status(), 200);
    apply(&s).await;
    let c = chaos(&s).await;
    let left = c["nodes"]["db"]["expires_in_ms"]["latency"].as_u64();
    assert!(
        left.is_some_and(|ms| ms > 0 && ms <= 60_000),
        "the lease went with it: {c}"
    );
    put_draft(&s, &ordered(&["nginx", "app"])).await;
    apply(&s).await;
    assert_eq!(chaos(&s).await["nodes"], json!({}), "no key, no lease");
}

#[tokio::test]
async fn an_active_scenario_survives_unless_the_new_draft_lacks_it() {
    let with = |names: &[&str]| {
        let mut y = ordered(&["nginx", "app", "db"]);
        y += "scenarios:\n";
        for n in names {
            let _ = writeln!(y, "  {n}: {{ chaos: [ {{ node: db, latency_ms: 5 }} ] }}");
        }
        y
    };
    let s = start(&with(&["s1", "s2"])).await;
    for n in ["s1", "s2"] {
        let r = s
            .send(
                Method::POST,
                &format!("/__kaanal/scenarios/{n}/activate"),
                None,
            )
            .await;
        assert_eq!(r.status(), 200);
    }
    put_draft(&s, &with(&["s1"])).await;
    let v = apply(&s).await;
    assert_eq!(v["kept"]["active_scenarios"], json!(["s1"]), "{v}");
    assert_eq!(v["dropped"]["active_scenarios"], json!(["s2"]));
    let a: Value = s
        .get("/__kaanal/scenarios/active")
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(a["active"], json!(["s1"]));
}

#[tokio::test]
async fn a_pin_survives_while_its_endpoint_does() {
    let full = ordered(&["nginx", "app", "db"]);
    let s = start(&full).await;
    let pin = json!({ "endpoint": "GET /users/{id}", "state": "loading" });
    assert_eq!(s.put("/__kaanal/pins", pin).await.status(), 200);
    let v = apply(&s).await;
    assert_eq!(v["kept"]["pins"], 1, "{v}");
    let pins: Value = s.get("/__kaanal/pins").await.json().await.unwrap();
    assert_eq!(pins["live"].as_array().unwrap().len(), 1, "{pins}");
    // the route is gone when `app` has no spec
    put_draft(&s, &full.replace("spec: openapi.yaml, ", "")).await;
    let v = apply(&s).await;
    assert_eq!(v["dropped"]["pins"], json!(["GET /users/{id}"]), "{v}");
}

#[tokio::test]
async fn a_scenario_file_naming_a_removed_node_is_flagged_and_apply_succeeds() {
    let s = start(&ordered(&["nginx", "app", "db"])).await;
    let dir = s.dir.join("scenarios");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("f.yaml"), "chaos:\n  - { node: db, down: true }\n").unwrap();
    put_draft(&s, &ordered(&["nginx", "app"])).await;
    // validate reports it too, and says the draft is fine
    let r = s.send(Method::POST, "/__kaanal/draft/validate", None).await;
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["ok"], true, "{v}");
    assert_eq!(v["flagged"][0]["scenario"], "f", "{v}");
    let v = apply(&s).await;
    let f = &v["flagged"][0];
    assert_eq!(f["scenario"], "f", "{v}");
    assert!(f["file"].as_str().unwrap().ends_with("f.yaml"));
    assert!(f["message"].as_str().unwrap().contains("db"), "{f}");
    let sc: Value = s.get("/__kaanal/scenarios").await.json().await.unwrap();
    assert!(
        sc.as_array().unwrap().iter().all(|e| e["name"] != "f"),
        "the flagged file is left out of the new system: {sc}"
    );
}
