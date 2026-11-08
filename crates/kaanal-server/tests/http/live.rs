//! The live routes: the stream's token rules, its first event, and the commands (step 8L).

use crate::common::*;
use serde_json::{Value, json};

#[tokio::test]
async fn the_live_stream_needs_a_token_and_a_wrong_one_is_401() {
    let s = start().await;
    let none = s
        .client
        .get(format!("{}/__kaanal/live", s.base))
        .send()
        .await
        .unwrap();
    assert_eq!(none.status(), 401);
    let wrong = s
        .client
        .get(format!("{}/__kaanal/live?token=wrong", s.base))
        .send()
        .await
        .unwrap();
    assert_eq!(wrong.status(), 401);
}

#[tokio::test]
async fn the_stream_takes_the_token_in_the_query_and_says_hello() {
    let s = start().await;
    let mut stream = s
        .client
        .get(format!(
            "{}/__kaanal/live?fresh=1&token={}",
            s.base, s.token
        ))
        .send()
        .await
        .unwrap();
    assert_eq!(stream.status(), 200);
    let mut seen = String::new();
    let got = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while let Some(chunk) = stream.chunk().await.unwrap() {
            seen.push_str(&String::from_utf8_lossy(&chunk));
            if seen.contains("event: hello") && seen.contains("\"session\"") {
                return true;
            }
        }
        false
    })
    .await
    .unwrap_or(false);
    assert!(got, "stream so far: {seen}");
}

#[tokio::test]
async fn commands_need_the_header_token_not_the_query() {
    let s = start().await;
    let r = s
        .client
        .post(format!("{}/__kaanal/live/pause?token={}", s.base, s.token))
        .send()
        .await
        .unwrap();
    assert_eq!(
        r.status(),
        401,
        "only the stream may take the token in the query"
    );
}

#[tokio::test]
async fn commands_without_a_page_connected_say_there_is_no_session() {
    let s = start().await;
    let r = s
        .auth(s.client.post(format!("{}/__kaanal/live/pause", s.base)))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 409);
}

#[tokio::test]
async fn pause_resume_and_a_refused_rate_over_http() {
    let s = start().await;
    let _stream = s
        .client
        .get(format!(
            "{}/__kaanal/live?fresh=1&token={}",
            s.base, s.token
        ))
        .send()
        .await
        .unwrap();
    let paused: Value = s
        .auth(s.client.post(format!("{}/__kaanal/live/pause", s.base)))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(paused["paused"], true);
    let resumed: Value = s
        .auth(s.client.post(format!("{}/__kaanal/live/resume", s.base)))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(resumed["paused"], false);
    let refused = s
        .auth(s.client.put(format!("{}/__kaanal/live/rate", s.base)))
        .json(&json!({ "rps": 1_000_000_000.0 }))
        .send()
        .await
        .unwrap();
    assert_eq!(refused.status(), 400);
    let ok = s
        .auth(s.client.put(format!("{}/__kaanal/live/rate", s.base)))
        .json(&json!({ "rps": 5.0 }))
        .send()
        .await
        .unwrap();
    assert_eq!(ok.status(), 200);
}

const WITH_TRAFFIC: &str = "version: 1
seed: 42
nodes:
  nginx: { type: gateway, proc: 1ms }
  app: { type: service, spec: openapi.yaml, proc: 1ms }
  db: { type: database, ops: { read: 1ms, write: 1ms, scan: 1ms } }
edges:
  - { from: nginx, to: app, net: 1ms }
  - { from: app, to: db, net: 1ms }
endpoints:
  \"GET /users/{id}\": { chain: [ db ] }
traffic:
  peak:
    rps: 200
    journeys:
      - { endpoint: 'GET /users/{id}', weight: 1 }
";

#[tokio::test]
async fn the_stream_carries_ticks_with_the_generators_numbers() {
    let s = start_yaml(WITH_TRAFFIC).await;
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
    let got = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        while let Some(chunk) = stream.chunk().await.unwrap() {
            seen.push_str(&String::from_utf8_lossy(&chunk));
            // a tick that has finished requests, a node list and sampled journeys
            if seen.matches("event: live").count() >= 3 && seen.contains("\"journeys\":[{") {
                return true;
            }
        }
        false
    })
    .await
    .unwrap_or(false);
    assert!(got, "stream so far: {seen}");
    let data = seen
        .lines()
        .rfind(|l| l.starts_with("data:") && l.contains("\"t_ms\""))
        .unwrap();
    let tick: Value = serde_json::from_str(data.trim_start_matches("data:").trim()).unwrap();
    assert_eq!(tick["nodes"].as_array().unwrap().len(), 3, "{tick}");
    assert_eq!(tick["status"]["rate"], 200.0);
    assert!(tick["t_ms"].as_u64().unwrap() >= 250);
}
