//! Config-load rules for pins, pins-only scenarios and scenario ttl expiry of pins.

use crate::common::*;

use std::time::{Duration, Instant};

use kaanal_core::Config;

use kaanal_server::ServeOpts;

use reqwest::Method;

use serde_json::{Value, json};

#[tokio::test]
async fn load_rejects_empty_pin_with_no_form_and_pin_on_head_and_unknown_endpoint() {
    let cases = [
        ("GET /nothing", "empty", "no empty form"),
        ("HEAD /users/{id}", "loading", "pin the GET route"),
        ("GET /nope", "loading", "unknown endpoint"),
    ];
    for (ep, state, want) in cases {
        let yaml = format!(
            "{NODES}{ENDPOINTS}scenarios:\n  bad: {{ pins: [ {{ endpoint: \"{ep}\", state: {state} }} ] }}\n"
        );
        let e = check_err(&yaml);
        assert!(e.contains(want), "{ep}: {e}");
        // and serve refuses it the same way
        let dir = new_dir();
        let opts = ServeOpts {
            host: "127.0.0.1".parse().unwrap(),
            port: 0,
            token: kaanal_server::new_token(),
            auth_key: None,
        };
        let r = kaanal_server::serve(&Config::from_yaml(&yaml).unwrap(), &dir, opts).await;
        assert!(r.is_err(), "{ep}");
        let _ = std::fs::remove_dir_all(&dir);
    }
    // the same empty pin on a route with a form loads
    let ok = format!(
        "{NODES}{ENDPOINTS}scenarios:\n  good: {{ pins: [ {{ endpoint: \"GET /users\", state: empty }} ] }}\n"
    );
    let dir = new_dir();
    assert!(kaanal_server::check(&Config::from_yaml(&ok).unwrap(), &dir).is_ok());
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test]
async fn pins_only_scenario_activates_and_deactivates() {
    let s = start().await;
    let r = s
        .send(
            Method::PUT,
            "/__kaanal/scenarios/rt",
            Some(json!({"pins": [{"endpoint": "GET /users", "state": "error", "status": 200 }]})),
        )
        .await;
    assert_eq!(r.status(), 400, "{}", r.text().await.unwrap());
    let r = s
        .send(
            Method::PUT,
            "/__kaanal/scenarios/rt",
            Some(json!({"pins": [
                {"endpoint": "GET /users", "state": "error", "status": 404, "delay": "100ms"},
                {"endpoint": "GET /nothing", "state": "loading"}
            ]})),
        )
        .await;
    assert_eq!(r.status(), 200);
    let r = s
        .send(
            Method::PUT,
            "/__kaanal/scenarios/rt",
            Some(json!({"pins": [{"endpoint": "GET /users", "state": "error", "status": 404, "delay": "100ms"}]})),
        )
        .await;
    assert_eq!(r.status(), 200, "{}", r.text().await.unwrap());
    // an empty pin with no form is a 400 here too
    let r = s
        .send(
            Method::PUT,
            "/__kaanal/scenarios/rt2",
            Some(json!({"pins": [{"endpoint": "GET /nothing", "state": "empty"}]})),
        )
        .await;
    assert_eq!(r.status(), 400);
    let g: Value = s
        .send(Method::GET, "/__kaanal/scenarios/rt", None)
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(g["pins"][0]["endpoint"], "GET /users");
    assert_eq!(g["pins"][0]["state"], "error");
    assert_eq!(g["pins"][0]["status"], 404);

    assert_eq!(s.status("/users").await, 200);
    s.activate("rt").await;
    assert_eq!(s.status("/users").await, 404);
    s.deactivate("rt").await;
    assert_eq!(s.status("/users").await, 200);
}

#[tokio::test]
async fn scenario_ttl_expires_its_pins() {
    let s = start().await;
    let r = s
        .send(Method::POST, "/__kaanal/scenarios/p-err/activate", None)
        .await;
    assert_eq!(r.status(), 200);
    s.deactivate("p-err").await;
    let r = s
        .client
        .post(format!("{}/__kaanal/scenarios/p-err/activate", s.base))
        .bearer_auth(&s.token)
        .header("content-type", "application/json")
        .body(r#"{"ttl":"300ms"}"#)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(s.status("/users").await, 500);
    let t = Instant::now();
    loop {
        if s.status("/users").await == 200 {
            break;
        }
        assert!(t.elapsed() < Duration::from_secs(3), "pin never expired");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
