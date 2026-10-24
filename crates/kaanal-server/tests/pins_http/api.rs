//! The pins API and `cycle`: put, get, delete, bad input, the five states, scoping and concurrency.

use crate::common::*;

use reqwest::Method;

use serde_json::{Value, json};

impl Sc {
    pub(crate) async fn pin(&self, body: Value) -> reqwest::Response {
        self.send(Method::PUT, "/__kaanal/pins", Some(body)).await
    }
    pub(crate) async fn cycle(&self, body: Value) -> reqwest::Response {
        self.send(Method::POST, "/__kaanal/cycle", Some(body)).await
    }
    pub(crate) async fn pins(&self) -> Value {
        self.send(Method::GET, "/__kaanal/pins", None)
            .await
            .json()
            .await
            .unwrap()
    }
}

#[tokio::test]
async fn put_get_delete_pins_round_trip() {
    let s = start().await;
    let r = s
        .pin(json!({"endpoint": "get /users/{id}", "state": "error", "status": 503, "delay": "150ms"}))
        .await;
    assert_eq!(r.status(), 200, "{}", r.text().await.unwrap());
    let g = s.pins().await;
    assert_eq!(
        g["live"],
        json!([{"endpoint": EP, "state": "error", "status": 503, "delay_ms": 150}])
    );
    assert_eq!(
        g["effective"][EP],
        json!({"state": "error", "status": 503, "delay_ms": 150, "source": "live"})
    );
    assert_eq!(s.status(USER).await, 503);
    assert_eq!(s.send(Method::DELETE, EP_Q, None).await.status(), 204);
    // absent is 204 too
    assert_eq!(s.send(Method::DELETE, EP_Q, None).await.status(), 204);
    assert_eq!(s.pins().await["live"], json!([]));
    assert_eq!(s.status(USER).await, 200);
}

#[tokio::test]
async fn delete_without_query_clears_all_live_pins() {
    let s = start().await;
    s.activate("p-err").await;
    assert_eq!(
        s.pin(json!({"endpoint": EP, "state": "error"}))
            .await
            .status(),
        200
    );
    assert_eq!(
        s.pin(json!({"endpoint": "POST /orders", "state": "empty"}))
            .await
            .status(),
        200
    );
    assert_eq!(s.pins().await["live"].as_array().unwrap().len(), 2);
    let r = s.send(Method::DELETE, "/__kaanal/pins", None).await;
    assert_eq!(r.status(), 204);
    let g = s.pins().await;
    assert_eq!(g["live"], json!([]));
    // scenario pins stay
    assert_eq!(
        g["effective"]["GET /users"]["source"],
        json!({"scenario": "p-err"})
    );
}

#[tokio::test]
async fn bad_pins_are_400_or_404_and_change_nothing() {
    let s = start().await;
    let bad = [
        (json!({"endpoint": "GET /nope", "state": "error"}), 404),
        (
            json!({"endpoint": "HEAD /users/{id}", "state": "error"}),
            400,
        ),
        (json!({"endpoint": "nonsense", "state": "error"}), 400),
        (json!({"endpoint": EP, "state": "bogus"}), 400),
        (
            json!({"endpoint": EP, "state": "loading", "status": 500}),
            400,
        ),
        (
            json!({"endpoint": EP, "state": "slow", "delay": "6min"}),
            400,
        ),
        (
            json!({"endpoint": EP, "state": "slow", "delay": "soon"}),
            400,
        ),
        (json!({"endpoint": "GET /nothing", "state": "empty"}), 400),
    ];
    for (b, code) in bad {
        assert_eq!(s.pin(b.clone()).await.status(), code, "{b}");
        assert_eq!(s.cycle(b.clone()).await.status(), code, "cycle {b}");
    }
    let g = s.pins().await;
    assert_eq!(g["live"], json!([]));
    assert_eq!(g["effective"], json!({}));
    // a failed write keeps the pin that was there
    s.pin(json!({"endpoint": EP, "state": "error"})).await;
    let r = s
        .pin(json!({"endpoint": EP, "state": "slow", "delay": "6min"}))
        .await;
    assert_eq!(r.status(), 400);
    assert_eq!(s.pins().await["live"][0]["state"], "error");
}

