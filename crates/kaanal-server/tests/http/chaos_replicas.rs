//! Chaos on one replica through the control API: set, list, clear and the rejected shapes.

use crate::common::*;
use serde_json::{Value, json};

async fn put(s: &Srv, body: Value) -> reqwest::Response {
    s.auth(s.client.put(format!("{}/__kaanal/chaos", s.base)))
        .json(&body)
        .send()
        .await
        .unwrap()
}

async fn delete(s: &Srv, query: &str) -> reqwest::Response {
    s.auth(s.client.delete(format!("{}/__kaanal/chaos{query}", s.base)))
        .send()
        .await
        .unwrap()
}

async fn three_db_replicas() -> Srv {
    start_yaml(&yaml("", "", ", replicas: 3")).await
}

#[tokio::test]
async fn a_downed_replica_is_skipped_listed_and_cleared() {
    let s = three_db_replicas().await;
    assert_eq!(
        put(&s, json!({ "node": "db", "replica": 1, "down": true }))
            .await
            .status(),
        200
    );
    let all: Value = s.get_authed("/__kaanal/chaos").await.json().await.unwrap();
    assert_eq!(all["replicas"][0]["node"], "db");
    assert_eq!(all["replicas"][0]["replica"], 1);
    assert_eq!(all["replicas"][0]["down"], true);
    assert_eq!(all["effective"]["replicas"][0]["replica"], 1);
    // the other two replicas keep serving
    let st = codes(&s).await;
    assert!(st.iter().all(|c| *c == 200), "{st:?}");
    for r in [0, 2] {
        assert_eq!(
            put(&s, json!({ "node": "db", "replica": r, "down": true }))
                .await
                .status(),
            200
        );
    }
    assert!(
        codes(&s).await.iter().all(|c| *c == 503),
        "every replica down"
    );
    // clearing one brings it back
    assert_eq!(delete(&s, "?node=db&replica=1").await.status(), 204);
    assert!(codes(&s).await.iter().all(|c| *c == 200));
    assert_eq!(delete(&s, "").await.status(), 204);
    let mut all: Value = s.get_authed("/__kaanal/chaos").await.json().await.unwrap();
    all.as_object_mut().unwrap().remove("effective");
    assert_eq!(all, json!({ "nodes": {}, "endpoints": {}, "node_at": [] }));
}

#[tokio::test]
async fn bad_replica_requests_are_400() {
    let s = three_db_replicas().await;
    let msg = |r: reqwest::Response| async move {
        let status = r.status().as_u16();
        (
            status,
            r.json::<Value>().await.unwrap()["error"]
                .as_str()
                .unwrap()
                .to_string(),
        )
    };
    let (c, m) = msg(put(&s, json!({ "replica": 0, "down": true })).await).await;
    assert_eq!((c, m.as_str()), (400, "replica needs a node"));
    let (c, m) = msg(put(&s, json!({ "node": "db", "replica": 3, "down": true })).await).await;
    assert_eq!((c, m.as_str()), (400, "node 'db' has 3 replicas (0 to 2)"));
    let body = json!({ "node": "db", "replica": 0, "endpoint": "GET /users/{id}", "down": true });
    let (c, m) = msg(put(&s, body).await).await;
    assert_eq!(c, 400);
    assert!(m.contains("takes no endpoint"), "{m}");
    let (c, m) = msg(delete(&s, "?node=db&replica=9").await).await;
    assert_eq!((c, m.as_str()), (400, "node 'db' has 3 replicas (0 to 2)"));
}

/// 16 plain requests to a route that reaches `db`; unlike `statuses`, it sets no chaos itself.
async fn codes(s: &Srv) -> Vec<u16> {
    let mut out = Vec::new();
    for _ in 0..16 {
        out.push(s.get("/users/42").await.status().as_u16());
    }
    out
}

#[tokio::test]
async fn the_trace_and_the_log_name_the_replica() {
    let s = three_db_replicas().await;
    let mut seen = std::collections::BTreeSet::new();
    for _ in 0..12 {
        let r = s.get("/users/42").await;
        assert_eq!(r.status(), 200);
        let trace = trace_of(&r);
        let at = trace
            .find("db#")
            .unwrap_or_else(|| panic!("no db#N in {trace}"));
        seen.insert(trace[at + 3..at + 4].to_string());
        assert!(!trace.contains("app#"), "app has one replica: {trace}");
    }
    assert!(
        seen.len() > 1,
        "round-robin shows more than one replica: {seen:?}"
    );
    // the structured log carries `replica` on db's hops only
    let logs: Value = s.get_authed("/__kaanal/logs").await.json().await.unwrap();
    let entry = logs["entries"].as_array().unwrap().last().unwrap();
    for h in entry["hops"].as_array().unwrap() {
        let has = h.get("replica").is_some();
        assert_eq!(has, h["node"] == "db", "{h}");
    }
}

#[tokio::test]
async fn chaos_latency_on_a_replica_is_attributed_to_its_hop() {
    let s = three_db_replicas().await;
    for r in 0..3 {
        let body = json!({ "node": "db", "replica": r, "latency_ms": 40 });
        assert_eq!(put(&s, body).await.status(), 200);
    }
    let r = s.get("/users/42").await;
    let trace = trace_of(&r);
    assert!(trace.contains("db#"), "{trace}");
    let logs: Value = s.get_authed("/__kaanal/logs").await.json().await.unwrap();
    let hops = logs["entries"].as_array().unwrap().last().unwrap()["hops"].clone();
    let db_op = hops
        .as_array()
        .unwrap()
        .iter()
        .find(|h| h["node"] == "db" && h["kind"] == "op")
        .unwrap();
    assert_eq!(db_op["why"], "chaos +40ms", "{db_op}");
}

#[tokio::test]
async fn the_topology_reports_replicas_only_for_replicated_nodes() {
    let s = three_db_replicas().await;
    let t: Value = s
        .get_authed("/__kaanal/topology")
        .await
        .json()
        .await
        .unwrap();
    let find = |n: &str| {
        t["nodes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|x| x["name"] == n)
            .unwrap()
            .clone()
    };
    assert_eq!(find("db")["replicas"], 3);
    assert!(find("app").get("replicas").is_none());
}
