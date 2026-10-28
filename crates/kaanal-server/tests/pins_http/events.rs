//! Saving pins, the events pins and cycles emit, and no-op deletes.

use crate::common::*;
use kaanal_core::Config;
use kaanal_server::ServeOpts;
use reqwest::Method;
use serde_json::{Value, json};
use std::time::Duration;

#[tokio::test]
async fn save_writes_pins_and_reload_round_trips() {
    let s = start().await;
    s.activate("p-503").await;
    s.pin(json!({"endpoint": "POST /orders", "state": "empty"}))
        .await;
    s.pin(json!({"endpoint": "GET /users", "state": "slow", "delay": "1s500ms"}))
        .await;
    let save = |body: Value| s.send(Method::POST, "/__kaanal/scenarios/save", Some(body));
    let r = save(json!({"name": "from-live"})).await;
    assert_eq!(r.status(), 200, "{}", r.text().await.unwrap());
    assert_eq!(
        save(json!({"name": "from-eff", "from": "effective"}))
            .await
            .status(),
        200
    );
    assert_eq!(
        save(json!({"name": "from-sc", "from": {"scenario": "p-503"}}))
            .await
            .status(),
        200
    );
    let live = std::fs::read_to_string(s.dir.join("scenarios/from-live.yaml")).unwrap();
    assert!(live.contains("pins:") && !live.contains("chaos:"), "{live}");
    assert!(live.contains("1s 500ms"), "{live}");
    assert!(!live.contains("GET /users/{id}"), "{live}");
    let eff = std::fs::read_to_string(s.dir.join("scenarios/from-eff.yaml")).unwrap();
    assert!(
        eff.contains("GET /users/{id}") && eff.contains("POST /orders"),
        "{eff}"
    );
    let sc = std::fs::read_to_string(s.dir.join("scenarios/from-sc.yaml")).unwrap();
    assert!(sc.contains("503"), "{sc}");
    // nothing to save: no chaos and no pins
    s.send(Method::DELETE, "/__kaanal/pins", None).await;
    s.deactivate("p-503").await;
    assert_eq!(save(json!({"name": "none"})).await.status(), 400);
    // clear_live also clears live pins
    s.pin(json!({"endpoint": EP, "state": "error"})).await;
    let r = save(json!({"name": "cl", "clear_live": true})).await;
    assert_eq!(r.status(), 200);
    assert_eq!(s.pins().await["live"], json!([]));
    // reload: a fresh server over the same directory reads the pins-only files
    let cfg = Config::from_yaml(&format!("{NODES}{ENDPOINTS}")).unwrap();
    let opts = ServeOpts {
        host: "127.0.0.1".parse().unwrap(),
        port: 0,
        token: "t".into(),
        auth_key: None,
        draft: None,
    };
    let s2 = kaanal_server::serve(&cfg, &s.dir, opts).await.unwrap();
    let base = format!("http://127.0.0.1:{}", s2.addr.port());
    let c = reqwest::Client::new();
    let post = |path: String| c.post(format!("{base}{path}")).bearer_auth("t").send();
    for (name, orders, user) in [
        ("from-live", true, 200),
        ("from-eff", true, 503),
        ("from-sc", false, 503),
    ] {
        let r = post(format!("/__kaanal/scenarios/{name}/activate"))
            .await
            .unwrap();
        assert_eq!(r.status(), 200, "{name}");
        let r = post("/orders".into()).await.unwrap();
        assert_eq!(r.status(), 201, "{name}");
        let body = r.text().await.unwrap();
        assert_eq!(body == r#"{"orders":[]}"#, orders, "{name}: {body}");
        let r = c.get(format!("{base}{USER}")).send().await.unwrap();
        assert_eq!(r.status(), user, "{name}");
        assert_eq!(post("/__kaanal/reset".into()).await.unwrap().status(), 204);
    }
}

#[tokio::test]
async fn scenario_events_for_pin_and_cycle() {
    let s = start().await;
    let mut stream = s
        .client
        .get(format!("{}/__kaanal/events?token={}", s.base, s.token))
        .send()
        .await
        .unwrap();
    s.pin(json!({"endpoint": EP, "state": "error"})).await;
    s.cycle(json!({"endpoint": "POST /orders"})).await;
    s.send(Method::DELETE, "/__kaanal/pins", None).await;
    let mut seen = String::new();
    let ok = tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(chunk) = stream.chunk().await.unwrap() {
            seen.push_str(&String::from_utf8_lossy(&chunk));
            if seen.matches("\"action\":\"pin\"").count() >= 2 {
                return true;
            }
        }
        false
    })
    .await
    .unwrap_or(false);
    assert!(ok, "{seen}");
    assert!(seen.contains("\"action\":\"cycle\""), "{seen}");
    assert!(seen.contains("\"name\":\"POST /orders\""), "{seen}");
    assert!(seen.contains(&format!("\"name\":\"{EP}\"")), "{seen}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_cycles_on_one_endpoint_walk_distinct_consecutive_states() {
    let s = start().await;
    for _ in 0..50 {
        s.reset().await;
        // Five spawned tasks released together by a barrier, so the handlers truly overlap.
        let gate = std::sync::Arc::new(tokio::sync::Barrier::new(5));
        let tasks: Vec<_> = (0..5)
            .map(|_| {
                let (c, url, tok, gate) = (
                    s.client.clone(),
                    format!("{}/__kaanal/cycle", s.base),
                    s.token.clone(),
                    gate.clone(),
                );
                tokio::spawn(async move {
                    gate.wait().await;
                    let r = c
                        .post(url)
                        .bearer_auth(tok)
                        .json(&json!({"endpoint": EP}))
                        .send()
                        .await
                        .unwrap();
                    r.json::<Value>().await.unwrap()["state"]
                        .as_str()
                        .unwrap()
                        .to_string()
                })
            })
            .collect();
        let mut seen = Vec::new();
        for t in tasks {
            seen.push(t.await.unwrap());
        }
        seen.sort();
        assert_eq!(seen, ["empty", "error", "loading", "slow", "success"]);
    }
}

