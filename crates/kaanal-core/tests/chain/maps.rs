//! Stale entries, error maps, limits, wire nodes, deep chains and the skeleton.

use crate::common::*;
use indexmap::IndexMap;
use kaanal_core::chain::{EndpointsInput, MAX_HOPS, RouteSpec, compile, resolve_error, skeleton};
use kaanal_core::config::ErrorMapping;
use kaanal_core::{Config, FailKind, Topology};
use serde_json::json;

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
