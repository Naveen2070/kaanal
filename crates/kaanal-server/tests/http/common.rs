//! Shared helpers: starting a server on an OS-chosen port with a config, a seed or a signing key.

use kaanal_core::Config;

use kaanal_server::ServeOpts;

use serde_json::json;

use std::path::{Path, PathBuf};

pub(crate) struct Srv {
    pub base: String,
    pub token: String,
    pub client: reqwest::Client,
    pub drawn_seed: Option<u64>,
}

pub async fn start() -> Srv {
    start_seeded(None).await
}

pub(crate) fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// One signing key per test binary (spec section 7); every server gets a clone.
pub(crate) fn test_key() -> kaanal_server::AuthKey {
    static KEY: std::sync::OnceLock<kaanal_server::AuthKey> = std::sync::OnceLock::new();
    KEY.get_or_init(|| kaanal_server::AuthKey::generate().unwrap())
        .clone()
}

/// Serves `cfg` with spec paths resolved against `dir`.
pub async fn start_in(cfg: &Config, dir: &Path) -> Srv {
    let token = kaanal_server::new_token();
    let opts = ServeOpts {
        host: "127.0.0.1".parse().unwrap(),
        port: 0,
        token: token.clone(),
        auth_key: Some(test_key()),
    };
    let s = kaanal_server::serve(cfg, dir, opts).await.unwrap();
    Srv {
        base: format!("http://127.0.0.1:{}", s.addr.port()),
        token,
        client: reqwest::Client::new(),
        drawn_seed: s.drawn_seed,
    }
}

/// Serves a whole `kaanal.yaml` given as text, next to the fixture's `openapi.yaml`.
pub async fn start_yaml(src: &str) -> Srv {
    start_in(&Config::from_yaml(src).unwrap(), &fixtures()).await
}

/// Like [`start`], optionally overriding the fixture's `seed:`.
pub async fn start_seeded(seed: Option<u64>) -> Srv {
    let mut cfg =
        Config::from_yaml(&std::fs::read_to_string(fixtures().join("kaanal.yaml")).unwrap())
            .unwrap();
    if let Some(seed) = seed {
        cfg.seed = seed;
    }
    start_in(&cfg, &fixtures()).await
}

/// Gateway, the fixture's service and a database, every hop 1 ms fixed; tests append entries.
pub(crate) const GW_APP_DB: &str = "version: 1\nnodes:\n  nginx: { type: gateway, proc: 1ms }\n  app: { type: service, spec: openapi.yaml, proc: 1ms }\n  db: { type: database, ops: { read: 1ms, write: 1ms, scan: 1ms } }\nedges:\n  - { from: nginx, to: app, net: 1ms }\n  - { from: app, to: db, net: 1ms }\nendpoints:\n";

pub(crate) fn trace_of(r: &reqwest::Response) -> String {
    r.headers()["x-kaanal-trace"].to_str().unwrap().to_string()
}

impl Srv {
    pub async fn get(&self, path: &str) -> reqwest::Response {
        self.client
            .get(format!("{}{path}", self.base))
            .send()
            .await
            .unwrap()
    }
}

/// A gateway in front of an auth node (`proc` 50 ms, so latency lower bounds survive the
/// Windows timer) with two users; tests append `endpoints:` entries.
pub(crate) const AUTH: &str = "version: 1
nodes:
  nginx: { type: gateway, proc: 1ms }
  login:
    type: auth
    proc: 50ms
    token_ttl: 2h
    claims: { aud: my-app, tier: free }
    users:
      - { username: alice, password: s3cr3t-pw, roles: [admin], scopes: [read, write], claims: { tier: pro } }
      - { username: bob, password: pw }
edges:
  - { from: nginx, to: login, net: 1ms }
endpoints:
";

pub(crate) async fn statuses(s: &Srv) -> Vec<u16> {
    s.put_chaos("db", json!({ "error_rate": 0.5 })).await;
    let mut out = Vec::new();
    for _ in 0..16 {
        out.push(s.get("/users/42").await.status().as_u16());
    }
    out
}

/// The `GW_APP_DB` topology with extra node fields appended, e.g. `yaml(", timeout: 200ms", "", "")`.
/// Tests append `endpoints:` entries to the result if they need them.
pub(crate) fn yaml(gw: &str, app: &str, db: &str) -> String {
    format!(
        "version: 1\nnodes:\n  nginx: {{ type: gateway, proc: 1ms{gw} }}\n  app: {{ type: service, spec: openapi.yaml, proc: 1ms{app} }}\n  db: {{ type: database, ops: {{ read: 1ms, write: 1ms, scan: 1ms }}{db} }}\nedges:\n  - {{ from: nginx, to: app, net: 1ms }}\n  - {{ from: app, to: db, net: 1ms }}\nendpoints:\n  \"GET /health\": {{ override: {{ body: {{ ok: true }} }} }}\n"
    )
}
