//! Chaos on a link through the control API: set, list, clear and the rejected shapes.

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

/// The standard topology with a 1 MB/s link on `app -> db` (`nginx -> app` has none) and a
/// 300 ms timeout on `db`, so a partition does not wait the full minute.
async fn linked() -> Srv {
    start_yaml(&yaml("", "", ", timeout: 300ms").replace(
        "- { from: app, to: db, net: 1ms }",
        "- { from: app, to: db, net: 1ms, bandwidth: 1000000 }",
    ))
    .await
}

async fn error_of(r: reqwest::Response) -> (u16, String) {
    let status = r.status().as_u16();
    let v: Value = r.json().await.unwrap();
    (status, v["error"].as_str().unwrap_or_default().to_string())
}

#[tokio::test]
async fn an_edge_entry_is_set_listed_and_cleared() {
    let s = linked().await;
    let r = put(
        &s,
        json!({ "edge": { "from": "app", "to": "db" }, "latency_ms": 20, "jitter_ms": 5, "bandwidth_cap": 500_000 }),
    )
    .await;
    assert_eq!(r.status(), 200);
    let all: Value = s.get_authed("/__kaanal/chaos").await.json().await.unwrap();
    let e = &all["edges"][0];
    assert_eq!(
        (e["edge"]["from"].as_str(), e["edge"]["to"].as_str()),
        (Some("app"), Some("db"))
    );
    assert_eq!(e["latency_ms"], 20);
    assert_eq!(e["jitter_ms"], 5);
    assert_eq!(e["bandwidth_cap"], 500_000);
    let eff = &all["effective"]["edges"][0];
    assert_eq!(
        (eff["jitter_ms"].as_u64(), eff["bandwidth_cap"].as_u64()),
        (Some(5), Some(500_000))
    );
    assert_eq!(delete(&s, "?edge_from=app&edge_to=db").await.status(), 204);
    let all: Value = s.get_authed("/__kaanal/chaos").await.json().await.unwrap();
    assert!(all.get("edges").is_none(), "{all}");
}

#[tokio::test]
async fn bad_edge_requests_are_rejected() {
    let s = linked().await;
    let edge = |from: &str, to: &str| json!({ "from": from, "to": to });
    let (c, m) = error_of(
        put(
            &s,
            json!({ "edge": edge("app", "db"), "error_rate": 0.5, "error_kind": "status" }),
        )
        .await,
    )
    .await;
    assert!(c == 400 && m.contains("reset or hang"), "{c} {m}");
    let (c, m) = error_of(
        put(
            &s,
            json!({ "edge": edge("nginx", "app"), "bandwidth_cap": 100 }),
        )
        .await,
    )
    .await;
    assert!(c == 400 && m.contains("no bandwidth to cap"), "{c} {m}");
    let (c, m) =
        error_of(put(&s, json!({ "edge": edge("nginx", "db"), "down": true })).await).await;
    assert!(c == 404 && m.contains("no edge 'nginx' -> 'db'"), "{c} {m}");
    let (c, m) = error_of(
        put(
            &s,
            json!({ "edge": edge("app", "db"), "node": "db", "down": true }),
        )
        .await,
    )
    .await;
    assert!(c == 400 && m.contains("takes no node"), "{c} {m}");
    let (c, m) = error_of(put(&s, json!({ "node": "db", "jitter_ms": 5 })).await).await;
    assert!(c == 400 && m.contains("for an edge key"), "{c} {m}");
    let (c, m) = error_of(delete(&s, "?edge_from=app").await).await;
    assert!(
        c == 400 && m.contains("both edge_from and edge_to"),
        "{c} {m}"
    );
}

async fn last_hops(s: &Srv) -> Vec<Value> {
    let logs: Value = s.get_authed("/__kaanal/logs").await.json().await.unwrap();
    let e = logs["entries"].as_array().unwrap().last().unwrap().clone();
    e["hops"].as_array().unwrap().clone()
}

async fn last_entry(s: &Srv) -> Value {
    let logs: Value = s.get_authed("/__kaanal/logs").await.json().await.unwrap();
    logs["entries"].as_array().unwrap().last().unwrap().clone()
}

