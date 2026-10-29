//! Shared helpers: a server started from a `kaanal.yaml` text that is also its draft.

use kaanal_core::Config;
use kaanal_server::{DraftSource, ServeOpts};
use reqwest::Method;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

pub(crate) const BASE: &str = "version: 1
seed: 42
# a comment the draft must keep
nodes:
  nginx: { type: gateway, proc: 1ms }
  app:   { type: service, proc: 1ms }
  db:    { type: database, ops: { read: 1ms, write: 1ms, scan: 1ms } }
edges:
  - { from: nginx, to: app, net: 1ms }
  - { from: app, to: db, net: 1ms }
";

pub(crate) struct Srv {
    pub(crate) base: String,
    pub(crate) token: String,
    pub(crate) client: reqwest::Client,
    pub(crate) dir: PathBuf,
}

impl Drop for Srv {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// A server for `cfg` whose draft is `dir/kaanal.yaml` holding `text` (written first unless it is
/// already there); `root` is what is removed when the server goes (`dir` or a folder above it).
pub(crate) async fn start_at(root: PathBuf, dir: &std::path::Path, cfg: &str, text: &str) -> Srv {
    let file = dir.join("kaanal.yaml");
    if !file.exists() {
        std::fs::write(&file, text).unwrap();
    }
    let token = kaanal_server::new_token();
    let opts = ServeOpts {
        host: "127.0.0.1".parse().unwrap(),
        port: 0,
        token: token.clone(),
        auth_key: None,
        draft: Some(DraftSource {
            file,
            text: text.to_string(),
        }),
    };
    let s = kaanal_server::serve(&Config::from_yaml(cfg).unwrap(), dir, opts)
        .await
        .unwrap();
    Srv {
        base: format!("http://127.0.0.1:{}", s.addr.port()),
        token,
        client: reqwest::Client::new(),
        dir: root,
    }
}

pub(crate) fn new_dir() -> PathBuf {
    static N: AtomicU32 = AtomicU32::new(0);
    let d = std::env::temp_dir().join(format!(
        "kaanal-dr-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A server with no system (a blank start) in a fresh folder that holds the fixture spec.
pub(crate) async fn start_blank() -> Srv {
    let dir = new_dir();
    let fixture =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/openapi.yaml");
    std::fs::copy(fixture, dir.join("openapi.yaml")).unwrap();
    start_blank_in(dir).await
}

/// A blank start in `dir`, which holds whatever the caller put there (the fresh-project case: nothing).
pub(crate) async fn start_blank_in(dir: PathBuf) -> Srv {
    let token = kaanal_server::new_token();
    let opts = ServeOpts {
        host: "127.0.0.1".parse().unwrap(),
        port: 0,
        token: token.clone(),
        // a blank start has no config to say whether an auth node will come, so `up` always gives a key
        auth_key: Some(kaanal_server::AuthKey::generate().unwrap()),
        draft: None,
    };
    let s = kaanal_server::serve_blank(&dir, &dir.join("kaanal.yaml"), opts)
        .await
        .unwrap();
    Srv {
        base: format!("http://127.0.0.1:{}", s.addr.port()),
        token,
        client: reqwest::Client::new(),
        dir,
    }
}

/// A server for `yaml`, with `yaml` as its draft (as `kaanal up` does with the file's text).
pub(crate) async fn start(yaml: &str) -> Srv {
    start_with(yaml, Some(yaml)).await
}

/// A server for `yaml` whose draft source is `draft` (`None`: no source, as in embedded use).
pub(crate) async fn start_with(yaml: &str, draft: Option<&str>) -> Srv {
    let dir = new_dir();
    let fixture =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/openapi.yaml");
    std::fs::copy(fixture, dir.join("openapi.yaml")).unwrap();
    std::fs::write(dir.join("extra.yaml"), EXTRA_SPEC).unwrap();
    if let Some(t) = draft {
        std::fs::write(dir.join("kaanal.yaml"), t).unwrap(); // the file `up` read the draft from
    }
    let cfg = Config::from_yaml(yaml).unwrap();
    let token = kaanal_server::new_token();
    let opts = ServeOpts {
        host: "127.0.0.1".parse().unwrap(),
        port: 0,
        token: token.clone(),
        auth_key: None,
        draft: draft.map(|t| DraftSource {
            file: dir.join("kaanal.yaml"),
            text: t.to_string(),
        }),
    };
    let s = kaanal_server::serve(&cfg, &dir, opts).await.unwrap();
    Srv {
        base: format!("http://127.0.0.1:{}", s.addr.port()),
        token,
        client: reqwest::Client::new(),
        dir,
    }
}

impl Srv {
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
        self.send(Method::GET, path, None).await
    }
    pub(crate) async fn put(&self, path: &str, b: Value) -> reqwest::Response {
        self.send(Method::PUT, path, Some(b)).await
    }
}

/// A gateway, an `app` service with the fixture spec and `proc` as its processing time, and a
/// database: `GET /users/{id}` is served, through the chain `app -> db`.
pub(crate) fn system(proc: &str) -> String {
    format!(
        "version: 1
seed: 42
nodes:
  nginx: {{ type: gateway, proc: 1ms }}
  app:   {{ type: service, spec: openapi.yaml, proc: {proc} }}
  db:    {{ type: database, ops: {{ read: 1ms, write: 1ms, scan: 1ms }} }}
edges:
  - {{ from: nginx, to: app, net: 1ms }}
  - {{ from: app, to: db, net: 1ms }}
"
    )
}

/// A second spec with one route, `GET /ping`.
pub(crate) const EXTRA_SPEC: &str = "openapi: 3.0.0
info: { title: extra, version: '1' }
paths:
  /ping:
    get:
      responses:
        '200':
          description: ok
          content:
            application/json:
              schema: { type: object, properties: { pong: { type: boolean } } }
";
