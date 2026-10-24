//! The pin states on a request: loading, slow, error, empty, success, HEAD, and how pins meet chaos and wire failures.

use crate::common::*;
use kaanal_core::Config;
use serde_json::{Value, json};
use std::time::{Duration, Instant};

#[tokio::test]
async fn loading_pin_answers_after_the_delay_without_a_timeout() {
    let s = start().await;
    let t = Instant::now();
    let r = s.scoped(USER, "p-loading").await;
    assert!(
        t.elapsed() >= Duration::from_millis(1150),
        "{:?}",
        t.elapsed()
    );
    assert_eq!(r.status(), 200);
    let tr = trace(&r);
    assert!(!tr.contains("timeout") && !tr.contains("error"), "{tr}");
    assert!(tr.ends_with("pinned=loading"), "{tr}");
    let b: Value = r.json().await.unwrap();
    assert!(b["id"].is_number(), "{b}");
}

#[tokio::test]
async fn slow_pin_adds_the_delay() {
    let s = start().await;
    let t = Instant::now();
    let r = s.scoped(USER, "p-slow").await;
    assert!(
        t.elapsed() >= Duration::from_millis(280),
        "{:?}",
        t.elapsed()
    );
    assert_eq!(r.status(), 200);
    assert!(trace(&r).ends_with("pinned=slow"));
    let t = Instant::now();
    assert_eq!(s.status(USER).await, 200);
    assert!(t.elapsed() < Duration::from_millis(250));
}

#[tokio::test]
async fn error_pin_defaults_and_overrides() {
    let s = start().await;
    let r = s.scoped("/users", "p-err").await;
    assert_eq!(r.status(), 500);
    assert!(r.headers().get("retry-after").is_none());
    assert_eq!(
        r.json::<Value>().await.unwrap(),
        json!({"error": "pinned", "state": "error"})
    );

    let r = s.scoped(USER, "p-err-states").await;
    assert_eq!(r.status(), 404);
    assert_eq!(r.json::<Value>().await.unwrap(), json!({"error": "gone"}));

    let r = s.scoped("/users", "p-429").await;
    assert_eq!(r.status(), 429);
    assert_eq!(r.headers()["retry-after"], "1");
    assert!(trace(&r).ends_with("pinned=error"));

    let r = s.scoped(USER, "p-503").await;
    assert_eq!(r.status(), 503);
    assert_eq!(r.headers()["retry-after"], "1");
}

#[tokio::test]
async fn empty_pin_from_schema_and_from_states_override() {
    let s = start().await;
    let r = s.scoped("/users", "p-empty").await;
    assert_eq!(r.status(), 200);
    assert!(trace(&r).ends_with("pinned=empty"));
    assert_eq!(r.json::<Value>().await.unwrap(), json!([]));
    let r = s
        .client
        .post(format!("{}/orders", s.base))
        .header("x-kaanal-scenario", "p-empty")
        .send()
        .await
        .unwrap();
    // the route's normal success status (201), the states.empty body
    assert_eq!(r.status(), 201);
    assert_eq!(r.json::<Value>().await.unwrap(), json!({"orders": []}));
}

#[tokio::test]
async fn empty_pin_on_a_204_route_stays_204() {
    let s = start().await;
    assert_eq!(s.status("/gone").await, 204);
    let r = s.scoped("/gone", "p-204").await;
    assert_eq!(r.status(), 204);
    assert!(trace(&r).ends_with("pinned=empty"));
    assert!(r.bytes().await.unwrap().is_empty());
}

#[tokio::test]
async fn empty_pin_on_static_override_route_derives_from_the_body() {
    let s = start().await;
    let r = s.scoped("/health", "p-static").await;
    assert_eq!(r.status(), 200);
    assert_eq!(
        r.json::<Value>().await.unwrap(),
        json!({"ok": true, "items": [], "total": 0})
    );
}

#[tokio::test]
async fn success_pin_hides_a_downed_node_and_a_429_limit_and_error_map() {
    let s = start().await;
    // control: without the pin, db down is the error_map's 502
    assert_eq!(s.scoped(USER, "db-down").await.status(), 502);
    let r = s.scoped(USER, "db-down,p-success").await;
    assert_eq!(r.status(), 200);
    let tr = trace(&r);
    assert!(tr.ends_with("pinned=success"), "{tr}");
    assert!(
        tr.contains("error"),
        "the chain still failed in the trace: {tr}"
    );
    assert!(r.json::<Value>().await.unwrap()["id"].is_number());

    // a 429 rate limit
    let yaml = format!("{NODES}{ENDPOINTS}{SCENARIOS}").replace(
        "nginx: { type: gateway, proc: 1ms, timeout: 500ms }",
        "nginx: { type: gateway, proc: 1ms, timeout: 500ms, rate_limit: { rps: 1, burst: 1 } }",
    );
    let s = start_yaml(&yaml).await;
    let mut limited = 0;
    for _ in 0..4 {
        limited += usize::from(s.status(USER).await == 429);
    }
    assert!(limited > 0, "the limit must bite without a pin");
    tokio::time::sleep(Duration::from_millis(1100)).await;
    for _ in 0..4 {
        assert_eq!(s.scoped(USER, "p-success").await.status(), 200);
    }
}

