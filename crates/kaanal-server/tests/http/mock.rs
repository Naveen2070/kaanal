//! Mock routes: generated and static answers, 404 and 405, CORS, HEAD, path parameters.

use crate::common::*;
use serde_json::{Value, json};

#[tokio::test]
async fn users_42_is_stable_traced_and_uses_path_id() {
    let s = start().await;
    let r1 = s.get("/users/42").await;
    assert_eq!(r1.status(), 200);
    let trace = r1.headers()["x-kaanal-trace"].to_str().unwrap().to_string();
    assert!(
        trace.starts_with("nginx.proc=") && trace.contains("db.read="),
        "{trace}"
    );
    let b1: Value = r1.json().await.unwrap();
    let b2: Value = s.get("/users/42").await.json().await.unwrap();
    assert_eq!(b1, b2);
    assert_eq!(b1["id"], 42);
    assert!(b1["email"].as_str().unwrap().ends_with("@example.com"));
}

#[tokio::test]
async fn non_numeric_id_for_integer_field_still_returns_200() {
    let s = start().await;
    let r = s.get("/users/abc").await;
    assert_eq!(r.status(), 200);
    assert!(r.json::<Value>().await.unwrap()["id"].is_i64());
}

#[tokio::test]
async fn static_override_beats_generated() {
    let s = start().await;
    let body: Value = s.get("/users").await.json().await.unwrap();
    assert_eq!(body, json!([{ "id": 1 }]));
}

#[tokio::test]
async fn static_only_route_works() {
    let s = start().await;
    let r = s.get("/health").await;
    assert_eq!(r.status(), 200);
    assert_eq!(r.json::<Value>().await.unwrap(), json!({ "ok": true }));
}

#[tokio::test]
async fn post_orders_returns_201() {
    let s = start().await;
    let r = s
        .client
        .post(format!("{}/orders", s.base))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 201);
    let b: Value = r.json().await.unwrap();
    assert_eq!(b["currency"], "INR");
}

#[tokio::test]
async fn unknown_and_trailing_slash_paths_are_404() {
    let s = start().await;
    for p in ["/nope", "/users/42/", "/__kaanal/nope"] {
        let r = s.get(p).await;
        assert_eq!(r.status(), 404, "{p}");
        assert_eq!(r.json::<Value>().await.unwrap()["error"], "no route");
    }
}

#[tokio::test]
async fn wrong_method_is_405() {
    let s = start().await;
    let r = s
        .client
        .delete(format!("{}/users/1", s.base))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 405);
}

#[tokio::test]
async fn mocks_send_cors_headers_and_answer_preflight() {
    let s = start().await;
    let r = s.get("/users/1").await;
    assert_eq!(r.headers()["access-control-allow-origin"], "*");
    let pre = s
        .client
        .request(reqwest::Method::OPTIONS, format!("{}/orders", s.base))
        .send()
        .await
        .unwrap();
    assert_eq!(pre.status(), 204);
    assert_eq!(pre.headers()["access-control-allow-origin"], "*");
}

#[tokio::test]
async fn health_needs_no_token_and_has_no_cors() {
    let s = start().await;
    let r = s.get("/__kaanal/health").await;
    assert_eq!(r.status(), 200);
    assert!(r.headers().get("access-control-allow-origin").is_none());
}

/// The status page's three facts, and nothing a token guards.
#[tokio::test]
async fn health_says_version_uptime_and_whether_a_system_is_loaded_and_nothing_more() {
    let s = start().await;
    let v: Value = s.get("/__kaanal/health").await.json().await.unwrap();
    assert_eq!(v["ok"], true);
    assert_eq!(v["version"], env!("CARGO_PKG_VERSION"));
    assert!(v["uptime_s"].as_u64().is_some(), "{v}");
    assert_eq!(v["system"], true);
    let mut keys: Vec<&str> = v.as_object().unwrap().keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(keys, ["ok", "system", "uptime_s", "version"]);
}

#[tokio::test]
async fn control_namespace_never_gets_cors_even_for_404_and_preflight() {
    let s = start().await;
    let r = s.get("/__kaanal/nope").await;
    assert_eq!(r.status(), 404);
    assert!(r.headers().get("access-control-allow-origin").is_none());
    let pre = s
        .client
        .request(
            reqwest::Method::OPTIONS,
            format!("{}/__kaanal/chaos", s.base),
        )
        .send()
        .await
        .unwrap();
    assert!(pre.headers().get("access-control-allow-origin").is_none());
}

#[tokio::test]
async fn head_answers_like_get_without_a_body() {
    let s = start().await;
    let r = s
        .client
        .head(format!("{}/users/42", s.base))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    assert!(r.headers().contains_key("x-kaanal-trace"));
    assert_eq!(r.bytes().await.unwrap(), "");
}

#[tokio::test]
async fn head_has_its_own_counter_and_never_shifts_get() {
    // R6.8: with 50% db errors, each GET's outcome depends on GET's counter alone.
    let status = |r: reqwest::Response| r.status().as_u16();
    let plain = start().await;
    plain.put_chaos("db", json!({ "error_rate": 0.5 })).await;
    let mut want = Vec::new();
    for _ in 0..8 {
        want.push(status(plain.get("/users/42").await));
    }
    let mixed = start().await;
    mixed.put_chaos("db", json!({ "error_rate": 0.5 })).await;
    let mut got = Vec::new();
    for _ in 0..8 {
        got.push(status(mixed.get("/users/42").await));
        let head = mixed
            .client
            .head(format!("{}/users/42", mixed.base))
            .send()
            .await
            .unwrap();
        assert!(head.status() == 200 || head.status() == 503);
    }
    assert_eq!(got, want, "HEADs between GETs must not change GET outcomes");
    assert!(want.contains(&200) && want.contains(&503), "{want:?}");
}

#[tokio::test]
async fn percent_encoded_path_param_is_decoded_before_use() {
    let s = start().await;
    // %34%32 is "42"
    let b: Value = s.get("/users/%34%32").await.json().await.unwrap();
    assert_eq!(b["id"], 42);
}

impl Srv {
    pub fn auth(&self, rb: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        rb.bearer_auth(&self.token)
    }
    pub async fn put_chaos(&self, node: &str, body: Value) -> reqwest::Response {
        self.auth(
            self.client
                .put(format!("{}/__kaanal/chaos/{node}", self.base)),
        )
        .json(&body)
        .send()
        .await
        .unwrap()
    }
    pub async fn get_authed(&self, path: &str) -> reqwest::Response {
        self.auth(self.client.get(format!("{}{path}", self.base)))
            .send()
            .await
            .unwrap()
    }
}
