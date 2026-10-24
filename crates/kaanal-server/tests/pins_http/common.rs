//! Shared helpers: a pins server, its config pieces and request helpers.

use std::path::{Path, PathBuf};

use std::sync::atomic::{AtomicU32, Ordering};

use kaanal_core::Config;

use kaanal_server::ServeOpts;

use reqwest::Method;

use serde_json::Value;

pub(crate) const NODES: &str = "version: 1
seed: 42
nodes:
  nginx: { type: gateway, proc: 1ms, timeout: 500ms }
  app:   { type: service, spec: openapi.yaml, proc: 1ms }
  db:    { type: database, flavor: postgres, ops: { read: 1ms, write: 1ms, scan: 1ms } }
edges:
  - { from: nginx, to: app, net: 1ms }
  - { from: app, to: db, net: 1ms }
";

pub(crate) const ENDPOINTS: &str = "endpoints:
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

pub(crate) const SCENARIOS: &str = "scenarios:
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

pub(crate) const USER: &str = "/users/7";

pub(crate) struct Sc {
    pub(crate) base: String,
    pub(crate) token: String,
    pub(crate) client: reqwest::Client,
    pub(crate) dir: PathBuf,
}

impl Drop for Sc {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

pub(crate) fn new_dir() -> PathBuf {
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

pub(crate) async fn start_yaml(yaml: &str) -> Sc {
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

pub(crate) async fn start() -> Sc {
    start_yaml(&format!("{NODES}{ENDPOINTS}{SCENARIOS}")).await
}

impl Sc {
    pub(crate) async fn send(
        &self,
        m: Method,
        path: &str,
        body: Option<Value>,
    ) -> reqwest::Response {
        let mut rb = self
            .client
            .request(m, format!("{}{path}", self.base))
            .bearer_auth(&self.token);
        if let Some(b) = body {
            rb = rb.json(&b);
        }
        rb.send().await.unwrap()
    }
    pub(crate) async fn get(&self, path: &str) -> reqwest::Response {
        self.client
            .get(format!("{}{path}", self.base))
            .send()
            .await
            .unwrap()
    }
    pub(crate) async fn scoped(&self, path: &str, header: &str) -> reqwest::Response {
        self.client
            .get(format!("{}{path}", self.base))
            .header("x-kaanal-scenario", header)
            .send()
            .await
            .unwrap()
    }
    pub(crate) async fn status(&self, path: &str) -> u16 {
        self.get(path).await.status().as_u16()
    }
    pub(crate) async fn activate(&self, name: &str) {
        let r = self
            .send(
                Method::POST,
                &format!("/__kaanal/scenarios/{name}/activate"),
                None,
            )
            .await;
        assert_eq!(r.status(), 200, "{name}");
    }
    pub(crate) async fn deactivate(&self, name: &str) {
        let r = self
            .send(
                Method::POST,
                &format!("/__kaanal/scenarios/{name}/deactivate"),
                None,
            )
            .await;
        assert_eq!(r.status(), 204, "{name}");
    }
    pub(crate) async fn reset(&self) {
        let r = self.send(Method::POST, "/__kaanal/reset", None).await;
        assert_eq!(r.status(), 204);
    }
}

pub(crate) fn trace(r: &reqwest::Response) -> String {
    r.headers()["x-kaanal-trace"].to_str().unwrap().to_string()
}

pub(crate) const EP: &str = "GET /users/{id}";

pub(crate) const EP_Q: &str = "/__kaanal/pins?endpoint=GET%20/users/%7Bid%7D";

pub(crate) fn check_err(yaml: &str) -> String {
    let dir = new_dir();
    let r = kaanal_server::check(&Config::from_yaml(yaml).unwrap(), &dir);
    let _ = std::fs::remove_dir_all(&dir);
    r.unwrap_err().to_string()
}
