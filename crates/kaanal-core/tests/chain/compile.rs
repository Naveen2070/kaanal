//! Default chains, custom chains, step keys and positioned compile errors.

use crate::common::*;
use kaanal_core::Op;
use kaanal_core::chain::Step;
use serde_json::json;
use std::sync::Arc;

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

/// `BASE` with the cache node's fields replaced by `fields`.
fn with_cache(fields: &str) -> Vec<String> {
    let src = BASE.replace(
        "cache: { type: cache, flavor: redis }",
        &format!("cache: {{ type: cache, {fields} }}"),
    );
    let (_, r) = build(&src, &ROUTES);
    r.unwrap().warnings
}

#[test]
fn a_cache_timeout_below_the_hit_p50_warns() {
    let w = with_cache("hit_ratio: 0.8, hit: 5ms, miss: 5ms, timeout: 2ms");
    assert_eq!(w.len(), 1, "{w:?}");
    assert!(
        w[0].contains("cache") && w[0].contains("cut to misses"),
        "{w:?}"
    );
    // a percentile pair uses its p50
    let w = with_cache("hit_ratio: 0.8, hit: { p50: 5ms, p99: 9ms }, miss: 1ms, timeout: 4ms");
    assert_eq!(w.len(), 1, "{w:?}");
}

#[test]
fn a_cache_timeout_at_or_above_the_hit_p50_or_none_is_quiet() {
    assert_eq!(
        with_cache("hit_ratio: 0.8, hit: 5ms, miss: 5ms, timeout: 20ms"),
        Vec::<String>::new()
    );
    assert_eq!(
        with_cache("hit_ratio: 0.8, hit: 5ms, miss: 5ms, timeout: 5ms"),
        Vec::<String>::new()
    );
    assert_eq!(
        with_cache("hit_ratio: 0.8, hit: 5ms, miss: 5ms"),
        Vec::<String>::new()
    );
}

#[test]
fn an_override_with_a_reserved_header_fails_to_compile() {
    let e = err(&format!(
        "{BASE}  \"GET /users/{{id}}\": {{ override: {{ body: {{}}, headers: {{ content-length: \"1\" }} }} }}
"
    ));
    assert!(
        e.msg.contains("override: headers: content-length"),
        "{}",
        e.msg
    );
}

#[test]
fn an_override_with_headers_compiles() {
    let (_, eps) = ok(
        "  \"GET /users/{id}\": { override: { status: 503, body: {}, headers: { x-demo: \"1\" } } }
",
    );
    let ov = eps
        .get("GET /users/{id}")
        .unwrap()
        .response
        .as_ref()
        .unwrap();
    assert_eq!(ov.headers["x-demo"], "1");
}

#[test]
fn a_file_override_that_loaded_before_step_8a_still_loads() {
    // 8A's status and body limits are for live edits; a file keeps the rules it always had
    let (_, eps) = ok(
        "  \"GET /users/{id}\": { override: { status: 204, body: {} } }
",
    );
    assert_eq!(
        eps.get("GET /users/{id}")
            .unwrap()
            .response
            .as_ref()
            .unwrap()
            .status,
        204
    );
}