#[tokio::test]
async fn pin_with_chaos_hang_on_the_endpoint_keeps_the_delay() {
    let s = start().await;
    let t = Instant::now();
    let r = s.scoped(USER, "hang,p-loading-short").await;
    // 300 ms of pin delay, then the hang lasts the 500 ms gateway timeout
    assert!(
        t.elapsed() >= Duration::from_millis(750),
        "{:?}",
        t.elapsed()
    );
    assert_eq!(r.status(), 200);
    assert!(trace(&r).ends_with("pinned=loading"));
}

#[tokio::test]
async fn head_follows_the_get_pin() {
    let s = start().await;
    let get = s.scoped(USER, "p-503").await;
    let head = s
        .client
        .head(format!("{}{USER}", s.base))
        .header("x-kaanal-scenario", "p-503")
        .send()
        .await
        .unwrap();
    assert_eq!(head.status(), get.status());
    assert_eq!(head.headers()["retry-after"], "1");
    assert!(trace(&head).ends_with("pinned=error"));
    assert!(head.bytes().await.unwrap().is_empty());
}

#[tokio::test]
async fn pin_does_not_change_seeded_draws() {
    let s = start().await;
    s.activate("flaky").await;
    s.reset().await;
    s.activate("flaky").await;
    let mut plain = Vec::new();
    for _ in 0..20 {
        plain.push(s.status(USER).await);
    }
    assert!(
        plain.contains(&200) && plain.iter().any(|c| *c != 200),
        "{plain:?}"
    );

    s.reset().await;
    s.activate("flaky").await;
    s.activate("p-success").await;
    for want in &plain[..10] {
        let r = s.get(USER).await;
        assert_eq!(r.status(), 200);
        // the chain's draws are the same: it failed here exactly when the plain run failed
        assert_eq!(trace(&r).contains(".error="), *want != 200, "{}", trace(&r));
    }
    s.deactivate("p-success").await;
    let mut after = Vec::new();
    for _ in 0..10 {
        after.push(s.status(USER).await);
    }
    assert_eq!(after, plain[10..]);
}

#[tokio::test]
async fn trace_ends_with_pinned_and_event_carries_pin() {
    let s = start().await;
    let mut stream = s
        .client
        .get(format!("{}/__kaanal/events?token={}", s.base, s.token))
        .send()
        .await
        .unwrap();
    let r = s.scoped("/users", "p-err").await;
    assert_eq!(r.status(), 500);
    assert!(trace(&r).ends_with(",pinned=error") || trace(&r) == "pinned=error");
    let mut seen = String::new();
    let ok = tokio::time::timeout(Duration::from_secs(3), async {
        while let Some(chunk) = stream.chunk().await.unwrap() {
            seen.push_str(&String::from_utf8_lossy(&chunk));
            if seen.contains("\"pin\":\"error\"") {
                return true;
            }
        }
        false
    })
    .await
    .unwrap_or(false);
    assert!(ok, "{seen}");
    assert!(seen.contains("\"failed_node\":null"), "{seen}");
}

// ponytail: scoped_request_sees_scenario_pins_not_live_pins needs the live-pin API (Task 4);
// the scoped tests above already prove a scoped request reads its own stack's pins.

#[tokio::test]
async fn states_empty_body_is_verbatim_not_param_substituted() {
    let s = start().await;
    let r = s.scoped(USER, "p-empty-user").await;
    assert_eq!(r.status(), 200);
    assert_eq!(r.json::<Value>().await.unwrap(), json!({"id": "keep"}));
}

#[tokio::test]
async fn pin_beats_wire_failures_and_a_downed_node() {
    let s = start().await;
    for chaos in ["app-reset", "app-malformed"] {
        let kind = chaos.trim_start_matches("app-");
        for (pin, state) in [("p-success", "success"), ("p-empty-user", "empty")] {
            let r = s.scoped(USER, &format!("{chaos},{pin}")).await;
            assert_eq!(r.status(), 200, "{chaos} {pin}");
            let tr = trace(&r);
            assert!(tr.contains(&format!("app.error={kind}")), "{tr}");
            assert!(tr.ends_with(&format!(",pinned={state}")), "{tr}");
            let b: Value = r.json().await.expect("an unbroken JSON body");
            assert!(b.is_object(), "{b}");
        }
    }
    let r = s.scoped(USER, "db-down,p-err-states").await;
    assert_eq!(r.status(), 404);
    assert_eq!(r.json::<Value>().await.unwrap(), json!({"error": "gone"}));
}

