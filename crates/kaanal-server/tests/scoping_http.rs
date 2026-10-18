//! Per-request scenario scoping: the `X-Kaanal-Scenario` header, its cache and its errors.

use kaanal_core::Config;
use kaanal_server::ServeOpts;
use reqwest::Method;
use reqwest::header::HeaderValue;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

const BASE: &str = "version: 1
seed: 42
nodes:
  nginx: { type: gateway, proc: 1ms }
  app:   { type: service, spec: openapi.yaml, proc: 1ms }
  db:    { type: database, flavor: postgres, ops: { read: 1ms, write: 1ms, scan: 1ms } }
edges:
  - { from: nginx, to: app, net: 1ms }
  - { from: app, to: db, net: 1ms }
";
const EP: &str = "/users/42";
const SCENARIOS: &str = "
scenarios:
  db-down: { chaos: [ { node: db, down: true } ] }
  a: { chaos: [ { node: db, error_rate: 1.0, error_status: 503 } ] }
  b: { chaos: [ { node: db, error_rate: 1.0, error_status: 504 } ] }
  flaky: { chaos: [ { node: db, error_rate: 0.5 } ] }
";

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
        "kaanal-sc-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let spec = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/openapi.yaml");
    std::fs::copy(spec, d.join("openapi.yaml")).unwrap();
    d
}

