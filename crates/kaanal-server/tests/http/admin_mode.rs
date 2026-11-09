//! Admin mode end to end (step 8A): a real request shows in the live ticks, a live response edit
//! reaches the next request, a scoped request never sees it, and taking it back restores the route.

use crate::common::*;
use serde_json::{Value, json};
use std::time::Duration;

fn yaml() -> String {
    format!(
        "{GW_APP_DB}  \"GET /health\": {{ override: {{ body: {{ ok: true }} }} }}\nscenarios:\n  quiet: {{ chaos: [ {{ node: db, latency_ms: 1 }} ] }}\n"
    )
}

/// Reads the live stream until `needle` appears in what it has sent so far.
async fn read_until(stream: &mut reqwest::Response, seen: &mut String, needle: &str) -> bool {
    tokio::time::timeout(Duration::from_secs(8), async {
        if seen.contains(needle) {
            return true;
        }
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
}

async fn last_log(s: &Srv) -> Value {
    let v: Value = s
        .get_authed("/__kaanal/logs?limit=1")
        .await
        .json()
        .await
        .unwrap();
    v["entries"][0].clone()
}

#[tokio::test]
async fn a_real_request_is_counted_then_edited_then_scoped_then_restored() {
    let s = start_yaml(&yaml()).await;
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
    assert!(
        read_until(&mut stream, &mut seen, "event: hello").await,
        "no hello"
    );

    // 1. a request a frontend made is in the next tick, as real
    assert_eq!(s.get("/users/1").await.status(), 200);
    assert!(
        read_until(&mut stream, &mut seen, "\"real\":{\"requests\":1").await,
        "no real request in the ticks: {seen}"
    );
    assert!(last_log(&s).await.get("edited").is_none());

    // 2. a live edit reaches the next request, with its headers, and the log marks it
    let put = s
        .auth(s.client.put(format!("{}/__kaanal/payload", s.base)))
        .json(&json!({
            "route": "GET /users/{id}",
            "status": 503,
            "body": { "down": true },
            "headers": { "x-demo": "1" }
        }))
        .send()
        .await
        .unwrap();
    assert_eq!(put.status(), 204);
    let r = s.get("/users/1").await;
    assert_eq!(r.status(), 503);
    assert_eq!(r.headers()["x-demo"], "1");
    assert_eq!(r.json::<Value>().await.unwrap(), json!({ "down": true }));
    assert_eq!(last_log(&s).await["edited"], true);
    assert!(
        read_until(&mut stream, &mut seen, "GET /users/{id} answers 503").await,
        "no marker for the edit"
    );

    // 3. a scoped request never sees live state, edits included
    let scoped = s
        .client
        .get(format!("{}/users/1", s.base))
        .header("x-kaanal-scenario", "quiet")
        .send()
        .await
        .unwrap();
    assert_eq!(scoped.status(), 200);
    assert!(last_log(&s).await.get("edited").is_none());

    // 4. back to normal
    let del = s
        .auth(s.client.delete(format!(
            "{}/__kaanal/payload?route=GET%20%2Fusers%2F%7Bid%7D",
            s.base
        )))
        .send()
        .await
        .unwrap();
    assert_eq!(del.status(), 204);
    assert_eq!(s.get("/users/1").await.status(), 200);
    assert!(last_log(&s).await.get("edited").is_none());
}

#[tokio::test]
async fn a_collection_route_refuses_an_edit_over_http() {
    let src = format!("{GW_APP_DB}  \"GET /users\": {{ collection: people }}\n");
    let s = start_yaml(&src).await;
    let r = s
        .auth(s.client.put(format!("{}/__kaanal/payload", s.base)))
        .json(&json!({ "route": "GET /users", "status": 200 }))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 422);
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["field"], "route");
    // nothing was stored: the route still answers from its collection
    let list: Value = s
        .get_authed("/__kaanal/payload")
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(list, json!({ "edits": [] }));
}
