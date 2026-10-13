use indexmap::IndexMap;
use kaanal_core::chain::{
    EndpointsInput, JWKS_KEY, LOGIN_KEY, MAX_HOPS, Responder, RouteSpec, Seq, Step, compile,
    resolve_error, skeleton,
};
use kaanal_core::config::ErrorMapping;
use kaanal_core::{
    ChainRequest, ChaosState, Config, ConfigError, Endpoints, FailKind, HopKind, Op, Topology,
    execute,
};
use serde_json::json;
use std::sync::Arc;

/// 22 lines; tests append endpoint entries from line 23.
const BASE: &str = "version: 1
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

const ROUTES: [(&str, &str); 4] = [
    ("GET /users/{id}", "app"),
    ("POST /users", "app"),
    ("GET /orders/{id}", "orders"),
    ("POST /orders", "orders"),
];

fn build(src: &str, routes: &[(&str, &str)]) -> (Topology, Result<Endpoints, ConfigError>) {
    let cfg = Config::from_yaml(src).unwrap();
    let t = Topology::build(&cfg).unwrap();
    let specs: Vec<RouteSpec> = routes
        .iter()
        .map(|(k, o)| RouteSpec::inferred(k, t.id(o).unwrap()))
        .collect();
    let r = compile(&t, &specs, EndpointsInput::inline(&cfg)).map_err(|e| e.in_source(src));
    (t, r)
}

fn ok(extra: &str) -> (Topology, Endpoints) {
    let (t, r) = build(&format!("{BASE}{extra}"), &ROUTES);
    (t, r.unwrap())
}

fn err(src: &str) -> ConfigError {
    build(src, &ROUTES).1.unwrap_err()
}