async fn start_with_scenarios(extra: &str) -> Sc {
    let dir = new_dir();
    let cfg = Config::from_yaml(&format!("{BASE}{extra}")).unwrap();
    let token = kaanal_server::new_token();
    let opts = ServeOpts {
        host: "127.0.0.1".parse().unwrap(),
        port: 0,
        token: token.clone(),
        auth_key: None,
    };
    let s = kaanal_server::serve(&cfg, &dir, opts).await.unwrap();
    Sc {
        base: format!("http://127.0.0.1:{}", s.addr.port()),
        token,
        client: reqwest::Client::new(),
        dir,
    }
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
    async fn get_scoped(&self, path: &str, header: &str) -> reqwest::Response {
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
    async fn scoped(&self, header: &str) -> u16 {
        self.get_scoped(EP, header).await.status().as_u16()
    }
    async fn post(&self, path: &str) -> reqwest::Response {
        self.send(Method::POST, path, None).await
    }
    async fn put(&self, path: &str, b: Value) -> reqwest::Response {
        self.send(Method::PUT, path, Some(b)).await
    }
    async fn delete(&self, path: &str) -> reqwest::Response {
        self.send(Method::DELETE, path, None).await
    }
    async fn reset(&self) {
        assert_eq!(self.post("/__kaanal/reset").await.status(), 204);
    }
    async fn define(&self, name: &str, chaos: Value) {
        let r = self
            .put(
                &format!("/__kaanal/scenarios/{name}"),
                json!({ "chaos": chaos }),
            )
            .await;
        assert!(r.status().is_success(), "{}", r.status());
    }
}

#[tokio::test]
async fn scoped_requests_get_their_own_chaos() {
    let s = start_with_scenarios(SCENARIOS).await;
    assert_eq!(s.scoped("db-down").await, 503);
    assert_eq!(s.scoped("").await, 200);
    assert_eq!(s.status(EP).await, 200);
    s.post("/__kaanal/scenarios/db-down/activate").await;
    assert_eq!(s.status(EP).await, 503);
    assert_eq!(s.scoped("").await, 200);
    assert_eq!(s.scoped("a").await, 503);
}

#[tokio::test]
async fn scoped_ignores_the_live_layer() {
    let s = start_with_scenarios(SCENARIOS).await;
    s.put("/__kaanal/chaos/db", json!({ "error_rate": 1 }))
        .await;
    assert_eq!(s.status(EP).await, 503);
    assert_eq!(s.scoped("").await, 200);
}

#[tokio::test]
async fn stack_order_is_header_order() {
    let s = start_with_scenarios(SCENARIOS).await;
    assert_eq!(s.scoped("a,b").await, 504);
    assert_eq!(s.scoped("b,a").await, 503);
    let r = s.get_scoped(EP, "a,b,a").await;
    assert_eq!(r.status(), 503);
    assert_eq!(r.headers()["x-kaanal-scope"], "b,a");
}

#[tokio::test]
async fn echo_header_and_cors() {
    let s = start_with_scenarios(SCENARIOS).await;
    let r = s.get_scoped(EP, "a").await;
    assert_eq!(r.headers()["x-kaanal-scope"], "a");
    assert!(
        r.headers()["access-control-expose-headers"]
            .to_str()
            .unwrap()
            .contains("x-kaanal-scope")
    );
    assert!(s.get(EP).await.headers().get("x-kaanal-scope").is_none());
    let r = s
        .client
        .request(Method::OPTIONS, format!("{}{EP}", s.base))
        .header("access-control-request-headers", "x-kaanal-scenario")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
    assert_eq!(r.headers()["access-control-allow-headers"], "*");
}

#[tokio::test]
async fn bad_scopes_are_400() {
    let s = start_with_scenarios(SCENARIOS).await;
    let many = (0..17)
        .map(|i| format!("s{i}"))
        .collect::<Vec<_>>()
        .join(",");
    let long = "a".repeat(257);
    let mut cases: Vec<Vec<HeaderValue>> =
        ["nope", "Bad!", "a,,b", "a,", many.as_str(), long.as_str()]
            .iter()
            .map(|v| vec![HeaderValue::from_str(v).unwrap()])
            .collect();
    cases.push(vec![HeaderValue::from_bytes(b"\xff").unwrap()]);
    cases.push(vec![
        HeaderValue::from_static("a"),
        HeaderValue::from_static("b"),
    ]);
    for (i, vals) in cases.into_iter().enumerate() {
        let mut rb = s.client.get(format!("{}{EP}", s.base));
        for v in vals {
            rb = rb.header("x-kaanal-scenario", v);
        }
        let r = rb.send().await.unwrap();
        assert_eq!(r.status(), 400, "case {i}");
        assert_eq!(r.headers()["access-control-allow-origin"], "*", "case {i}");
        let b: Value = r.json().await.unwrap();
        assert_eq!(b["error"], "bad scenario scope", "case {i}");
        if i == 0 {
            assert!(
                b["detail"]
                    .as_str()
                    .unwrap()
                    .contains("unknown scenario 'nope'")
            );
        }
    }
}

#[tokio::test]
async fn bad_scope_does_not_perturb_seeded_draws() {
    let s = start_with_scenarios(SCENARIOS).await;
    s.reset().await;
    s.put("/__kaanal/chaos/db", json!({ "error_rate": 0.5 }))
        .await;
    let mut v = Vec::new();
    for _ in 0..20 {
        v.push(s.status(EP).await);
    }
    s.reset().await;
    s.put("/__kaanal/chaos/db", json!({ "error_rate": 0.5 }))
        .await;
    let mut w = Vec::new();
    for _ in 0..20 {
        assert_eq!(s.scoped("nope").await, 400);
        w.push(s.status(EP).await);
    }
    assert_eq!(v, w);
}

#[tokio::test]
async fn control_namespace_ignores_the_header() {
    let s = start_with_scenarios(SCENARIOS).await;
    let r = s
        .client
        .get(format!("{}/__kaanal/health", s.base))
        .bearer_auth(&s.token)
        .header("x-kaanal-scenario", "nope")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
}

#[tokio::test]
async fn definition_changes_reach_the_cache() {
    let s = start_with_scenarios(SCENARIOS).await;
    let down = json!([{ "node": "db", "down": true }]);
    s.define("rt", down.clone()).await;
    assert_eq!(s.scoped("rt").await, 503);
    s.define("rt", json!([{ "node": "db", "latency_ms": 1 }]))
        .await;
    assert_eq!(s.scoped("rt").await, 200);
    assert_eq!(s.delete("/__kaanal/scenarios/rt").await.status(), 204);
    assert_eq!(s.scoped("rt").await, 400);
    s.define("rt", down).await;
    assert_eq!(s.scoped("rt").await, 503);
    s.reset().await;
    assert_eq!(s.scoped("rt").await, 400);
}

#[tokio::test]
async fn force_save_reaches_the_cache() {
    let s = start_with_scenarios(SCENARIOS).await;
    s.put("/__kaanal/chaos/db", json!({ "down": true })).await;
    let save = |force: bool| {
        s.client
            .post(format!("{}/__kaanal/scenarios/save", s.base))
            .bearer_auth(&s.token)
            .json(&json!({ "name": "rt2", "force": force }))
            .send()
    };
    assert_eq!(save(false).await.unwrap().status(), 200);
    assert_eq!(s.scoped("rt2").await, 503);
    assert_eq!(s.delete("/__kaanal/chaos/db").await.status(), 204);
    s.put("/__kaanal/chaos/db", json!({ "latency_ms": 1 }))
        .await;
    assert_eq!(save(true).await.unwrap().status(), 200);
    assert_eq!(s.scoped("rt2").await, 200);
}

#[tokio::test]
async fn scoped_miss_racing_put_scenario_never_serves_stale() {
    let s = Arc::new(start_with_scenarios(SCENARIOS).await);
    let down = json!([{ "node": "db", "down": true }]);
    let calm = json!([{ "node": "db", "latency_ms": 1 }]);
    s.define("rt", calm.clone()).await;
    let mut tasks = Vec::new();
    for _ in 0..4 {
        let s = Arc::clone(&s);
        tasks.push(tokio::spawn(async move {
            for _ in 0..60 {
                s.scoped("rt").await;
            }
        }));
    }
    for k in 0..30 {
        let (def, want) = if k % 2 == 0 {
            (&down, 503)
        } else {
            (&calm, 200)
        };
        s.define("rt", def.clone()).await;
        assert_eq!(s.scoped("rt").await, want, "round {k}");
    }
    for t in tasks {
        t.await.unwrap();
    }
}

#[tokio::test]
async fn scoped_equals_global_activation() {
    let s = start_with_scenarios(SCENARIOS).await;
    s.reset().await;
    s.post("/__kaanal/scenarios/flaky/activate").await;
    let mut v = Vec::new();
    for _ in 0..20 {
        v.push(s.status(EP).await);
    }
    s.reset().await;
    let mut w = Vec::new();
    for _ in 0..20 {
        w.push(s.scoped("flaky").await);
    }
    assert_eq!(v, w);
}

#[tokio::test]
async fn parallel_workers_do_not_interfere() {
    let s = Arc::new(start_with_scenarios(SCENARIOS).await);
    let mut tasks = Vec::new();
    for i in 0..8 {
        let s = Arc::clone(&s);
        tasks.push(tokio::spawn(async move {
            let (header, want) = if i % 2 == 0 {
                ("db-down", 503)
            } else {
                ("", 200)
            };
            for _ in 0..25 {
                assert_eq!(s.scoped(header).await, want);
            }
        }));
    }
    for k in 0..20 {
        let verb = if k % 2 == 0 { "activate" } else { "deactivate" };
        s.post(&format!("/__kaanal/scenarios/a/{verb}")).await;
    }
    for t in tasks {
        t.await.unwrap();
    }
}

#[tokio::test]
async fn more_than_the_cache_bound_still_answers_right() {
    let s = start_with_scenarios("").await;
    for i in 0..17 {
        let chaos = if i == 0 {
            json!([{ "node": "db", "down": true }])
        } else {
            json!([{ "node": "db", "latency_ms": 1 }])
        };
        s.define(&format!("s{i:02}"), chaos).await;
    }
    for i in 0..17 {
        for j in 0..17 {
            if i == j {
                continue;
            }
            let want = if i == 0 || j == 0 { 503 } else { 200 };
            assert_eq!(s.scoped(&format!("s{i:02},s{j:02}")).await, want, "{i},{j}");
        }
    }
}
