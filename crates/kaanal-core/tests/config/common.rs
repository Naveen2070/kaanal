//! Shared helpers: the sample config.

use kaanal_core::ConfigError;

pub(crate) const SAMPLE: &str = r#"
version: 1
seed: 42
port: 8080
nodes:
  nginx: { type: gateway,  flavor: nginx, proc: { p50: 1ms, p99: 3ms } }
  app:   { type: service,  spec: openapi.yaml, proc: 5ms }
  db:    { type: database, flavor: postgres,
           ops: { read: { p50: 4ms, p99: 20ms }, write: 8ms, scan: 30ms } }
edges:
  - { from: nginx, to: app, net: 1ms }
  - { from: app,   to: db }
endpoints:
  "GET /health": { override: { body: { ok: true } } }
"#;

/// The text of `src` from the position `e` reports.
pub(crate) fn at<'a>(src: &'a str, e: &ConfigError) -> &'a str {
    let line = src.lines().nth(e.line.expect("a line") - 1).unwrap();
    &line[e.col.expect("a column") - 1..]
}
