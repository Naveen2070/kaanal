//! Shared helpers: the base config, route sets and the outline printer.

use kaanal_core::chain::{EndpointsInput, RouteSpec, Seq, Step, compile};

use kaanal_core::{Config, ConfigError, Endpoints, Op, Topology};

/// 22 lines; tests append endpoint entries from line 23.
pub(crate) const BASE: &str = "version: 1
nodes:
  gw: { type: gateway, proc: 1ms }
  login: { type: auth, proc: 1ms }
  app: { type: service, proc: 1ms }
  orders: { type: service, proc: 1ms, chains: { reserve: [ { call: db, op: write } ] } }
  db: { type: database, flavor: postgres }
  cache: { type: cache, flavor: redis }
  pay: { type: external-api, proc: 1ms }
  shop: { type: monolith, proc: 1ms, flavor: mysql }
edges:
  - { from: gw, to: app }
  - { from: gw, to: orders }
  - { from: gw, to: login }
  - { from: app, to: orders, calls: 2 }
  - { from: app, to: shop }
  - { from: app, to: cache }
  - { from: app, to: db }
  - { from: orders, to: pay }
  - { from: orders, to: cache }
  - { from: orders, to: db }
endpoints:
";

pub(crate) const ROUTES: [(&str, &str); 4] = [
    ("GET /users/{id}", "app"),
    ("POST /users", "app"),
    ("GET /orders/{id}", "orders"),
    ("POST /orders", "orders"),
];

pub(crate) fn build(
    src: &str,
    routes: &[(&str, &str)],
) -> (Topology, Result<Endpoints, ConfigError>) {
    let cfg = Config::from_yaml(src).unwrap();
    let t = Topology::build(&cfg).unwrap();
    let specs: Vec<RouteSpec> = routes
        .iter()
        .map(|(k, o)| RouteSpec::inferred(k, t.id(o).unwrap()))
        .collect();
    let r = compile(&t, &specs, EndpointsInput::inline(&cfg)).map_err(|e| e.in_source(src));
    (t, r)
}

pub(crate) fn ok(extra: &str) -> (Topology, Endpoints) {
    let (t, r) = build(&format!("{BASE}{extra}"), &ROUTES);
    (t, r.unwrap())
}

pub(crate) fn err(src: &str) -> ConfigError {
    build(src, &ROUTES).1.unwrap_err()
}

/// `gw.proc login[login.proc] app[...]`: `name*repeat:op[body]` for calls, `cache(c)` for
/// cache steps (`?` = default-chain cache, skipped for writes), `par(a | b)` for parallel.
pub(crate) fn outline(t: &Topology, s: &Seq) -> String {
    let name = |n: usize| t.nodes[n].name.clone();
    s.steps
        .iter()
        .map(|st| match st {
            Step::Proc { node } => format!("{}.proc", name(*node)),
            Step::Op { node, op } => format!("{}.{}", name(*node), op.map_or("op", Op::name)),
            Step::Call(c) => format!(
                "{}{}{}[{}]",
                name(c.node),
                if c.repeat > 1 {
                    format!("*{}", c.repeat)
                } else {
                    String::new()
                },
                c.op.map_or(String::new(), |o| format!(":{}", o.name())),
                outline(t, &c.body)
            ),
            Step::Cache(c) => {
                format!(
                    "cache({}{})",
                    name(c.node),
                    if c.reads_only { "?" } else { "" }
                ) + &if c.repeat > 1 {
                    format!("*{}", c.repeat)
                } else {
                    String::new()
                }
            }
            Step::Parallel(bs) => format!(
                "par({})",
                bs.iter()
                    .map(|b| outline(t, b))
                    .collect::<Vec<_>>()
                    .join(" | ")
            ),
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// `k` nested `parallel` steps around one `db`.
pub(crate) fn nest(k: usize) -> String {
    if k == 0 {
        "db".into()
    } else {
        format!("{{ parallel: [ {}, db ] }}", nest(k - 1))
    }
}