#[tokio::test]
async fn no_op_deletes_publish_no_event_and_real_ones_do() {
    let s = start().await;
    let mut stream = s
        .client
        .get(format!("{}/__kaanal/events?token={}", s.base, s.token))
        .send()
        .await
        .unwrap();
    // absent key, unknown endpoint and an empty live layer: all 204, nothing published
    assert_eq!(s.send(Method::DELETE, EP_Q, None).await.status(), 204);
    let unknown = "/__kaanal/pins?endpoint=GET%20/nope";
    assert_eq!(s.send(Method::DELETE, unknown, None).await.status(), 204);
    assert_eq!(
        s.send(Method::DELETE, "/__kaanal/pins", None)
            .await
            .status(),
        204
    );
    s.pin(json!({"endpoint": EP, "state": "error"})).await;
    assert_eq!(s.send(Method::DELETE, EP_Q, None).await.status(), 204);
    // clearing a non-empty live layer is a real removal: one more event
    s.pin(json!({"endpoint": EP, "state": "error"})).await;
    assert_eq!(
        s.send(Method::DELETE, "/__kaanal/pins", None)
            .await
            .status(),
        204
    );
    // a reset marks the end of the stream we care about
    s.reset().await;
    let mut seen = String::new();
    let ok = tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(chunk) = stream.chunk().await.unwrap() {
            seen.push_str(&String::from_utf8_lossy(&chunk));
            if seen.contains("\"action\":\"reset\"") {
                return true;
            }
        }
        false
    })
    .await
    .unwrap_or(false);
    // the put and the real delete, then the put and the clear-all
    assert!(ok, "{seen}");
    assert_eq!(seen.matches("\"action\":\"pin\"").count(), 4, "{seen}");
}
