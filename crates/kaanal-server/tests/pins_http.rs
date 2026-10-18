//! UI-state pins on the data path: load, snapshot and the mock front door.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

use kaanal_core::Config;
use kaanal_server::ServeOpts;
use reqwest::Method;
use serde_json::{Value, json};

const NODES: &str = "version: 1
seed: 42
nodes:
  nginx: { type: gateway, proc: 1ms, timeout: 500ms }
  app:   { type: service, spec: openapi.yaml, proc: 1ms }
  db:    { type: database, flavor: postgres, ops: { read: 1ms, write: 1ms, scan: 1ms } }
edges:
  - { from: nginx, to: app, net: 1ms }
  - { from: app, to: db, net: 1ms }
";
const ENDPOINTS: &str = "endpoints:
  \"GET /users/{id}\":
    error_map: { \"5xx\": 502 }
    states:
      error: { status: 404, body: { error: gone } }
      empty: { body: { id: keep } }
  \"POST /orders\":
    states:
      empty: { body: { orders: [] } }
  \"GET /health\": { override: { body: { ok: true, items: [1, 2], total: 5 } } }
  \"GET /gone\":
    override: { status: 204 }
  \"GET /nothing\": { override: {} }
";
const SCENARIOS: &str = "scenarios:
  db-down: { chaos: [ { node: db, down: true } ] }
  flaky: { chaos: [ { node: db, error_rate: 0.5 } ] }
  hang: { chaos: [ { endpoint: \"GET /users/{id}\", error_rate: 1.0, error_kind: hang } ] }
  p-loading: { pins: [ { endpoint: \"GET /users/{id}\", state: loading, delay: 1200ms } ] }
  p-slow: { pins: [ { endpoint: \"GET /users/{id}\", state: slow, delay: 300ms } ] }
  p-loading-short: { pins: [ { endpoint: \"GET /users/{id}\", state: loading, delay: 300ms } ] }
  p-err: { pins: [ { endpoint: \"GET /users\", state: error } ] }
  p-err-states: { pins: [ { endpoint: \"GET /users/{id}\", state: error } ] }
  p-429: { pins: [ { endpoint: \"GET /users\", state: error, status: 429 } ] }
  p-503: { pins: [ { endpoint: \"GET /users/{id}\", state: error, status: 503 } ] }
  p-empty: { pins: [ { endpoint: \"GET /users\", state: empty }, { endpoint: \"POST /orders\", state: empty } ] }
  p-204: { pins: [ { endpoint: \"GET /gone\", state: empty } ] }
  p-static: { pins: [ { endpoint: \"GET /health\", state: empty } ] }
  p-empty-user: { pins: [ { endpoint: \"GET /users/{id}\", state: empty } ] }
  app-reset: { chaos: [ { node: app, error_rate: 1.0, error_kind: reset } ] }
  app-malformed: { chaos: [ { node: app, error_rate: 1.0, error_kind: malformed } ] }
  p-success: { pins: [ { endpoint: \"GET /users/{id}\", state: success } ] }
";
const USER: &str = "/users/7";

struct Sc {
    base: String,
    token: String,
    client: reqwest::Client,
    dir: PathBuf,
}

impl Drop for Sc {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn new_dir() -> PathBuf {
    static N: AtomicU32 = AtomicU32::new(0);
    let d = std::env::temp_dir().join(format!(
        "kaanal-pins-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let spec = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/openapi.yaml");
    std::fs::copy(spec, d.join("openapi.yaml")).unwrap();
    d
}

async fn start_yaml(yaml: &str) -> Sc {
    let dir = new_dir();
    let cfg = Config::from_yaml(yaml).unwrap();
    let token = kaanal_server::new_token();
    let opts = ServeOpts {
        host: "127.0.0.1".parse().unwrap(),
        port: 0,
        token: token.clone(),
        // An auth node needs a key to start; the built-in-route cycle test has one.
        auth_key: Some(kaanal_server::AuthKey::generate().unwrap()),
    };
    let s = kaanal_server::serve(&cfg, &dir, opts).await.unwrap();
    Sc {
        base: format!("http://127.0.0.1:{}", s.addr.port()),
        token,
        client: reqwest::Client::new(),
        dir,
    }
}

async fn start() -> Sc {
    start_yaml(&format!("{NODES}{ENDPOINTS}{SCENARIOS}")).await
}

impl Sc {
    async fn send(&self, m: Method, path: &str, body: Option<Value>) -> reqwest::Response {
        let mut rb = self
            .client
            .request(m, format!("{}{path}", self.base))
            .bearer_auth(&self.token);
        if let Some(b) = body {
            rb = rb.json(&b);
        }
        rb.send().await.unwrap()
    }
    async fn get(&self, path: &str) -> reqwest::Response {
        self.client
            .get(format!("{}{path}", self.base))
            .send()
            .await
            .unwrap()
    }
    async fn scoped(&self, path: &str, header: &str) -> reqwest::Response {
        self.client
            .get(format!("{}{path}", self.base))
            .header("x-kaanal-scenario", header)
            .send()
            .await
            .unwrap()
    }
    async fn status(&self, path: &str) -> u16 {
        self.get(path).await.status().as_u16()
    }
    async fn activate(&self, name: &str) {
        let r = self
            .send(
                Method::POST,
                &format!("/__kaanal/scenarios/{name}/activate"),
                None,
            )
            .await;
        assert_eq!(r.status(), 200, "{name}");
    }
    async fn deactivate(&self, name: &str) {
        let r = self
            .send(
                Method::POST,
                &format!("/__kaanal/scenarios/{name}/deactivate"),
                None,
            )
            .await;
        assert_eq!(r.status(), 204, "{name}");
    }
    async fn reset(&self) {
        let r = self.send(Method::POST, "/__kaanal/reset", None).await;
        assert_eq!(r.status(), 204);
    }
}

fn trace(r: &reqwest::Response) -> String {
    r.headers()["x-kaanal-trace"].to_str().unwrap().to_string()
}

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

fn check_err(yaml: &str) -> String {
    let dir = new_dir();
    let r = kaanal_server::check(&Config::from_yaml(yaml).unwrap(), &dir);
    let _ = std::fs::remove_dir_all(&dir);
    r.unwrap_err().to_string()
}

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

const EP: &str = "GET /users/{id}";
const EP_Q: &str = "/__kaanal/pins?endpoint=GET%20/users/%7Bid%7D";

impl Sc {
    async fn pin(&self, body: Value) -> reqwest::Response {
        self.send(Method::PUT, "/__kaanal/pins", Some(body)).await
    }
    async fn cycle(&self, body: Value) -> reqwest::Response {
        self.send(Method::POST, "/__kaanal/cycle", Some(body)).await
    }
    async fn pins(&self) -> Value {
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