/// `gw.proc login[login.proc] app[...]`: `name*repeat:op[body]` for calls, `cache(c)` for
/// cache steps (`?` = default-chain cache, skipped for writes), `par(a | b)` for parallel.
fn outline(t: &Topology, s: &Seq) -> String {
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

#[test]
fn default_chains_follow_the_spec_order() {
    let (t, eps) = ok("");
    let get = eps.get("GET /users/{id}").unwrap();
    assert_eq!(
        outline(&t, &get.chain),
        "gw.proc login[login.proc] app[app.proc orders*2[orders.proc pay[pay.proc] cache(cache?) db[db.op]] shop[shop.proc shop.op] cache(cache?) db[db.op]]"
    );
    assert_eq!((get.op, get.owner), (Op::Read, t.id("app")));
    let post = eps.get("POST /users").unwrap();
    assert_eq!(post.op, Op::Write);
    assert_eq!(
        outline(&t, &post.chain),
        outline(&t, &get.chain),
        "one op-free tree for both"
    );
    // Default bodies are built once and shared.
    let orders = eps.get("GET /orders/{id}").unwrap();
    let Step::Call(app) = &get.chain.steps[2] else {
        panic!("entry call")
    };
    let Step::Call(inner) = &app.body.steps[1] else {
        panic!("orders call")
    };
    assert!(Arc::ptr_eq(&inner.body, &orders.body));
}

#[test]
fn static_endpoint_runs_the_gateway_chain() {
    let (t, eps) = ok("  \"GET /health\": { override: { body: { ok: true } } }\n");
    let h = eps.get("GET /health").unwrap();
    assert_eq!(h.owner, None);
    assert_eq!(outline(&t, &h.chain), "gw.proc login[login.proc]");
    assert_eq!(h.response.as_ref().unwrap().body, json!({ "ok": true }));
}

#[test]
fn custom_chain_starts_with_the_owners_work() {
    let (t, eps) = ok("  \"GET /orders/{id}\": { chain: [ { cache: cache }, db ] }\n");
    assert_eq!(
        outline(&t, &eps.get("GET /orders/{id}").unwrap().chain),
        "gw.proc login[login.proc] orders[orders.proc cache(cache) db[db.op]]"
    );
}

#[test]
fn call_bodies_inline_with_and_via() {
    let (t, eps) = ok(r#"  "GET /users/{id}":
    chain:
      - { call: orders, chain: [ pay ] }
      - { call: orders, with: reserve }
      - { call: orders, via: "POST /orders" }
      - { call: shop, op: scan }
"#);
    assert_eq!(
        outline(&t, &eps.get("GET /users/{id}").unwrap().body),
        "app.proc orders*2[orders.proc pay[pay.proc]] orders*2[orders.proc db:write[db.op]] orders*2:write[orders.proc pay[pay.proc] cache(cache?) db[db.op]] shop:scan[shop.proc shop.op]"
    );
}

#[test]
fn head_routes_copy_get_with_their_own_key() {
    let (t, eps) = ok("");
    let (get, head) = (
        eps.get("GET /users/{id}").unwrap(),
        eps.get("HEAD /users/{id}").unwrap(),
    );
    assert!(head.synthesized && !get.synthesized);
    assert!(Arc::ptr_eq(&get.chain, &head.chain));
    assert_eq!(
        (head.key.as_str(), head.op, head.owner),
        ("HEAD /users/{id}", Op::Read, t.id("app"))
    );
    let (_, eps) = ok("  \"HEAD /users/{id}\": { override: { status: 204 } }\n");
    let head = eps.get("HEAD /users/{id}").unwrap();
    assert!(
        !head.synthesized,
        "an explicit HEAD entry keeps GET's owner (R6.8)"
    );
    assert_eq!(head.owner, t.id("app"));
}

#[test]
fn step_keys_count_occurrences_per_target_across_parallel_branches() {
    let (t, eps) = ok(
        "  \"GET /users/{id}\":\n    chain: [ db, { cache: cache }, db, { parallel: [ db, orders ] } ]\n",
    );
    let body = &eps.get("GET /users/{id}").unwrap().body;
    let mut keys = Vec::new();
    let mut push = |s: &Step| match s {
        Step::Call(c) => keys.push(format!("{}#{}", t.nodes[c.key.target].name, c.key.occ)),
        Step::Cache(c) => keys.push(format!("{}#{}", t.nodes[c.key.target].name, c.key.occ)),
        _ => {}
    };
    for s in &body.steps {
        if let Step::Parallel(bs) = s {
            bs.iter().flat_map(|b| &b.steps).for_each(&mut push);
        } else {
            push(s);
        }
    }
    assert_eq!(keys, ["db#0", "cache#0", "db#1", "db#2", "orders#0"]);
}

/// `k` nested `parallel` steps around one `db`.
fn nest(k: usize) -> String {
    if k == 0 {
        "db".into()
    } else {
        format!("{{ parallel: [ {}, db ] }}", nest(k - 1))
    }
}

#[test]
#[expect(clippy::too_many_lines, reason = "one table of every compile error")]
fn errors_name_the_endpoint_and_point_at_the_step() {
    let entry = "  \"GET /users/{id}\":\n    chain:\n"; // lines 23–24; the step is line 25
    let cases: &[(&str, (usize, usize), &str)] = &[
        (
            "      - call: nope\n",
            (25, 15),
            "endpoint 'GET /users/{id}': unknown node 'nope'",
        ),
        (
            "      - nope\n",
            (25, 9),
            "endpoint 'GET /users/{id}': unknown node 'nope'",
        ),
        (
            "      - call: pay\n",
            (25, 15),
            "endpoint 'GET /users/{id}': 'app' has no edge to 'pay' (add it under edges:)",
        ),
        (
            "      - cache: db\n",
            (25, 16),
            "endpoint 'GET /users/{id}': cache: 'db' is a database, not a cache",
        ),
        (
            "      - call: cache\n",
            (25, 15),
            "endpoint 'GET /users/{id}': 'cache' is a cache; use `cache: cache`",
        ),
        (
            "      - { call: orders, op: read }\n",
            (25, 29),
            "endpoint 'GET /users/{id}': 'orders' (service) has no ops",
        ),
        (
            "      - { call: orders, op: write, via: \"POST /orders\" }\n",
            (25, 29),
            "endpoint 'GET /users/{id}': op cannot be used with via (the endpoint's own op is used)",
        ),
        (
            "      - { call: db, repeat: 0 }\n",
            (25, 29),
            "endpoint 'GET /users/{id}': repeat must be between 1 and 1000 (got 0)",
        ),
        (
            "      - { call: db, repeat: 1001 }\n",
            (25, 29),
            "endpoint 'GET /users/{id}': repeat must be between 1 and 1000 (got 1001)",
        ),
        (
            "      - parallel: [ db ]\n",
            (25, 9),
            "endpoint 'GET /users/{id}': parallel needs at least 2 branches",
        ),
        (
            "      - { call: orders, chain: [ pay ], with: reserve }\n",
            (25, 9),
            "endpoint 'GET /users/{id}': use only one of chain, with and via",
        ),
        (
            "      - { call: db, with: reserve }\n",
            (25, 9),
            "endpoint 'GET /users/{id}': 'db' (database) has no chains; chain, with and via need a service or monolith",
        ),
        (
            "      - { call: orders, with: cancel }\n",
            (25, 31),
            "endpoint 'GET /users/{id}': with: 'cancel' is not a chain of 'orders' (chains: reserve)",
        ),
        (
            "      - { call: orders, via: \"GET /nope\" }\n",
            (25, 30),
            "endpoint 'GET /users/{id}': via: 'GET /nope' is not an endpoint of 'orders' (endpoints: GET /orders/{id}, POST /orders)",
        ),
        (
            "      - { call: orders, chain: [ shop ] }\n",
            (25, 34),
            "endpoint 'GET /users/{id}': 'orders' has no edge to 'shop' (add it under edges:)",
        ),
        (
            "      - parallel: [ db, [ pay ] ]\n",
            (25, 27),
            "endpoint 'GET /users/{id}': 'app' has no edge to 'pay' (add it under edges:)",
        ),
    ];
    for (step, (line, col), want) in cases {
        let e = err(&format!("{BASE}{entry}{step}"));
        assert_eq!(
            (e.line, e.col, e.msg.as_str()),
            (Some(*line), Some(*col), *want),
            "{step}"
        );
    }
    let key = "  \"GET /users/{id}\":\n"; // line 23; the field is line 24
    for (field, (line, col), want) in [
        (
            "    chain: []\n",
            (24, 5),
            "endpoint 'GET /users/{id}': a chain needs at least one step",
        ),
        (
            "    error_map: { nope: 503 }\n",
            (24, 18),
            "endpoint 'GET /users/{id}': error_map: 'nope' is not a node name, a status (400-599) or a class (4xx, 5xx)",
        ),
        (
            "    error_map: { db: 99 }\n",
            (24, 22),
            "endpoint 'GET /users/{id}': error_map: status 99 must be between 200 and 599",
        ),
        (
            "    override: { status: 99 }\n",
            (24, 25),
            "endpoint 'GET /users/{id}': override: status 99 must be between 200 and 599",
        ),
    ] {
        let e = err(&format!("{BASE}{key}{field}"));
        assert_eq!(
            (e.line, e.col, e.msg.as_str()),
            (Some(line), Some(col), want),
            "{field}"
        );
    }
    for (entry, (line, col), want) in [
        (
            "  nope: {}\n",
            (23, 3),
            "endpoint 'nope': key must look like 'GET /path'",
        ),
        (
            "  \"FOO /x\": {}\n",
            (23, 3),
            "endpoint 'FOO /x': unknown method 'FOO'",
        ),
        (
            "  \"GET users\": {}\n",
            (23, 3),
            "endpoint 'GET users': path must start with '/'",
        ),
        (
            "  \"get /users/{id}\": {}\n  \"GET /users/{id}\": {}\n",
            (24, 3),
            "endpoint 'GET /users/{id}': same route as 'get /users/{id}'",
        ),
    ] {
        let e = err(&format!("{BASE}{entry}"));
        assert_eq!(
            (e.line, e.col, e.msg.as_str()),
            (Some(line), Some(col), want),
            "{entry}"
        );
    }
    // Node chains are compiled too, placed in `nodes:`.
    let e = err(&BASE.replace("{ call: db, op: write }", "{ call: shop }"));
    assert_eq!(
        (e.line, e.col, e.msg.as_str()),
        (
            Some(6),
            Some(68),
            "node 'orders': chain 'reserve': 'orders' has no edge to 'shop' (add it under edges:)"
        )
    );
    // An edge's `calls` is a default `repeat`, so the same cap applies (D6).
    let e = err(&BASE.replace("calls: 2", "calls: 1001"));
    assert_eq!(
        (e.line, e.col, e.msg.as_str()),
        (
            Some(15),
            Some(37),
            "edge #3: calls must be at most 1000 when a chain repeats it (got 1001)"
        )
    );
}

#[test]
fn static_routes_need_a_gateway_and_stale_entries_warn() {
    let src = "version: 1\nnodes:\n  app: { type: service, proc: 1ms }\n  db: { type: database, flavor: postgres }\nedges:\n  - { from: app, to: db }\nendpoints:\n  \"GET /old\": {}\n";
    let routes = [("GET /users/{id}", "app")];
    let eps = build(src, &routes).1.unwrap();
    assert_eq!(
        eps.warnings,
        [
            "endpoint 'GET /old': no spec defines this route and it has no override (stale entry: remove it, or add an override to serve it)"
        ]
    );
    assert!(eps.get("GET /old").is_none());
    let e = build(
        &format!("{src}  \"GET /health\": {{ override: {{ body: 1 }} }}\n"),
        &routes,
    )
    .1
    .unwrap_err();
    assert_eq!(
        (e.line, e.col, e.msg.as_str()),
        (
            Some(9),
            Some(3),
            "endpoint 'GET /health': static route needs a gateway or a service that owns it"
        )
    );
}

#[test]
fn via_target_error_map_is_ignored_with_a_warning() {
    let (_, eps) = ok(
        r#"  "GET /users/{id}": { chain: [ { call: orders, via: "POST /orders" } ] }
  "POST /orders": { error_map: { db: 503, "5XX": 502 } }
"#,
    );
    assert_eq!(
        eps.warnings,
        [
            "endpoint 'GET /users/{id}': via 'POST /orders': that endpoint's error_map is ignored here (each endpoint maps its own errors)"
        ]
    );
    let keys: Vec<&String> = eps.get("POST /orders").unwrap().error_map.keys().collect();
    assert_eq!(keys, ["db", "5xx"], "classes are stored lowercase (D11)");
}

#[test]
fn limits_are_named_errors() {
    let at = |entry: &str| err(&format!("{BASE}  \"GET /users/{{id}}\":\n{entry}"));
    let many = vec!["db"; 257].join(", ");
    let e = at(&format!("    chain: [ {many} ]\n"));
    assert_eq!((e.line, e.col), (Some(24), Some(5)), "{e}");
    assert_eq!(
        e.msg,
        "endpoint 'GET /users/{id}': a chain allows at most 256 steps (got 257)"
    );
    let ok_many = vec!["db"; 256].join(", ");
    ok(&format!(
        "  \"GET /users/{{id}}\":\n    chain: [ {ok_many} ]\n"
    ));
    let wide = vec!["db"; 33].join(", ");
    let e = at(&format!("    chain:\n      - parallel: [ {wide} ]\n"));
    assert_eq!(
        e.msg,
        "endpoint 'GET /users/{id}': parallel allows at most 32 branches (got 33)"
    );
    // nest(k) has k parallels; the steps inside the k-th sit at level k + 1.
    ok(&format!(
        "  \"GET /users/{{id}}\":\n    chain: [ {} ]\n",
        nest(15)
    ));
    let e = at(&format!("    chain: [ {} ]\n", nest(16)));
    assert_eq!(
        e.msg,
        "endpoint 'GET /users/{id}': chains nest at most 16 levels deep"
    );
    assert_eq!(e.line, Some(24), "{e}");
    let e = at(
        "    chain: [ { call: orders, repeat: 1000, chain: [ { call: db, repeat: 1000 } ] } ]\n",
    );
    assert_eq!((e.line, e.col), (Some(23), Some(3)), "{e}");
    assert_eq!(
        e.msg,
        format!(
            "endpoint 'GET /users/{{id}}': resolves to 2002005 hops (max {MAX_HOPS}); lower repeat or calls"
        )
    );
    // D9: deep call trees are bounded too (70 services in a line).
    let nodes = (0..70)
        .map(|i| format!("  s{i}: {{ type: service }}\n"))
        .collect::<Vec<_>>()
        .concat();
    let edges = (0..69)
        .map(|i| format!("  - {{ from: s{i}, to: s{} }}\n", i + 1))
        .collect::<Vec<_>>()
        .concat();
    let y = format!("version: 1\nnodes:\n{nodes}edges:\n{edges}");
    let e = build(&y, &[("GET /x", "s0")]).1.unwrap_err();
    assert_eq!(
        e.msg,
        "endpoint 'GET /x': resolves to calls nested 69 deep (max 64)"
    );
    // No entry to point at: the error lands on the owning node (line 3 is `s0`).
    assert_eq!((e.line, e.col), (Some(3), Some(3)), "{e}");
    // The same for a default chain that is too big through edge `calls`.
    let y = "version: 1\nnodes:\n  app: { type: service }\n  x: { type: service }\n  y: { type: service }\nedges:\n  - { from: app, to: x, calls: 1000 }\n  - { from: x, to: y, calls: 1000 }\n";
    let e = build(y, &[("GET /x", "app")]).1.unwrap_err();
    assert!(e.msg.contains("hops (max 100000)"), "{e}");
    assert_eq!((e.line, e.col), (Some(3), Some(3)), "{e}");
}

#[test]
fn resolve_error_looks_up_node_then_status_then_class() {
    let m = |s: u16, body: Option<serde_json::Value>| ErrorMapping { status: s, body };
    let map: IndexMap<String, ErrorMapping> = [
        ("db".to_string(), m(503, Some(json!({ "e": 1 })))),
        ("504".to_string(), m(500, None)),
        ("5xx".to_string(), m(502, None)),
        ("4xx".to_string(), m(400, None)),
    ]
    .into_iter()
    .collect();
    let s = FailKind::Status;
    assert_eq!(
        resolve_error(&map, "db", s, 504),
        (503, Some(json!({ "e": 1 })))
    );
    assert_eq!(resolve_error(&map, "pay", s, 504), (500, None));
    assert_eq!(resolve_error(&map, "pay", s, 503), (502, None));
    assert_eq!(resolve_error(&map, "pay", s, 429), (400, None));
    assert_eq!(resolve_error(&IndexMap::new(), "pay", s, 503), (503, None));
}

#[test]
fn error_map_resolves_node_then_kind_then_status_then_class() {
    let map = |pairs: &[(&str, u16)]| -> IndexMap<String, ErrorMapping> {
        pairs
            .iter()
            .map(|(k, s)| {
                (
                    k.to_string(),
                    ErrorMapping {
                        status: *s,
                        body: None,
                    },
                )
            })
            .collect()
    };
    let m = map(&[("db", 418), ("timeout", 598), ("504", 597), ("5xx", 596)]);
    let r = |node, kind, status| resolve_error(&m, node, kind, status).0;
    assert_eq!(r("db", FailKind::Timeout, 504), 418, "node wins");
    assert_eq!(r("app", FailKind::Timeout, 504), 598, "then kind");
    assert_eq!(r("app", FailKind::Status, 504), 597, "then status");
    assert_eq!(r("app", FailKind::Status, 503), 596, "then class");
    // reset has no status: defaults to 502 first, then status/class keys see 502
    let m2 = map(&[("502", 599)]);
    assert_eq!(resolve_error(&m2, "app", FailKind::Reset, 502).0, 599);
}

#[test]
fn error_map_accepts_the_kind_keys() {
    let (_, eps) =
        ok("  \"GET /users/{id}\": { error_map: { reset: 503, malformed: 502, timeout: 599 } }\n");
    let ep = eps.get("GET /users/{id}").unwrap();
    let keys: Vec<&str> = ep.error_map.keys().map(String::as_str).collect();
    assert_eq!(keys, ["reset", "malformed", "timeout"]);
}

#[test]
fn wire_nodes_are_the_gateway_and_the_owner() {
    let (t, eps) = ok("");
    let ep = eps.get("GET /users/{id}").unwrap();
    let (gw, app, db) = (
        t.id("gw").unwrap(),
        t.id("app").unwrap(),
        t.id("db").unwrap(),
    );
    assert!(ep.is_wire_node(&t, gw) && ep.is_wire_node(&t, app) && !ep.is_wire_node(&t, db));
    assert_eq!(ep.entry_node, Some(gw));
}

#[test]
fn deep_call_chains_drop_without_deep_recursion() {
    // Leaf first in `nodes:`, so the root's body is the last one a naive drop reaches.
    let n = 5000;
    let nodes = (0..n)
        .rev()
        .map(|i| format!("  s{i}: {{ type: service }}\n"))
        .collect::<Vec<_>>()
        .concat();
    let edges = (0..n - 1)
        .map(|i| format!("  - {{ from: s{i}, to: s{} }}\n", i + 1))
        .collect::<Vec<_>>()
        .concat();
    let cfg = Config::from_yaml(&format!("version: 1\nnodes:\n{nodes}edges:\n{edges}")).unwrap();
    let t = Topology::build(&cfg).unwrap();
    let routes = [RouteSpec::inferred("GET /x", t.id("s0").unwrap())];
    // A small stack: dropping the 5000-deep tree recursively would overflow it (R6.12).
    std::thread::scope(|s| {
        std::thread::Builder::new()
            .stack_size(256 * 1024)
            .spawn_scoped(s, || {
                let e = compile(&t, &routes, EndpointsInput::inline(&cfg)).unwrap_err();
                assert!(e.msg.contains("nested 4999 deep"), "{}", e.msg);
            })
            .unwrap()
            .join()
            .unwrap();
    });
}

#[test]
fn error_map_keys_are_strict_and_unique() {
    let at = |map: &str| {
        err(&format!(
            "{BASE}  \"GET /users/{{id}}\":\n    error_map: {map}\n"
        ))
    };
    for key in ["\"+503\"", "\"0503\"", "\" 503\"", "\"50\"", "\"6xx\""] {
        let e = at(&format!("{{ {key}: 502 }}"));
        assert!(
            e.msg
                .contains("is not a node name, a status (400-599) or a class"),
            "{key}: {e}"
        );
    }
    let e = at("{ \"5xx\": 501, \"5XX\": 502 }");
    assert_eq!(
        (e.line, e.msg.as_str()),
        (
            Some(24),
            "endpoint 'GET /users/{id}': error_map: '5XX' is the same key as '5xx'"
        )
    );
}

#[test]
fn via_target_override_is_ignored_with_a_warning() {
    let (_, eps) = ok(
        r#"  "GET /users/{id}": { chain: [ { call: orders, via: "POST /orders" } ] }
  "POST /orders": { override: { status: 201 } }
"#,
    );
    assert_eq!(
        eps.warnings,
        [
            "endpoint 'GET /users/{id}': via 'POST /orders': that endpoint's override is ignored here (via runs its chain, not its fixed response)"
        ]
    );
}

#[test]
fn head_entry_of_a_static_get_warns_that_the_copy_is_served() {
    let (_, eps) =
        ok("  \"GET /h\": { override: { body: 1 } }\n  \"HEAD /h\": { chain: [ app ] }\n");
    assert_eq!(
        eps.warnings,
        [
            "endpoint 'HEAD /h': ignored: a HEAD of a static GET entry is served as a copy of that GET (give it an override to serve it separately)"
        ]
    );
    assert!(eps.get("HEAD /h").unwrap().synthesized);
}

#[test]
fn an_edge_with_calls_is_one_cache_step_with_a_repeat() {
    let src = BASE.replace(
        "{ from: app, to: cache }",
        "{ from: app, to: cache, calls: 3 }",
    );
    let (t, r) = build(&src, &ROUTES);
    let eps = r.unwrap();
    let get = eps.get("GET /users/{id}").unwrap();
    let body = outline(&t, &get.body);
    assert_eq!(body.matches("cache(cache?)*3").count(), 1, "{body}");
}

#[test]
fn skeleton_lists_missing_spec_routes_sorted() {
    let (t, _) = ok("");
    let routes: Vec<RouteSpec> = ROUTES
        .iter()
        .map(|(k, o)| RouteSpec::inferred(k, t.id(o).unwrap()))
        .collect();
    // Existing keys match after normalizing (method case, path spaces): `post /users` covers `POST /users`.
    assert_eq!(
        skeleton(&t, &routes, &["post /users".to_string()]),
        [
            r#""GET /orders/{id}": {} # owner orders, op read"#,
            r#""GET /users/{id}": {} # owner app, op read"#,
            r#""POST /orders": {} # owner orders, op write"#,
        ]
    );
    assert_eq!(skeleton(&t, &routes, &[]).len(), 4);
}

/// An auth node `login` with a user store, a session cache and a mail API behind it, and a
/// service `app`. `gw` is a gateway node line or ""; `edges` adds edges; `entries` goes under
/// `endpoints:` (line 14 with a gateway, so entries start at line 15).
fn auth_topo(
    gw: &str,
    edges: &str,
    entries: &str,
    routes: &[(&str, &str)],
) -> (Topology, Result<Endpoints, ConfigError>) {
    build(
        &format!(
            "version: 1\nnodes:\n{gw}  login: {{ type: auth, proc: 1ms }}\n  app: {{ type: service, proc: 1ms }}\n  users: {{ type: database, flavor: postgres }}\n  sessions: {{ type: cache, flavor: redis }}\n  mail: {{ type: external-api, proc: 1ms }}\nedges:\n  - {{ from: login, to: users }}\n  - {{ from: login, to: sessions }}\n  - {{ from: login, to: mail }}\n{edges}endpoints:\n{entries}"
        ),
        routes,
    )
}

const GW: &str = "  gw: { type: gateway, proc: 1ms }\n";

/// X12: the login body walks auth's out-edges like a service body (callees, caches, databases).
const LOGIN_BODY: &str = "login.proc mail[mail.proc] cache(sessions?) users[users.op]";

#[test]
fn login_and_jwks_are_built_in_with_the_auth_edge() {
    let (t, eps) = auth_topo(
        GW,
        "  - { from: gw, to: app }\n  - { from: gw, to: login }\n",
        "",
        &[("GET /users/{id}", "app")],
    );
    let eps = eps.unwrap();
    let login = eps.get(LOGIN_KEY).unwrap();
    assert_eq!(
        (login.owner, login.op, login.responder),
        (t.id("login"), Op::Read, Some(Responder::Login))
    );
    // X7: the gateway's auth call runs the login body, so auth is charged exactly once.
    assert_eq!(
        outline(&t, &login.chain),
        format!("gw.proc login[{LOGIN_BODY}]")
    );
    let jwks = eps.get(JWKS_KEY).unwrap();
    assert_eq!(jwks.responder, Some(Responder::Jwks));
    assert_eq!(outline(&t, &jwks.chain), "gw.proc login[login.proc]");
    // Ordinary routes keep the auth_request call with auth's `proc` only (A2 D5).
    let get = eps.get("GET /users/{id}").unwrap();
    assert_eq!(
        outline(&t, &get.chain),
        "gw.proc login[login.proc] app[app.proc]"
    );
    assert!(get.responder.is_none());
    assert!(eps.warnings.is_empty(), "{:?}", eps.warnings);
}

#[test]
fn login_without_the_auth_edge_runs_auth_inline() {
    let (t, eps) = auth_topo(GW, "  - { from: gw, to: app }\n", "", &[]);
    let eps = eps.unwrap();
    // X7 local validation: no network hop between gateway and auth, auth's proc once (P7).
    assert_eq!(
        outline(&t, &eps.get(LOGIN_KEY).unwrap().chain),
        format!("gw.proc {LOGIN_BODY}")
    );
    let jwks = eps.get(JWKS_KEY).unwrap();
    let o = execute(
        &t,
        &ChaosState::default(),
        &ChainRequest {
            endpoint: jwks,
            n: 0,
            seed: 1,
        },
    );
    let hops: Vec<(String, HopKind)> = o
        .hops
        .iter()
        .map(|h| (t.nodes[h.node].name.clone(), h.kind))
        .collect();
    assert_eq!(
        hops,
        [
            ("gw".to_string(), HopKind::Proc),
            ("login".to_string(), HopKind::Proc)
        ]
    );
}

#[test]
fn gatewayless_and_auth_only_logins_start_at_the_auth_node() {
    let (t, eps) = auth_topo("", "", "", &[("GET /users/{id}", "app")]);
    let eps = eps.unwrap();
    assert_eq!(outline(&t, &eps.get(LOGIN_KEY).unwrap().chain), LOGIN_BODY);
    assert_eq!(outline(&t, &eps.get(JWKS_KEY).unwrap().chain), "login.proc");
    // X11: an auth node alone compiles both routes and the JWKS HEAD copy.
    let (t, eps) = build(
        "version: 1\nnodes:\n  login: { type: auth, proc: 1ms }\nendpoints:\n",
        &[],
    );
    let eps = eps.unwrap();
    let keys: Vec<&str> = eps.iter().map(|e| e.key.as_str()).collect();
    assert_eq!(
        keys,
        [
            "GET /.well-known/jwks.json",
            "HEAD /.well-known/jwks.json",
            "POST /login"
        ]
    );
    assert_eq!(
        outline(&t, &eps.get(LOGIN_KEY).unwrap().chain),
        "login.proc"
    );
}

#[test]
fn login_charges_auth_once_even_with_edge_calls() {
    let (t, eps) = auth_topo(
        GW,
        "  - { from: gw, to: app }\n  - { from: gw, to: login, calls: 2 }\n",
        "",
        &[("GET /users/{id}", "app")],
    );
    let eps = eps.unwrap();
    // Two auth_request checks per ordinary request, but one login (P6).
    assert_eq!(
        outline(&t, &eps.get("GET /users/{id}").unwrap().chain),
        "gw.proc login*2[login.proc] app[app.proc]"
    );
    assert_eq!(
        outline(&t, &eps.get(LOGIN_KEY).unwrap().chain),
        format!("gw.proc login[{LOGIN_BODY}]")
    );
    assert_eq!(
        outline(&t, &eps.get(JWKS_KEY).unwrap().chain),
        "gw.proc login[login.proc]"
    );
}

#[test]
fn a_login_chain_runs_with_the_auth_node_as_runner() {
    let (t, eps) = auth_topo(
        GW,
        "  - { from: gw, to: login }\n",
        "  \"POST /login\": { chain: [ users, { call: mail, repeat: 2 } ] }\n  \"GET /.well-known/jwks.json\": { chain: [ { cache: sessions } ] }\n",
        &[],
    );
    let eps = eps.unwrap();
    assert_eq!(
        outline(&t, &eps.get(LOGIN_KEY).unwrap().chain),
        "gw.proc login[login.proc users[users.op] mail*2[mail.proc]]"
    );
    assert_eq!(
        outline(&t, &eps.get(JWKS_KEY).unwrap().chain),
        "gw.proc login[login.proc cache(sessions)]"
    );
    // A step auth has no edge for is the usual compile error, placed at the step.
    let (_, r) = auth_topo(
        GW,
        "  - { from: gw, to: login }\n",
        "  \"POST /login\": { chain: [ app ] }\n",
        &[],
    );
    let e = r.unwrap_err();
    assert_eq!((e.line, e.col), (Some(15), Some(29)), "{e}");
    assert!(e.msg.contains("'login' has no edge to 'app'"), "{e}");
}

#[test]
fn a_spec_route_replaces_the_built_in_route_with_a_warning() {
    let (t, eps) = auth_topo(
        GW,
        "  - { from: gw, to: app }\n  - { from: gw, to: login }\n",
        "",
        &[("POST /login", "app")],
    );
    let eps = eps.unwrap();
    let login = eps.get(LOGIN_KEY).unwrap();
    assert_eq!((login.owner, login.responder), (t.id("app"), None));
    assert_eq!(
        eps.get(JWKS_KEY).unwrap().responder,
        Some(Responder::Jwks),
        "the other built-in route stays"
    );
    assert_eq!(
        eps.warnings,
        [
            "endpoint 'POST /login': the spec of 'app' defines it, so it replaces the built-in route of auth node 'login'"
        ]
    );
}

#[test]
fn endpoint_entries_customise_built_in_routes() {
    let (_, eps) = auth_topo(
        GW,
        "  - { from: gw, to: login }\n",
        "  \"post /login\": { error_map: { login: 502 } }\n  \"GET /.well-known/jwks.json\": { override: { body: { keys: [] } } }\n",
        &[],
    );
    let eps = eps.unwrap();
    let login = eps.get(LOGIN_KEY).unwrap();
    assert_eq!(
        login.error_map["login"],
        ErrorMapping {
            status: 502,
            body: None
        }
    );
    assert_eq!(login.responder, Some(Responder::Login));
    let jwks = eps.get(JWKS_KEY).unwrap();
    assert_eq!(
        jwks.response.as_ref().map(|o| o.body.clone()),
        Some(json!({ "keys": [] }))
    );
    assert_eq!(
        jwks.responder,
        Some(Responder::Jwks),
        "the responder stays; the server lets the override win"
    );
    assert!(eps.warnings.is_empty(), "not stale: {:?}", eps.warnings);
}

#[test]
fn head_copy_of_jwks_keeps_its_responder() {
    let (t, eps) = auth_topo(GW, "  - { from: gw, to: login }\n", "", &[]);
    let eps = eps.unwrap();
    let head = eps.get("HEAD /.well-known/jwks.json").unwrap();
    assert!(head.synthesized);
    assert_eq!(head.responder, Some(Responder::Jwks));
    assert_eq!(outline(&t, &head.chain), "gw.proc login[login.proc]");
    assert!(eps.get("HEAD /login").is_none(), "POST only, no HEAD copy");
}

#[test]
fn a_head_entry_for_the_jwks_is_a_copy_of_the_built_in_route() {
    let entry = "  \"HEAD /.well-known/jwks.json\": { override: { status: 204, body: null } }\n";
    // Without a gateway this used to be "static route needs a gateway"; with one it was
    // an ownerless static route. Both now serve the auth node's JWKS route, customised.
    for (gw, edges, shown) in [
        ("", "", "login.proc"),
        (
            GW,
            "  - { from: gw, to: login }\n",
            "gw.proc login[login.proc]",
        ),
    ] {
        let (t, eps) = auth_topo(gw, edges, entry, &[]);
        let eps = eps.unwrap();
        let head = eps.get("HEAD /.well-known/jwks.json").unwrap();
        assert_eq!(
            (head.owner, head.op, head.responder, head.synthesized),
            (t.id("login"), Op::Read, Some(Responder::Jwks), false)
        );
        assert!(head.response.is_some());
        assert_eq!(outline(&t, &head.chain), shown);
        assert!(eps.warnings.is_empty(), "{:?}", eps.warnings);
    }
}

#[test]
fn without_an_auth_node_there_are_no_built_in_routes() {
    let (_, r) = build(
        "version: 1\nnodes:\n  gw: { type: gateway }\n  app: { type: service }\nedges:\n  - { from: gw, to: app }\nendpoints:\n  \"POST /login\": {}\n",
        &[("GET /users/{id}", "app")],
    );
    let eps = r.unwrap();
    assert!(eps.get(LOGIN_KEY).is_none() && eps.get(JWKS_KEY).is_none());
    assert_eq!(
        eps.warnings,
        [
            "endpoint 'POST /login': no spec defines this route and it has no override (stale entry: remove it, or add an override to serve it)"
        ]
    );
}
