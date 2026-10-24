//! Timeouts and wire failures (reset, malformed) and their error-map and trace behaviour.

use crate::common::*;

use serde_json::{Value, json};

use std::time::Duration;

pub(crate) async fn raw_get(addr_base: &str, path: &str) -> (String, bool) {
    // returns (bytes read as lossy text, whether the read ended in an error rather than a clean EOF)
    // std sockets in spawn_blocking: tokio's `io-util` feature is not enabled in this crate (Task 0).
    use std::io::{Read, Write};
    let host = addr_base.trim_start_matches("http://").to_string();
    let path = path.to_string();
    tokio::task::spawn_blocking(move || {
        let mut s = std::net::TcpStream::connect(&host).unwrap();
        s.set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        s.write_all(
            format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: keep-alive\r\n\r\n")
                .as_bytes(),
        )
        .unwrap();
        let mut buf = Vec::new();
        let r = s.read_to_end(&mut buf);
        (String::from_utf8_lossy(&buf).into_owned(), r.is_err())
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn timeout_returns_504_near_the_deadline_with_kind_in_the_body() {
    let s = start_yaml(&yaml(", timeout: 200ms", "", "")).await;
    s.put_chaos("db", json!({ "latency_ms": 2000 })).await;
    let t0 = std::time::Instant::now();
    let r = s.get("/users/42").await;
    let took = t0.elapsed();
    assert_eq!(r.status(), 504);
    assert!(
        took >= Duration::from_millis(190) && took < Duration::from_millis(900),
        "{took:?}"
    );
    let body: Value = r.json().await.unwrap();
    assert_eq!(
        body,
        json!({ "error": "chaos", "node": "nginx", "kind": "timeout" })
    );
}

#[tokio::test]
async fn status_failure_body_is_unchanged() {
    // existing contract: no "kind" field for kind status
    let s = start().await;
    s.put_chaos("db", json!({ "error_rate": 1.0, "error_status": 503 }))
        .await;
    let b: Value = s.get("/users/42").await.json().await.unwrap();
    assert_eq!(b, json!({ "error": "chaos", "node": "db" }));
}

#[tokio::test]
async fn hang_with_a_short_node_timeout_answers_504() {
    let s = start_yaml(&yaml("", "", ", timeout: 300ms")).await;
    s.put_chaos("db", json!({ "error_rate": 1.0, "error_kind": "hang" }))
        .await;
    let t0 = std::time::Instant::now();
    let r = s.get("/users/42").await;
    assert_eq!(r.status(), 504);
    assert!(t0.elapsed() >= Duration::from_millis(290));
}

#[tokio::test]
async fn deep_reset_and_malformed_become_502() {
    let s = start().await;
    for k in ["reset", "malformed"] {
        s.put_chaos("db", json!({ "error_rate": 1.0, "error_kind": k }))
            .await;
        let r = s.get("/users/42").await;
        assert_eq!(r.status(), 502, "{k}");
        assert_eq!(r.json::<Value>().await.unwrap()["kind"], k);
    }
}

#[tokio::test]
async fn reset_on_the_owner_behind_a_gateway_aborts_the_connection() {
    // Z7: the gateway is transparent, the owner is a wire node
    let s = start().await;
    s.put_chaos("app", json!({ "error_rate": 1.0, "error_kind": "reset" }))
        .await;
    let (text, _) = raw_get(&s.base, "/users/42").await;
    assert!(
        text.starts_with("HTTP/1.1 200"),
        "Task 0: the head must arrive before the abort: {text:?}"
    );
    let declared: usize = text
        .lines()
        .find_map(|l| l.strip_prefix("content-length: "))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let body = text.split("\r\n\r\n").nth(1).unwrap_or("");
    assert!(
        !body.is_empty() && body.len() < declared,
        "partial body, then EOF: {text:?}"
    );
    let r = s
        .client
        .get(format!("{}/users/42", s.base))
        .send()
        .await
        .expect("the head arrives");
    assert_eq!(r.status(), 200);
    assert!(r.bytes().await.is_err(), "the body read must abort");
}

#[tokio::test]
async fn malformed_on_a_wire_node_is_valid_http_with_invalid_json() {
    let s = start().await;
    s.put_chaos(
        "app",
        json!({ "error_rate": 1.0, "error_kind": "malformed" }),
    )
    .await;
    let r = s.get("/users/42").await;
    assert_eq!(r.status(), 200);
    let declared = r.content_length();
    let text = r.text().await.unwrap();
    assert!(serde_json::from_str::<Value>(&text).is_err(), "{text}");
    assert_eq!(
        declared,
        Some(text.len() as u64),
        "Content-Length matches the cut body"
    );
    assert_ne!(text, "");
}

#[tokio::test]
async fn malformed_on_head_answers_the_plain_response() {
    let s = start().await;
    let clean = s.get("/users/42").await.content_length();
    s.put_chaos(
        "app",
        json!({ "error_rate": 1.0, "error_kind": "malformed" }),
    )
    .await;
    let r = s
        .client
        .head(format!("{}/users/42", s.base))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200, "nothing to cut");
    let declared: Option<u64> = r.headers()["content-length"].to_str().unwrap().parse().ok();
    assert_eq!(
        declared, clean,
        "HEAD declares the clean GET length, not a cut one"
    );
}

#[tokio::test]
async fn error_map_kind_key_overrides_the_wire_behaviour() {
    let src = format!(
        "{}  \"GET /users/{{id}}\": {{ error_map: {{ reset: {{ status: 418 }} }} }}\n",
        yaml("", "", "")
    );
    let s = start_yaml(&src).await;
    s.put_chaos("app", json!({ "error_rate": 1.0, "error_kind": "reset" }))
        .await;
    assert_eq!(s.get("/users/42").await.status(), 418);
}

#[tokio::test]
async fn class_key_does_not_swallow_a_wire_reset() {
    let src = format!(
        "{}  \"GET /users/{{id}}\": {{ error_map: {{ \"5xx\": {{ status: 599 }} }} }}\n",
        yaml("", "", "")
    );
    let s = start_yaml(&src).await;
    s.put_chaos("app", json!({ "error_rate": 1.0, "error_kind": "reset" }))
        .await;
    let r = s.client.get(format!("{}/users/42", s.base)).send().await;
    assert!(
        r.is_err() || r.unwrap().status() != 599,
        "the 5xx key must not turn a reset into JSON"
    );
}

#[tokio::test]
async fn endpoint_and_node_at_chaos_through_the_control_api() {
    let s = start().await;
    let put = |body: Value| {
        s.auth(
            s.client
                .put(format!("{}/__kaanal/chaos", s.base))
                .json(&body),
        )
        .send()
    };
    let r = put(json!({ "endpoint": "GET /users/{id}", "error_rate": 1.0, "error_status": 502 }))
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(s.get("/users/42").await.status(), 502);
    assert_eq!(
        s.get("/health").await.status(),
        200,
        "other endpoints untouched"
    );
    s.put_chaos("db", json!({ "latency_ms": 5 })).await;
    // exact-key delete
    let d = s
        .auth(s.client.delete(format!(
            "{}/__kaanal/chaos?endpoint=GET%20/users/%7Bid%7D",
            s.base
        )))
        .send()
        .await
        .unwrap();
    assert_eq!(d.status(), 204);
    assert_eq!(s.get("/users/42").await.status(), 200);
    let all: Value = s.get_authed("/__kaanal/chaos").await.json().await.unwrap();
    assert_eq!(
        all["nodes"]["db"]["latency_ms"], 5,
        "the exact-key delete leaves node chaos alone"
    );
    assert!(all["endpoints"].as_object().unwrap().is_empty());
    // a typo in the query is a 400 and wipes nothing
    let typo = s
        .auth(
            s.client
                .delete(format!("{}/__kaanal/chaos?endpoin=GET%20/x", s.base)),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(typo.status(), 400);
    assert!(typo.json::<Value>().await.unwrap()["error"].is_string());
    let all: Value = s.get_authed("/__kaanal/chaos").await.json().await.unwrap();
    assert_eq!(all["nodes"]["db"]["latency_ms"], 5);
    // node@endpoint
    let r = put(json!({ "node": "db", "endpoint": "get /users/{id}", "error_rate": 1.0 }))
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let all: Value = s.get_authed("/__kaanal/chaos").await.json().await.unwrap();
    assert_eq!(all["node_at"][0]["node"], "db");
    assert_eq!(all["node_at"][0]["endpoint"], "GET /users/{id}");
    assert_eq!(s.get("/users/42").await.status(), 503);
    let bad = put(json!({ "node": "db", "error_kind": "nope" }))
        .await
        .unwrap();
    assert_eq!(bad.status(), 400);
    let bad = put(json!({ "node": "db", "error_kind": "reset", "error_status": 200 }))
        .await
        .unwrap();
    assert_eq!(bad.status(), 400, "status is range-checked for every kind");
    let bad = put(json!({ "endpoint": "GET /nope", "down": true }))
        .await
        .unwrap();
    assert_eq!(bad.status(), 404);
    let none = put(json!({ "down": true })).await.unwrap();
    assert_eq!(none.status(), 400, "needs node or endpoint");
}

#[tokio::test]
async fn get_chaos_is_nested() {
    let s = start().await;
    s.put_chaos("db", json!({ "latency_ms": 5 })).await;
    let v: Value = s
        .auth(s.client.get(format!("{}/__kaanal/chaos", s.base)))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(v["nodes"]["db"]["latency_ms"], 5);
    assert_eq!(v["nodes"]["db"]["error_kind"], "status");
    assert!(v["endpoints"].is_object() && v["node_at"].is_array());
}

#[tokio::test]
async fn trace_names_reset_and_malformed_and_no_body_routes_get_the_plain_response() {
    let src = format!(
        "{}  \"GET /nocontent\": {{ override: {{ status: 204 }} }}\n  \"GET /five\": {{ override: {{ body: 5 }} }}\n",
        yaml("", "", "")
    );
    let s = start_yaml(&src).await;
    s.put_chaos(
        "app",
        json!({ "error_rate": 1.0, "error_kind": "malformed" }),
    )
    .await;
    let r = s.get("/users/42").await;
    assert!(
        trace_of(&r).ends_with("app.error=malformed"),
        "{}",
        trace_of(&r)
    );
    s.put_chaos("app", json!({ "error_kind": "reset" })).await;
    let (text, _) = raw_get(&s.base, "/users/42").await;
    assert!(text.to_lowercase().contains("app.error=reset"), "{text}");
    // a 204 has no body to break, even with reset at the gateway
    s.put_chaos("nginx", json!({ "error_rate": 1.0, "error_kind": "reset" }))
        .await;
    let r = s.get("/nocontent").await;
    assert_eq!(r.status(), 204);
    assert!(
        trace_of(&r).ends_with("nginx.error=reset"),
        "{}",
        trace_of(&r)
    );
    // a one-byte body: the declared length must still exceed what was sent
    let (text, _) = raw_get(&s.base, "/five").await;
    let declared: usize = text
        .lines()
        .find_map(|l| l.strip_prefix("content-length: "))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let body = text.split("\r\n\r\n").nth(1).unwrap_or("");
    assert!(!body.is_empty() && body.len() < declared, "{text:?}");
}

#[tokio::test]
async fn error_map_keyed_by_node_name_overrides_a_wire_reset() {
    let src = format!(
        "{}  \"GET /users/{{id}}\": {{ error_map: {{ app: {{ status: 418 }} }} }}\n",
        yaml("", "", "")
    );
    let s = start_yaml(&src).await;
    s.put_chaos("app", json!({ "error_rate": 1.0, "error_kind": "reset" }))
        .await;
    assert_eq!(s.get("/users/42").await.status(), 418);
}

#[tokio::test]
async fn endpoint_level_reset_belongs_to_the_gateway_and_aborts() {
    let s = start().await;
    let put = s
        .auth(s.client.put(format!("{}/__kaanal/chaos", s.base)))
        .json(&json!({ "endpoint": "GET /users/{id}", "error_rate": 1.0, "error_kind": "reset" }));
    assert_eq!(put.send().await.unwrap().status(), 200);
    let r = s
        .client
        .get(format!("{}/users/42", s.base))
        .send()
        .await
        .expect("the head arrives");
    assert!(
        r.headers()["x-kaanal-trace"]
            .to_str()
            .unwrap()
            .contains("nginx.error=reset")
    );
    assert!(r.bytes().await.is_err(), "the body read must abort");
}

#[tokio::test]
async fn a_gateway_timeout_goes_through_error_map_like_any_failure() {
    // Decided (B2a Z7 follow-up): a timeout at a wire node is not a wire break; `504` and `5xx` keys
    // remap it (nginx `error_page 504`), and without a key it stays a plain 504 with the kind in the body.
    let src = format!(
        "{}  \"GET /users/{{id}}\": {{ error_map: {{ \"504\": {{ status: 502, body: {{ mapped: true }} }} }} }}\n",
        yaml(", timeout: 200ms", "", "")
    );
    let s = start_yaml(&src).await;
    s.put_chaos("db", json!({ "latency_ms": 2000 })).await;
    let r = s.get("/users/42").await;
    assert_eq!(r.status(), 502);
    assert_eq!(r.json::<Value>().await.unwrap(), json!({ "mapped": true }));
}

#[tokio::test]
async fn malformed_on_an_auth_responder_cuts_the_real_reply() {
    let s = start_yaml(AUTH).await;
    s.put_chaos(
        "login",
        json!({ "error_rate": 1.0, "error_kind": "malformed" }),
    )
    .await;
    let r = s.get("/.well-known/jwks.json").await;
    assert_eq!(r.status(), 200);
    let text = r.text().await.unwrap();
    assert!(text.starts_with("{\"keys\""), "{text}");
    assert!(serde_json::from_str::<Value>(&text).is_err());
}
