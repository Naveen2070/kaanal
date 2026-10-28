//! Shared helpers: a server over the paging fixture and request helpers.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

use kaanal_core::Config;
use kaanal_server::ServeOpts;
use serde_json::Value;

pub(crate) const NODES: &str = "version: 1
seed: 42
nodes:
  nginx: { type: gateway, proc: 1ms, timeout: 500ms }
  app:   { type: service, spec: openapi.yaml, proc: 1ms }
edges:
  - { from: nginx, to: app, net: 1ms }
";

pub(crate) const SCENARIOS: &str = "scenarios:
  items-503: { chaos: [ { endpoint: \"GET /items\", error_rate: 1.0, error_status: 503 } ] }
  p-empty: { pins: [ { endpoint: \"GET /items\", state: empty } ] }
  slow-app: { chaos: [ { node: app, latency_ms: 2 } ] }
  items-malformed: { chaos: [ { endpoint: \"GET /items\", error_rate: 1.0, error_kind: malformed } ] }
  items-reset: { chaos: [ { endpoint: \"GET /items\", error_rate: 1.0, error_kind: reset } ] }
  p-success: { pins: [ { endpoint: \"GET /items\", state: success } ] }
  p-slow: { pins: [ { endpoint: \"GET /items\", state: slow, delay: 1ms } ] }
";

pub(crate) struct Sc {
    pub(crate) base: String,
    pub(crate) token: String,
    pub(crate) client: reqwest::Client,
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
        "kaanal-paging-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::copy(fixture(), d.join("openapi.yaml")).unwrap();
    d
}

/// The paging spec fixture.
pub(crate) fn fixture() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/paging.yaml")
}

/// The warnings `kaanal validate` prints for `yaml` over the fixture.
pub(crate) fn check_yaml(yaml: &str) -> Vec<String> {
    let dir = new_dir();
    let cfg = Config::from_yaml(yaml).unwrap();
    let w = kaanal_server::check(&cfg, &dir).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    w
}

pub(crate) async fn start_yaml(yaml: &str) -> Sc {
    let dir = new_dir();
    let cfg = Config::from_yaml(yaml).unwrap();
    let token = kaanal_server::new_token();
    let opts = ServeOpts {
        host: "127.0.0.1".parse().unwrap(),
        port: 0,
        token: token.clone(),
        auth_key: None,
        draft: None,
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
    start_yaml(&format!("{NODES}{SCENARIOS}")).await
}

impl Sc {
    pub(crate) async fn get(&self, path: &str) -> reqwest::Response {
        self.client
            .get(format!("{}{path}", self.base))
            .send()
            .await
            .unwrap()
    }
    pub(crate) async fn json(&self, path: &str) -> Value {
        let r = self.get(path).await;
        assert_eq!(r.status(), 200, "{path}");
        r.json().await.unwrap()
    }
    pub(crate) async fn activate(&self, name: &str) {
        let r = self
            .client
            .post(format!("{}/__kaanal/scenarios/{name}/activate", self.base))
            .bearer_auth(&self.token)
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 200, "{name}");
    }
}

/// A response header as a string, `None` when absent.
pub(crate) fn header(r: &reqwest::Response, name: &str) -> Option<String> {
    r.headers()
        .get(name)
        .map(|v| v.to_str().unwrap().to_string())
}

/// `X-Total-Count` as a number.
pub(crate) fn total(r: &reqwest::Response) -> usize {
    header(r, "x-total-count").unwrap().parse().unwrap()
}