#[tokio::test]
async fn head_on_empty_and_loading_pins_has_no_body() {
    let s = start().await;
    for pin in ["p-empty-user", "p-loading-short"] {
        let get = s.scoped(USER, pin).await.status();
        let head = s
            .client
            .head(format!("{}{USER}", s.base))
            .header("x-kaanal-scenario", pin)
            .send()
            .await
            .unwrap();
        assert_eq!(head.status(), get, "{pin}");
        assert!(trace(&head).contains("pinned="), "{pin}");
        assert!(head.bytes().await.unwrap().is_empty());
    }
}

#[tokio::test]
async fn pinned_event_lists_the_chain_nodes_but_no_failure() {
    let s = start().await;
    let mut stream = s
        .client
        .get(format!("{}/__kaanal/events?token={}", s.base, s.token))
        .send()
        .await
        .unwrap();
    assert_eq!(s.scoped(USER, "db-down,p-success").await.status(), 200);
    let mut seen = String::new();
    let ok = tokio::time::timeout(Duration::from_secs(3), async {
        while let Some(chunk) = stream.chunk().await.unwrap() {
            seen.push_str(&String::from_utf8_lossy(&chunk));
            if seen.contains("\"pin\":\"success\"") {
                return true;
            }
        }
        false
    })
    .await
    .unwrap_or(false);
    assert!(ok, "{seen}");
    let line = seen
        .lines()
        .find(|l| l.contains("\"pin\":\"success\""))
        .unwrap();
    let ev: Value = serde_json::from_str(line.trim_start_matches("data:").trim()).unwrap();
    assert!(
        ev["nodes"].as_array().unwrap().contains(&json!("db")),
        "{ev}"
    );
    assert_eq!(ev["failed_node"], Value::Null);
    assert_eq!(ev["kind"], Value::Null);
}

#[test]
fn built_in_route_needs_an_override_to_take_an_empty_pin() {
    let auth = |ep: &str| {
        format!(
            "version: 1
nodes:
  nginx: {{ type: gateway, proc: 1ms }}
  login: {{ type: auth, users: [ {{ username: a, password: b }} ] }}
edges:
  - {{ from: nginx, to: login, net: 1ms }}
endpoints:
{ep}scenarios:
  bad: {{ pins: [ {{ endpoint: \"POST /login\", state: empty }} ] }}
"
        )
    };
    // the built-in responder answers, so there is no empty form
    assert!(check_err(&auth("")).contains("no empty form"));
    // an `override:` replaces the responder, so the route is an override route: 204 has no body
    let dir = new_dir();
    let yaml = auth(
        "  \"POST /login\": { override: { status: 204 } }
",
    );
    let r = kaanal_server::check(&Config::from_yaml(&yaml).unwrap(), &dir);
    let _ = std::fs::remove_dir_all(&dir);
    assert!(r.is_ok(), "{:?}", r.err().map(|e| e.to_string()));
}

#[tokio::test]
async fn a_204_or_304_empty_pin_sends_no_body() {
    let s = start().await;
    // a pin status of 204 on a normally-200 route (304 is outside the 200-299 pin range)
    let r = s
        .pin(json!({"endpoint": "GET /users", "state": "empty", "status": 204}))
        .await;
    assert_eq!(r.status(), 200, "{}", r.text().await.unwrap());
    let r = s.get("/users").await;
    assert_eq!(r.status(), 204);
    let len = r.headers().get("content-length").cloned();
    assert!(len.as_ref().is_none_or(|l| l == "0"), "{len:?}");
    assert!(r.bytes().await.unwrap().is_empty());
    // a route that normally answers 304
    let yaml = format!(
        "{NODES}{ENDPOINTS}  \"GET /same\": {{ override: {{ status: 304 }} }}
{SCENARIOS}"
    )
    .replace(
        "p-204:",
        "p-same: { pins: [ { endpoint: \"GET /same\", state: empty } ] }
  p-204:",
    );
    let s = start_yaml(&yaml).await;
    let r = s.scoped("/same", "p-same").await;
    assert_eq!(r.status(), 304);
    let len = r.headers().get("content-length").cloned();
    assert!(len.as_ref().is_none_or(|l| l == "0"), "{len:?}");
    assert!(r.bytes().await.unwrap().is_empty());
}