#[tokio::test]
async fn a_link_chaos_hop_is_flagged_in_the_trace() {
    let s = linked().await;
    let body = json!({ "edge": { "from": "app", "to": "db" }, "latency_ms": 30, "jitter_ms": 2 });
    assert_eq!(put(&s, body).await.status(), 200);
    s.get("/users/42").await;
    let e = last_entry(&s).await;
    let net = e["hops"]
        .as_array()
        .unwrap()
        .iter()
        .find(|h| h["node"] == "db" && h["kind"] == "net")
        .unwrap()
        .clone();
    let why = net["why"].as_str().unwrap_or_default();
    assert!(
        why.contains("chaos +30ms") && why.contains("link jitter ±2ms"),
        "{net}"
    );
    assert!(e["flags"].as_array().unwrap().contains(&json!("DI")), "{e}");
    // the other edge's hop is untouched
    let app_net = last_hops(&s)
        .await
        .into_iter()
        .find(|h| h["node"] == "app" && h["kind"] == "net")
        .unwrap();
    assert!(app_net["why"].is_null(), "{app_net}");
}

#[tokio::test]
async fn a_link_cap_and_a_partition_are_flagged() {
    let s = linked().await;
    let cap = json!({ "edge": { "from": "app", "to": "db" }, "bandwidth_cap": 500_000 });
    assert_eq!(put(&s, cap).await.status(), 200);
    // a request body makes the transfer visible
    let r = s
        .auth(s.client.get(format!("{}/users/42", s.base)))
        .header("content-length", "5000")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let link = last_hops(&s)
        .await
        .into_iter()
        .find(|h| h["kind"] == "link")
        .unwrap();
    assert_eq!(link["why"], "link capped at 500000 B/s", "{link}");
    // a partition is injected failure (FI) with a 504
    assert_eq!(delete(&s, "").await.status(), 204);
    let down = json!({ "edge": { "from": "app", "to": "db" }, "down": true });
    assert_eq!(put(&s, down).await.status(), 200);
    let r = s.get("/users/42").await;
    assert_eq!(r.status(), 504);
    let e = last_entry(&s).await;
    assert!(e["flags"].as_array().unwrap().contains(&json!("FI")), "{e}");
}

#[tokio::test]
async fn a_cap_takes_a_unit_like_the_yaml_and_is_rejected_like_it() {
    let s = linked().await;
    let edge = json!({ "from": "app", "to": "db" });
    // `500KB/s` is 512000 bytes a second, under the edge's 1000000
    let r = put(&s, json!({ "edge": edge, "bandwidth_cap": "500KB/s" })).await;
    assert_eq!(r.status(), 200);
    let all: Value = s.get_authed("/__kaanal/chaos").await.json().await.unwrap();
    assert_eq!(all["edges"][0]["bandwidth_cap"], 512_000, "{all}");
    // above the edge's own bandwidth: the same rule as for a number
    let (c, m) = error_of(put(&s, json!({ "edge": edge, "bandwidth_cap": "2MB/s" })).await).await;
    assert!(
        c == 400 && m.contains("above the edge's bandwidth"),
        "{c} {m}"
    );
    // a unit that does not exist, and zero after the conversion
    let (c, m) = error_of(put(&s, json!({ "edge": edge, "bandwidth_cap": "5XB" })).await).await;
    assert!(
        c == 422 && m.contains("bandwidth_cap: unknown unit in `5XB`"),
        "{c} {m}"
    );
    let (c, m) = error_of(put(&s, json!({ "edge": edge, "bandwidth_cap": "0KB" })).await).await;
    assert!(
        c == 400 && m.contains("bandwidth_cap must be at least 1"),
        "{c} {m}"
    );
    let (c, m) = error_of(put(&s, json!({ "edge": edge, "bandwidth_cap": -5 })).await).await;
    assert!(
        c == 422 && m.contains("bandwidth_cap: -5 is negative"),
        "{c} {m}"
    );
}
