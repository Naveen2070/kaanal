//! Topology, the event stream and the embedded UI page.

use crate::common::*;
use serde_json::{Value, json};

#[tokio::test]
async fn topology_lists_nodes_edges_routes() {
    let s = start().await;
    let t: Value = s
        .get_authed("/__kaanal/topology")
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(
        t["nodes"][0],
        json!({ "name": "nginx", "type": "gateway", "flavor": "nginx" })
    );
    assert_eq!(t["edges"][1], json!({ "from": "app", "to": "db" }));
    let routes: Vec<&str> = t["routes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r.as_str().unwrap())
        .collect();
    assert_eq!(
        routes,
        vec![
            "GET /health",
            "GET /users",
            "GET /users/{id}",
            "POST /orders"
        ]
    );
}

#[tokio::test]
async fn events_stream_reports_requests() {
    let s = start().await;
    let mut stream = s
        .client
        .get(format!("{}/__kaanal/events?token={}", s.base, s.token))
        .send()
        .await
        .unwrap();
    assert_eq!(stream.status(), 200);
    s.get("/users/7").await;
    let mut seen = String::new();
    let found = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while let Some(chunk) = stream.chunk().await.unwrap() {
            seen.push_str(&String::from_utf8_lossy(&chunk));
            if seen.contains("event: request") && seen.contains("/users/7") {
                return true;
            }
        }
        false
    })
    .await
    .unwrap_or(false);
    assert!(found, "stream so far: {seen}");
    assert!(
        seen.contains("\"nodes\":[\"nginx\",\"app\",\"db\"]"),
        "{seen}"
    );
    // `ms` is measured wall time, so it is at least the 5 simulated 1ms hops
    let data = seen.lines().find(|l| l.starts_with("data:")).unwrap();
    let ev: Value = serde_json::from_str(data.trim_start_matches("data:").trim()).unwrap();
    assert!(ev["ms"].as_u64().unwrap() >= 5, "{ev}");
    assert_eq!(ev["endpoint"], "GET /users/{id}", "{ev}");
}

#[tokio::test]
async fn events_carry_the_failure_kind_for_non_status_failures() {
    let s = start().await;
    let mut stream = s
        .client
        .get(format!("{}/__kaanal/events?token={}", s.base, s.token))
        .send()
        .await
        .unwrap();
    s.put_chaos(
        "app",
        json!({ "error_rate": 1.0, "error_kind": "malformed" }),
    )
    .await;
    s.get("/users/7").await;
    s.put_chaos("app", json!({ "error_kind": "status" })).await;
    s.get("/users/8").await;
    let mut seen = String::new();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while let Some(chunk) = stream.chunk().await.unwrap() {
            seen.push_str(&String::from_utf8_lossy(&chunk));
            if seen.contains("/users/8") {
                break;
            }
        }
    })
    .await
    .unwrap();
    let evs: Vec<Value> = seen
        .lines()
        .filter_map(|l| l.strip_prefix("data:"))
        .map(|d| serde_json::from_str(d.trim()).unwrap())
        .collect();
    assert_eq!(evs[0]["kind"], "malformed", "{seen}");
    assert_eq!(evs[0]["failed_node"], "app");
    assert!(
        evs[1]["kind"].is_null(),
        "a status failure has no kind: {seen}"
    );
}

#[tokio::test]
async fn events_without_token_is_401() {
    let s = start().await;
    assert_eq!(s.get("/__kaanal/events").await.status(), 401);
}

#[tokio::test]
async fn ui_page_cannot_be_framed_and_sends_no_referrer() {
    let s = start().await;
    let r = s.get("/__kaanal/ui").await;
    assert_eq!(r.status(), 200);
    let csp = r.headers()["content-security-policy"].to_str().unwrap();
    assert!(csp.contains("frame-ancestors 'none'"), "{csp}");
    assert_eq!(r.headers()["referrer-policy"], "no-referrer");
}

#[tokio::test]
async fn ui_page_is_served_without_token() {
    let s = start().await;
    let r = s.get("/__kaanal/ui").await;
    assert_eq!(r.status(), 200);
    assert!(
        r.headers()["content-type"]
            .to_str()
            .unwrap()
            .starts_with("text/html")
    );
    let html = r.text().await.unwrap();
    assert!(
        html.contains("Kaanal") && html.contains("EventSource") && html.contains("location.hash")
    );
}

/// Static guard for the page's security rules: no HTML-string sinks, no eval, no inline
/// handlers, no token storage, no external URLs. Request paths and node names are untrusted.
#[tokio::test]
async fn ui_page_avoids_unsafe_dom_apis_and_external_assets() {
    let s = start().await;
    let html = s.get("/__kaanal/ui").await.text().await.unwrap();
    for bad in [
        "innerHTML",
        "outerHTML",
        "insertAdjacentHTML",
        "document.write",
        "eval(",
        "onclick=",
        "localStorage",
        "http://",
        "https://",
    ] {
        assert!(!html.contains(bad), "page contains {bad}");
    }
}

#[tokio::test]
async fn ui_controls_have_accessible_names_and_dark_form_controls() {
    let s = start().await;
    let html = s.get("/__kaanal/ui").await.text().await.unwrap();
    assert!(html.contains("color-scheme: light dark"));
    for needle in ["setAttribute('aria-label'", "role=\"alert\""] {
        assert!(html.contains(needle), "missing {needle}");
    }
}

#[tokio::test]
async fn ui_keeps_stream_status_and_api_errors_on_separate_lines_and_uses_counters() {
    let s = start().await;
    let html = s.get("/__kaanal/ui").await.text().await.unwrap();
    assert!(html.contains("id=\"note\"") && html.contains("role=\"status\""));
    assert!(html.contains("const note = ") && html.contains("const say = "));
    // fixed-size counters, not a growing event list filtered every second
    assert!(html.contains("const buckets = ") && !html.contains("recent.filter"));
    // node names are used as Map keys, never as plain object keys
    assert!(!html.contains("stats[") && html.contains("new Map()"));
}

#[tokio::test]
async fn topology_reports_expected_visits_per_endpoint() {
    let src = GW_APP_DB.replace(
        "{ from: app, to: db, net: 1ms }",
        "{ from: app, to: db, net: 1ms, calls: 3, prob: 0.5 }",
    );
    let s = start_yaml(&src).await;
    let t: Value = s
        .get_authed("/__kaanal/topology")
        .await
        .json()
        .await
        .unwrap();
    let eps = t["endpoints"].as_array().unwrap();
    let get = eps.iter().find(|e| e["key"] == "GET /users/{id}").unwrap();
    assert_eq!(
        get["visits"],
        json!({ "nginx": 1.0, "app": 1.0, "db": 1.5 })
    );
    assert!(
        eps.iter()
            .all(|e| !e["key"].as_str().unwrap().starts_with("HEAD ")),
        "synthesized HEAD copies are not listed"
    );
}