#[tokio::test]
async fn cycle_walks_the_five_states_and_wraps() {
    let s = start().await;
    let mut seen = Vec::new();
    for _ in 0..6 {
        let r = s.cycle(json!({"endpoint": EP})).await;
        assert_eq!(r.status(), 200);
        let b: Value = r.json().await.unwrap();
        assert_eq!(b["endpoint"], EP);
        seen.push(b["state"].as_str().unwrap().to_string());
    }
    assert_eq!(
        seen,
        ["loading", "empty", "error", "slow", "success", "loading"]
    );
    assert_eq!(s.pins().await["live"][0]["state"], "loading");
}

#[tokio::test]
async fn cycle_skips_empty_on_a_built_in_route() {
    let auth = "version: 1
nodes:
  nginx: { type: gateway, proc: 1ms }
  login: { type: auth, users: [ { username: a, password: b } ] }
edges:
  - { from: nginx, to: login, net: 1ms }
";
    let s = start_yaml(auth).await;
    let mut seen = Vec::new();
    for _ in 0..5 {
        let b: Value = s
            .cycle(json!({"endpoint": "POST /login"}))
            .await
            .json()
            .await
            .unwrap();
        seen.push(b["state"].as_str().unwrap().to_string());
    }
    assert_eq!(seen, ["loading", "error", "slow", "success", "loading"]);
    let r = s
        .cycle(json!({"endpoint": "POST /login", "state": "empty"}))
        .await;
    assert_eq!(r.status(), 400);
}

#[tokio::test]
async fn cycle_with_state_pins_it_and_with_invalid_status_is_400() {
    let s = start().await;
    let b: Value = s
        .cycle(json!({"endpoint": EP, "state": "error", "status": 429, "delay": "1s"}))
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(
        b,
        json!({"endpoint": EP, "state": "error", "status": 429, "delay_ms": 1000})
    );
    assert_eq!(s.status(USER).await, 429);
    let r = s
        .cycle(json!({"endpoint": EP, "state": "error", "status": 200}))
        .await;
    assert_eq!(r.status(), 400);
    // no state: the next after error is slow, which takes no status
    let r = s.cycle(json!({"endpoint": EP, "status": 500})).await;
    assert_eq!(r.status(), 400);
    let g = s.pins().await;
    assert_eq!(g["live"][0]["state"], "error");
    assert_eq!(g["live"][0]["status"], 429);
}

#[tokio::test]
async fn live_pin_masks_a_scenario_pin_and_off_restores_it() {
    let s = start().await;
    s.activate("p-503").await;
    assert_eq!(s.status(USER).await, 503);
    s.pin(json!({"endpoint": EP, "state": "success"})).await;
    assert_eq!(s.status(USER).await, 200);
    assert_eq!(s.pins().await["effective"][EP]["source"], "live");
    s.send(Method::DELETE, EP_Q, None).await;
    assert_eq!(s.status(USER).await, 503);
    assert_eq!(
        s.pins().await["effective"][EP]["source"],
        json!({"scenario": "p-503"})
    );
}

#[tokio::test]
async fn reset_clears_live_pins() {
    let s = start().await;
    s.pin(json!({"endpoint": EP, "state": "error"})).await;
    assert_eq!(s.status(USER).await, 404);
    s.reset().await;
    assert_eq!(s.pins().await["live"], json!([]));
    assert_eq!(s.status(USER).await, 200);
}

#[tokio::test]
async fn live_cycle_does_not_reach_a_scoped_request() {
    let s = start().await;
    s.cycle(json!({"endpoint": EP, "state": "error"})).await;
    assert_eq!(s.status(USER).await, 404);
    // a scoped request sees only its scenario's pins
    assert_eq!(s.scoped(USER, "p-503").await.status(), 503);
    assert_eq!(s.scoped(USER, "p-success").await.status(), 200);
    // no pins in db-down: the chain runs and fails as the scenario says, not as the live pin
    assert_eq!(s.scoped(USER, "db-down").await.status(), 502);
    // a cycle step after the scoped cache filled is not stale for plain requests
    s.cycle(json!({"endpoint": EP, "state": "success"})).await;
    assert_eq!(s.status(USER).await, 200);
    assert_eq!(s.scoped(USER, "p-503").await.status(), 503);
}

#[tokio::test]
async fn concurrent_cycles_on_two_endpoints_both_land() {
    let s = start().await;
    for _ in 0..20 {
        s.reset().await;
        let (a, b) = tokio::join!(
            s.cycle(json!({"endpoint": EP})),
            s.cycle(json!({"endpoint": "POST /orders"}))
        );
        assert_eq!(a.status(), 200);
        assert_eq!(b.status(), 200);
        let live = s.pins().await["live"].clone();
        assert_eq!(live.as_array().unwrap().len(), 2, "{live}");
    }
}
