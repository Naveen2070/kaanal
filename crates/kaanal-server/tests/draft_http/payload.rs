//! `/__kaanal/payload`: live response edits (step 8A, RULES 108).

use crate::common::*;
use reqwest::Method;
use serde_json::{Value, json};

const PAYLOAD: &str = "/__kaanal/payload";
const USER_ID: &str = "/__kaanal/payload?route=GET%20%2Fusers%2F%7Bid%7D";

async fn put(s: &Srv, body: Value) -> reqwest::Response {
    s.send(Method::PUT, PAYLOAD, Some(body)).await
}

async fn mock(s: &Srv, path: &str) -> reqwest::Response {
    s.client
        .get(format!("{}{path}", s.base))
        .send()
        .await
        .unwrap()
}

fn with_collection() -> String {
    format!(
        "{}endpoints:\n  \"GET /users\": {{ collection: people }}\n",
        system("1ms")
    )
}

#[tokio::test]
async fn put_then_the_route_answers_the_edit_and_get_lists_it() {
    let s = start(&system("1ms")).await;
    let r = put(
        &s,
        json!({ "route": "GET /users/{id}", "status": 503, "body": { "down": true }, "headers": { "x-demo": "1" } }),
    )
    .await;
    assert_eq!(r.status(), 204);
    let r = mock(&s, "/users/1").await;
    assert_eq!(r.status(), 503);
    assert_eq!(r.headers()["x-demo"], "1");
    assert_eq!(r.json::<Value>().await.unwrap(), json!({ "down": true }));
    let list: Value = s.get(PAYLOAD).await.json().await.unwrap();
    assert_eq!(
        list,
        json!({ "edits": [ { "route": "GET /users/{id}", "status": 503, "body": { "down": true }, "headers": { "x-demo": "1" } } ] })
    );
}

#[tokio::test]
async fn an_unknown_route_is_404() {
    let s = start(&system("1ms")).await;
    let r = put(&s, json!({ "route": "GET /nope", "status": 200 })).await;
    assert_eq!(r.status(), 404);
}

#[tokio::test]
async fn a_collection_route_is_422_naming_the_route_field() {
    let s = start(&with_collection()).await;
    let r = put(&s, json!({ "route": "GET /users", "status": 200 })).await;
    assert_eq!(r.status(), 422);
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["field"], "route");
    assert!(v["error"].as_str().unwrap().contains("collection"));
}

#[tokio::test]
async fn bad_values_are_422_naming_the_field() {
    let s = start(&system("1ms")).await;
    let cases = [
        (
            json!({ "route": "GET /users/{id}", "status": 99 }),
            "status",
        ),
        (
            json!({ "route": "GET /users/{id}", "status": 204, "body": { "a": 1 } }),
            "status",
        ),
        (
            json!({ "route": "GET /users/{id}", "headers": { "content-length": "1" } }),
            "headers",
        ),
        (
            json!({ "route": "GET /users/{id}", "headers": { "x-a": "a\r\nx-b: 1" } }),
            "headers",
        ),
    ];
    for (body, field) in cases {
        let r = put(&s, body.clone()).await;
        assert_eq!(r.status(), 422, "{body}");
        let v: Value = r.json().await.unwrap();
        assert_eq!(v["field"], field, "{body}");
    }
    let list: Value = s.get(PAYLOAD).await.json().await.unwrap();
    assert_eq!(list, json!({ "edits": [] }));
}

#[tokio::test]
async fn a_body_over_the_cap_is_413_and_malformed_json_is_400() {
    let s = start(&system("1ms")).await;
    let big = format!(
        "{{\"route\":\"GET /users/{{id}}\",\"body\":\"{}\"}}",
        "a".repeat((1 << 20) + 8192)
    );
    let r = s
        .client
        .put(format!("{}{PAYLOAD}", s.base))
        .bearer_auth(&s.token)
        .header("content-type", "application/json")
        .body(big)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 413);
    let r = s
        .client
        .put(format!("{}{PAYLOAD}", s.base))
        .bearer_auth(&s.token)
        .body("not json")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 400);
}

#[tokio::test]
async fn delete_removes_the_edit_then_404() {
    let s = start(&system("1ms")).await;
    put(&s, json!({ "route": "GET /users/{id}", "status": 503 })).await;
    assert_eq!(s.send(Method::DELETE, USER_ID, None).await.status(), 204);
    assert_eq!(mock(&s, "/users/1").await.status(), 200);
    assert_eq!(s.send(Method::DELETE, USER_ID, None).await.status(), 404);
}

#[tokio::test]
async fn preview_is_what_the_route_answers_without_the_edit() {
    let s = start(&system("1ms")).await;
    let q = "/__kaanal/payload/preview?route=GET%20%2Fusers%2F%7Bid%7D";
    let before: Value = s.get(q).await.json().await.unwrap();
    assert_eq!(before["status"], 200);
    assert!(before["body"].is_object());
    put(
        &s,
        json!({ "route": "GET /users/{id}", "status": 503, "body": 1 }),
    )
    .await;
    let after: Value = s.get(q).await.json().await.unwrap();
    assert_eq!(before, after, "an edit never shows in the preview");
    let gone = s.get("/__kaanal/payload/preview?route=GET%20%2Fnope").await;
    assert_eq!(gone.status(), 404);
}

#[tokio::test]
async fn the_routes_need_the_token() {
    let s = start(&system("1ms")).await;
    for (m, p) in [
        (Method::GET, "/__kaanal/payload"),
        (Method::PUT, "/__kaanal/payload"),
        (Method::DELETE, "/__kaanal/payload?route=GET%20%2Fx"),
        (Method::GET, "/__kaanal/payload/preview?route=GET%20%2Fx"),
    ] {
        let r = s
            .client
            .request(m.clone(), format!("{}{p}", s.base))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 401, "{m} {p}");
    }
}

#[tokio::test]
async fn each_put_and_delete_marks_the_live_timeline() {
    let s = start(&system("1ms")).await;
    let mut stream = s
        .client
        .get(format!(
            "{}/__kaanal/live?fresh=1&token={}",
            s.base, s.token
        ))
        .send()
        .await
        .unwrap();
    let mut seen = String::new();
    let mut read_until = async |needle: &str| {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while let Some(chunk) = stream.chunk().await.unwrap() {
                seen.push_str(&String::from_utf8_lossy(&chunk));
                if seen.contains(needle) {
                    return true;
                }
            }
            false
        })
        .await
        .unwrap_or(false)
    };
    assert!(read_until("event: hello").await, "no hello");
    put(&s, json!({ "route": "GET /users/{id}", "status": 503 })).await;
    assert!(read_until("GET /users/{id} answers 503").await, "no marker");
    s.send(Method::DELETE, USER_ID, None).await;
    assert!(
        read_until("GET /users/{id} back to normal").await,
        "no marker"
    );
}

#[tokio::test]
async fn apply_prunes_an_edit_of_a_route_that_became_a_collection_and_says_so() {
    let s = start(&system("1ms")).await;
    put(&s, json!({ "route": "GET /users", "status": 503 })).await;
    let text = with_collection();
    assert_eq!(
        s.put("/__kaanal/draft", json!({ "text": text }))
            .await
            .status(),
        200
    );
    let r = s.send(Method::POST, "/__kaanal/draft/apply", None).await;
    assert_eq!(r.status(), 200);
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["payloads_dropped"], json!(["GET /users"]));
    let list: Value = s.get(PAYLOAD).await.json().await.unwrap();
    assert_eq!(list, json!({ "edits": [] }));
}
