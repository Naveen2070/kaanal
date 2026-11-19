//! Shared helpers: a scenario server in a scratch folder, its start variants and request helpers.

use kaanal_core::Config;

use kaanal_server::ServeOpts;

use reqwest::Method;

use serde_json::Value;

use std::path::{Path, PathBuf};

use std::sync::atomic::{AtomicU32, Ordering};

pub(crate) const BASE: &str = "version: 1
seed: 42
nodes:
  nginx: { type: gateway, proc: 1ms }
  app:   { type: service, spec: openapi.yaml, proc: 1ms }
  db:    { type: database, flavor: postgres, ops: { read: 1ms, write: 1ms, scan: 1ms } }
edges:
  - { from: nginx, to: app, net: 1ms }
  - { from: app, to: db, net: 1ms }
";

pub(crate) const EP: &str = "GET /users/{id}";

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

pub(crate) fn new_dir(files: &[(&str, &str)]) -> PathBuf {
    static N: AtomicU32 = AtomicU32::new(0);
    let d = std::env::temp_dir().join(format!(
        "kaanal-sh-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let spec = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/openapi.yaml");
    std::fs::copy(spec, d.join("openapi.yaml")).unwrap();
    for (f, body) in files {
        let p = d.join("scenarios").join(f);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }
    d
}

pub(crate) async fn start_in(dir: PathBuf, extra: &str) -> Sc {
    start_full(dir, &format!("{BASE}{extra}")).await
}

/// A server for a whole `kaanal.yaml`.
pub(crate) async fn start_full(dir: PathBuf, yaml: &str) -> Sc {
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

/// A server whose config carries `extra` (usually a `scenarios:` section).
pub(crate) async fn start_with_scenarios(extra: &str) -> Sc {
    start_in(new_dir(&[]), extra).await
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
    /// A mock request (no token needed).
    pub(crate) async fn get(&self, path: &str) -> reqwest::Response {
        self.client
            .get(format!("{}{path}", self.base))
            .send()
            .await
            .unwrap()
    }
    pub(crate) async fn ctl(&self, path: &str) -> reqwest::Response {
        self.send(Method::GET, path, None).await
    }
    pub(crate) async fn post(&self, path: &str) -> reqwest::Response {
        self.send(Method::POST, path, None).await
    }
    pub(crate) async fn post_json(&self, path: &str, b: Value) -> reqwest::Response {
        self.send(Method::POST, path, Some(b)).await
    }
    pub(crate) async fn put(&self, path: &str, b: Value) -> reqwest::Response {
        self.send(Method::PUT, path, Some(b)).await
    }
    pub(crate) async fn delete(&self, path: &str) -> reqwest::Response {
        self.send(Method::DELETE, path, None).await
    }
    pub(crate) async fn put_chaos(&self, node: &str, b: Value) -> reqwest::Response {
        self.put(&format!("/__kaanal/chaos/{node}"), b).await
    }
    pub(crate) async fn status(&self, path: &str) -> u16 {
        self.get(path).await.status().as_u16()
    }
    pub(crate) async fn active(&self) -> Value {
        self.ctl("/__kaanal/scenarios/active")
            .await
            .json()
            .await
            .unwrap()
    }
    pub(crate) async fn live(&self) -> Value {
        self.ctl("/__kaanal/chaos").await.json().await.unwrap()
    }
    pub(crate) fn files(&self) -> Vec<String> {
        let mut v: Vec<String> =
            std::fs::read_dir(self.dir.join("scenarios")).map_or_default(|rd| {
                rd.map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                    .collect()
            });
        v.sort();
        v
    }
}
