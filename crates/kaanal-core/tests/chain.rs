use indexmap::IndexMap;
use kaanal_core::chain::{
    EndpointsInput, MAX_HOPS, RouteSpec, Seq, Step, compile, resolve_error, skeleton,
};
use kaanal_core::config::ErrorMapping;
use kaanal_core::{Config, ConfigError, Endpoints, Op, Topology};
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
    assert_eq!(
        resolve_error(&map, "db", 504),
        (503, Some(json!({ "e": 1 })))
    );
    assert_eq!(resolve_error(&map, "pay", 504), (500, None));
    assert_eq!(resolve_error(&map, "pay", 503), (502, None));
    assert_eq!(resolve_error(&map, "pay", 429), (400, None));
    assert_eq!(resolve_error(&IndexMap::new(), "pay", 503), (503, None));
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
