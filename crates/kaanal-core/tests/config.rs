use kaanal_core::config::{
    Branch, CacheFlavor, CallStep, DbFlavor, Determinism, DistConfig, EndpointsConfig,
    ErrorMapping, NodeConfig, Override, SeedSource, StepConfig, parse_endpoints,
};
use kaanal_core::{Config, NodeKind, Op, Topology};
use serde_json::json;
use std::fmt::Write as _;
use std::num::NonZeroU32;
use std::time::Duration;

const SAMPLE: &str = r#"
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

#[test]
fn parses_sample() {
    let cfg = Config::from_yaml(SAMPLE).unwrap();
    assert_eq!(cfg.seed, 42);
    assert_eq!(cfg.port, 8080);
    assert_eq!(cfg.nodes.len(), 3);
    assert_eq!(cfg.nodes["nginx"].kind(), NodeKind::Gateway);
    assert_eq!(cfg.nodes["app"].spec(), Some("openapi.yaml"));
    assert_eq!(cfg.edges.len(), 2);
    let Some(EndpointsConfig::Inline(eps)) = &cfg.endpoints else {
        panic!("inline endpoints: {:?}", cfg.endpoints)
    };
    assert_eq!(eps["GET /health"].response.as_ref().unwrap().status, 200);
    match cfg.nodes["app"].proc().unwrap() {
        DistConfig::Fixed(d) => assert_eq!(*d, Duration::from_millis(5)),
        other => panic!("expected fixed, got {other:?}"),
    }
}

#[test]
fn defaults_apply() {
    let cfg = Config::from_yaml("version: 1\nnodes: {}\n").unwrap();
    assert_eq!(cfg.seed, 0);
    assert_eq!(cfg.port, 8080);
    assert!(cfg.edges.is_empty());
    assert_eq!(cfg.determinism, Determinism::Controlled);
    assert_eq!(cfg.seed_source(), SeedSource::Fixed(0));
    assert!(cfg.endpoints.is_none());
}

#[test]
fn bad_value_reports_line() {
    let err = Config::from_yaml("version: 1\nseed: 1\nport: notanumber\nnodes: {}\n").unwrap_err();
    assert_eq!(err.line, Some(3), "{err}");
    assert!(err.to_string().starts_with("3:"), "{err}");
}

#[test]
fn unknown_field_is_rejected() {
    let err = Config::from_yaml("version: 1\nnodes: {}\nbogus: 1\n").unwrap_err();
    assert!(err.msg.contains("bogus"), "{err}");
}

#[test]
fn wrong_version_is_rejected() {
    let err = Config::from_yaml("version: 2\nnodes: {}\n").unwrap_err();
    assert!(err.msg.contains("version"), "{err}");
}

#[test]
fn bom_and_crlf_parse_like_lf() {
    let windows = format!("\u{feff}{}", SAMPLE.replace('\n', "\r\n"));
    let cfg = Config::from_yaml(&windows).unwrap();
    assert_eq!(cfg.nodes.len(), 3);
}

#[test]
fn bad_duration_explains_what_is_expected() {
    let yaml = "version: 1\nnodes:\n  gw: { type: gateway, proc: { p50: oops, p99: 3ms } }\n";
    let err = Config::from_yaml(yaml).unwrap_err();
    assert!(err.msg.contains("expected a duration like"), "{err}");
    assert_eq!(err.line, Some(3), "{err}");
    let err =
        Config::from_yaml("version: 1\nnodes:\n  gw: { type: gateway, proc: fast }\n").unwrap_err();
    assert!(err.msg.contains("expected a duration like"), "{err}");
}

#[test]
fn huge_durations_are_rejected_with_a_position() {
    let yaml = "version: 1\nnodes:\n  gw: { type: gateway, proc: 10000000000000000000s }\n";
    let err = Config::from_yaml(yaml).unwrap_err();
    assert!(err.to_string().contains("at most 1h"), "{err}");
    assert_eq!(err.line, Some(3), "{err}");
    // A p99 over the limit is caught too; 1h itself is fine.
    let yaml = "version: 1\nnodes:\n  gw: { type: gateway, proc: { p50: 1ms, p99: 2h } }\n";
    assert!(Config::from_yaml(yaml).is_err());
    let yaml = "version: 1\nnodes:\n  gw: { type: gateway, proc: 1h }\n";
    assert!(Config::from_yaml(yaml).is_ok());
    // p50 above p99 would become an uncapped Fixed(p50); equal is fine.
    let yaml = "version: 1\nnodes:\n  gw: { type: gateway, proc: { p50: 100h, p99: 1ms } }\n";
    assert!(Config::from_yaml(yaml).is_err());
    let yaml = "version: 1\nnodes:\n  gw: { type: gateway, proc: { p50: 1ms, p99: 1ms } }\n";
    assert!(Config::from_yaml(yaml).is_ok());
}

#[test]
fn exponential_duration_parses_and_is_strict() {
    let cfg =
        Config::from_yaml("version: 1\nnodes:\n  gw: { type: gateway, proc: { mean: 5ms } }\n")
            .unwrap();
    assert!(
        matches!(
            cfg.nodes["gw"].proc(),
            Some(DistConfig::Exp { mean }) if *mean == Duration::from_millis(5)
        ),
        "{:?}",
        cfg.nodes["gw"].proc()
    );
    // Over the 1 h cap: rejected with its line.
    let err =
        Config::from_yaml("version: 1\nnodes:\n  gw: { type: gateway, proc: { mean: 2h } }\n")
            .unwrap_err();
    assert!(err.to_string().contains("at most 1h"), "{err}");
    assert_eq!(err.line, Some(3), "{err}");
    // Mixed shapes are not silently read as one of them.
    let err = Config::from_yaml(
        "version: 1\nnodes:\n  gw: { type: gateway, proc: { mean: 5ms, p99: 9ms } }\n",
    )
    .unwrap_err();
    assert!(err.msg.contains("{ mean: 5ms }"), "{err}");
    // Extra keys on a percentile pair are rejected too, not dropped.
    let err = Config::from_yaml(
        "version: 1
nodes:
  gw: { type: gateway, proc: { p50: 1ms, p99: 2ms, mean: 3ms } }
",
    )
    .unwrap_err();
    assert!(err.msg.contains("{ mean: 5ms }"), "{err}");
}

#[test]
fn concurrency_parses_on_every_node_type_and_must_be_positive() {
    let yaml = "version: 1\nnodes:\n  gw: { type: gateway, concurrency: 4 }\n  app: { type: service, concurrency: 2 }\n  db: { type: database, concurrency: 1, ops: { read: 1ms, write: 1ms, scan: 1ms } }\nedges:\n  - { from: gw, to: app }\n  - { from: app, to: db }\n";
    let cfg = Config::from_yaml(yaml).unwrap();
    let got: Vec<u32> = ["gw", "app", "db"]
        .iter()
        .map(|n| cfg.nodes[*n].concurrency().unwrap().get())
        .collect();
    assert_eq!(got, [4, 2, 1]);
    let t = Topology::build(&cfg).unwrap();
    assert_eq!(
        t.nodes[t.id("db").unwrap()]
            .concurrency
            .map(NonZeroU32::get),
        Some(1)
    );
    for bad in ["0", "-1", "1.5"] {
        let err = Config::from_yaml(&format!(
            "version: 1\nnodes:\n  gw: {{ type: gateway, concurrency: {bad} }}\n"
        ))
        .unwrap_err();
        assert_eq!(err.line, Some(3), "{bad}: {err}");
    }
    let cfg = Config::from_yaml("version: 1\nnodes:\n  gw: { type: gateway }\n").unwrap();
    assert!(
        cfg.nodes["gw"].concurrency().is_none(),
        "absent means unlimited"
    );
}

const ALL_KINDS: &str = r#"
version: 1
nodes:
  gw:    { type: gateway, flavor: nginx, proc: 1ms, concurrency: 8 }
  app:   { type: service, spec: openapi.yaml, proc: 2ms, concurrency: 4 }
  mono:  { type: monolith, spec: openapi.yaml, proc: 3ms, flavor: mysql, ops: { read: 1ms }, concurrency: 2 }
  db:    { type: database, flavor: postgres, ops: { write: 9ms }, concurrency: 16,
           pool: { size: 10, queue: 5, overflow_status: 504 } }
  cache: { type: cache, flavor: redis, hit_ratio: 0.8, hit: 1ms, miss: 2ms, ttl: 30s, concurrency: 32 }
  pay:   { type: external-api, proc: 50ms, error_rate: 0.01, rate_limit: { rps: 100, burst: 20 },
           proxy: "https://api.example.com", pool: { size: 4 }, concurrency: 4 }
  login: { type: auth, proc: 5ms, token_ttl: 2h, concurrency: 2,
           users: [ { username: alice, password: secret, roles: [admin] }, { username: bob, password: pw } ] }
"#;

#[test]
fn all_seven_kinds_parse_with_every_field() {
    let cfg = Config::from_yaml(ALL_KINDS).unwrap();
    let kinds: Vec<&str> = cfg.nodes.values().map(|n| n.kind().as_str()).collect();
    assert_eq!(
        kinds,
        [
            "gateway",
            "service",
            "monolith",
            "database",
            "cache",
            "external-api",
            "auth"
        ]
    );
    let NodeConfig::Monolith(mono) = &cfg.nodes["mono"] else {
        panic!("mono")
    };
    assert_eq!(mono.flavor, Some(DbFlavor::Mysql));
    assert_eq!(
        mono.ops.as_ref().unwrap().read,
        Some(DistConfig::Fixed(Duration::from_millis(1)))
    );
    let NodeConfig::Database(db) = &cfg.nodes["db"] else {
        panic!("db")
    };
    let pool = db.pool.as_ref().unwrap();
    assert_eq!(
        (pool.size.get(), pool.queue, pool.overflow_status),
        (10, 5, 504)
    );
    let NodeConfig::Cache(cache) = &cfg.nodes["cache"] else {
        panic!("cache")
    };
    assert_eq!(cache.flavor, Some(CacheFlavor::Redis));
    assert_eq!(cache.hit_ratio, Some(0.8));
    assert_eq!(cache.ttl, Some(Duration::from_secs(30)));
    let NodeConfig::ExternalApi(pay) = &cfg.nodes["pay"] else {
        panic!("pay")
    };
    assert_eq!(pay.proxy.as_deref(), Some("https://api.example.com"));
    assert_eq!(
        pay.pool.as_ref().unwrap().overflow_status,
        503,
        "default overflow status"
    );
    assert_eq!(pay.rate_limit.as_ref().unwrap().burst.get(), 20);
    let NodeConfig::Auth(login) = &cfg.nodes["login"] else {
        panic!("login")
    };
    assert_eq!(login.users.len(), 2);
    assert_eq!(login.users[1].roles, Vec::<String>::new());
    assert_eq!(login.token_ttl, Some(Duration::from_secs(7200)));
    assert_eq!(cfg.nodes["gw"].flavor(), Some("nginx"));
    assert_eq!(cfg.nodes["db"].flavor(), Some("postgres"));
    assert_eq!(cfg.nodes["mono"].spec(), Some("openapi.yaml"));
    assert!(cfg.nodes["db"].proc().is_none());
}

#[test]
fn unknown_type_lists_the_kinds() {
    let err = Config::from_yaml("version: 1\nnodes:\n  r: { type: router }\n").unwrap_err();
    for kind in [
        "gateway",
        "service",
        "monolith",
        "database",
        "cache",
        "external-api",
        "auth",
    ] {
        assert!(err.msg.contains(kind), "{kind}: {err}");
    }
    assert_eq!(err.line, Some(3), "{err}");
}

#[test]
fn fields_belong_to_their_kind() {
    for (node, field) in [
        ("{ type: service, ops: { read: 1ms } }", "ops"),
        ("{ type: gateway, spec: x.yaml }", "spec"),
        ("{ type: database, proc: 1ms }", "proc"),
        ("{ type: cache, pool: { size: 1 } }", "pool"),
        ("{ type: auth, flavor: postgres }", "flavor"),
    ] {
        let err = Config::from_yaml(&format!("version: 1\nnodes:\n  n: {node}\n")).unwrap_err();
        assert!(
            err.msg.contains(&format!("unknown field '{field}'")),
            "{node}: {err}"
        );
    }
}

#[test]
fn p50_above_p99_is_rejected_with_a_position() {
    for proc in ["{ p50: 20ms, p99: 5ms }", "{ p50: 100h, p99: 1ms }"] {
        let err = Config::from_yaml(&format!(
            "version: 1\nnodes:\n  gw: {{ type: gateway, proc: {proc} }}\n"
        ))
        .unwrap_err();
        assert!(err.msg.contains("above p99"), "{proc}: {err}");
        assert_eq!(err.line, Some(3), "{proc}: {err}");
    }
    // Equal percentiles are fine: a fixed cost.
    Config::from_yaml(
        "version: 1\nnodes:\n  gw: { type: gateway, proc: { p50: 5ms, p99: 5ms } }\n",
    )
    .unwrap();
}

#[test]
fn inert_fields_are_validated() {
    let cases = [
        (
            "{ type: database, flavor: postgres, pool: { size: 0 } }",
            "nonzero",
        ),
        (
            "{ type: database, flavor: postgres, pool: { queue: 1 } }",
            "size",
        ),
        (
            "{ type: database, flavor: postgres, pool: { size: 1, overflow_status: 418 } }",
            "between 500 and 504",
        ),
        (
            "{ type: external-api, rate_limit: { rps: 0, burst: 1 } }",
            "rps must be above 0",
        ),
        (
            "{ type: external-api, rate_limit: { rps: 10, burst: 0 } }",
            "nonzero",
        ),
        (
            "{ type: external-api, error_rate: -0.1 }",
            "between 0 and 1",
        ),
        ("{ type: external-api, proxy: \"ftp://x\" }", "http(s) URL"),
        ("{ type: cache, hit_ratio: 1.5 }", "between 0 and 1"),
        ("{ type: auth, token_ttl: 2d }", "at most 1 day"),
        (
            "{ type: auth, users: [ { username: a, password: x }, { username: a, password: y } ] }",
            "duplicate username 'a'",
        ),
        ("{ type: auth, users: [ { username: a } ] }", "password"),
    ];
    for (node, want) in cases {
        let err = Config::from_yaml(&format!("version: 1\nnodes:\n  n: {node}\n")).unwrap_err();
        assert!(err.msg.contains(want), "{node}: want '{want}' in: {err}");
        assert_eq!(err.line, Some(3), "{node}: {err}");
    }
}

#[test]
fn edge_calls_default_to_one_and_reject_zero() {
    let base = "version: 1\nnodes:\n  a: { type: service }\n  b: { type: service }\nedges:\n";
    let cfg = Config::from_yaml(&format!(
        "{base}  - {{ from: a, to: b }}\n  - {{ from: b, to: a, calls: 3 }}\n"
    ))
    .unwrap();
    assert_eq!(
        cfg.edges.iter().map(|e| e.calls.get()).collect::<Vec<_>>(),
        [1, 3]
    );
    let err =
        Config::from_yaml(&format!("{base}  - {{ from: a, to: b, calls: 0 }}\n")).unwrap_err();
    assert_eq!(err.line, Some(6), "{err}");
}

#[test]
fn node_kind_names_match_yaml() {
    assert_eq!(NodeKind::ExternalApi.as_str(), "external-api");
    assert_eq!(NodeKind::Monolith.to_string(), "monolith");
}

#[test]
fn keyword_named_nodes_load() {
    for name in ["true", "null", "yes", "on", "1"] {
        let cfg = Config::from_yaml(&format!(
            "version: 1\nnodes:\n  {name}: {{ type: service }}\n"
        ))
        .unwrap();
        assert!(
            cfg.nodes.contains_key(name),
            "{name}: {:?}",
            cfg.nodes.keys().collect::<Vec<_>>()
        );
    }
}

#[test]
fn schema_has_one_branch_per_kind() {
    let s = kaanal_core::config_schema();
    let branches = s["$defs"]["NodeConfig"]["oneOf"].as_array().unwrap();
    let consts: Vec<&str> = branches
        .iter()
        .map(|b| b["properties"]["type"]["const"].as_str().unwrap())
        .collect();
    assert_eq!(
        consts,
        [
            "gateway",
            "service",
            "monolith",
            "database",
            "cache",
            "external-api",
            "auth"
        ]
    );
    for b in branches {
        assert_eq!(
            b["additionalProperties"], false,
            "editors must flag typos: {b}"
        );
    }
}

#[test]
fn routes_key_is_replaced_by_endpoints() {
    let err = Config::from_yaml("version: 1\nnodes: {}\nroutes:\n  \"GET /health\": { body: 1 }\n")
        .unwrap_err();
    assert_eq!(
        err.to_string(),
        "3:1: routes: was replaced by endpoints: (see docs)"
    );
}

const ENDPOINTS: &str = r#"
version: 1
determinism: realistic
nodes:
  app: { type: service, chains: { reserve: [ db, { call: db, op: write } ] } }
  db: { type: database, flavor: postgres }
endpoints:
  "GET /orders/{id}":
    op: read
    chain:
      - cache: c
      - { call: svc, chain: [ { call: db, op: read, repeat: 2 } ] }
      - { call: svc, with: reserve }
      - { call: svc, via: "POST /orders" }
      - parallel: [ audit, [ pay, { call: db } ] ]
    override: { status: 201, body: { ok: true } }
    error_map: { db: 503, "5xx": 502, pay: { status: 502, body: { error: upstream } } }
"#;

#[test]
fn endpoints_parse_inline_with_every_step_shape() {
    let cfg = Config::from_yaml(ENDPOINTS).unwrap();
    assert_eq!(cfg.determinism, Determinism::Realistic);
    assert_eq!(cfg.seed_source(), SeedSource::Entropy);
    let Some(EndpointsConfig::Inline(eps)) = &cfg.endpoints else {
        panic!("inline: {:?}", cfg.endpoints)
    };
    let e = &eps["GET /orders/{id}"];
    assert_eq!(e.op, Some(Op::Read));
    let call = |node: &str| CallStep {
        node: node.into(),
        ..CallStep::default()
    };
    let bare = |node: &str| {
        StepConfig::Call(CallStep {
            bare: true,
            ..call(node)
        })
    };
    let want = vec![
        StepConfig::Cache { node: "c".into() },
        StepConfig::Call(CallStep {
            chain: Some(vec![StepConfig::Call(CallStep {
                op: Some(Op::Read),
                repeat: Some(2),
                ..call("db")
            })]),
            ..call("svc")
        }),
        StepConfig::Call(CallStep {
            with: Some("reserve".into()),
            ..call("svc")
        }),
        StepConfig::Call(CallStep {
            via: Some("POST /orders".into()),
            ..call("svc")
        }),
        StepConfig::Parallel(vec![
            Branch {
                steps: vec![bare("audit")],
                list: false,
            },
            Branch {
                steps: vec![bare("pay"), StepConfig::Call(call("db"))],
                list: true,
            },
        ]),
    ];
    assert_eq!(e.chain.as_ref(), Some(&want));
    assert_eq!(
        e.response,
        Some(Override {
            status: 201,
            body: json!({ "ok": true })
        })
    );
    assert_eq!(
        e.error_map["db"],
        ErrorMapping {
            status: 503,
            body: None
        }
    );
    assert_eq!(
        e.error_map["5xx"],
        ErrorMapping {
            status: 502,
            body: None
        }
    );
    assert_eq!(
        e.error_map["pay"],
        ErrorMapping {
            status: 502,
            body: Some(json!({ "error": "upstream" }))
        }
    );
    assert_eq!(
        cfg.nodes["app"].chains().unwrap()["reserve"],
        vec![
            bare("db"),
            StepConfig::Call(CallStep {
                op: Some(Op::Write),
                ..call("db")
            })
        ]
    );
    assert!(cfg.nodes["db"].chains().is_none());
}

#[test]
fn step_shape_errors_are_placed_at_the_step() {
    // Lines 1–5; each case is line 6. Positions come from serde's own marks (D1).
    let head = "version: 1\nnodes: {}\nendpoints:\n  \"GET /x\":\n    chain:\n";
    for (step, (line, col), want) in [
        (
            "      - { cache: c, repeat: 2 }\n",
            (6, 9),
            "a cache step takes no other fields (got 'repeat')",
        ),
        (
            "      - { parallel: [a, b], op: read }\n",
            (6, 9),
            "a parallel step takes no other fields (got 'op')",
        ),
        (
            "      - { op: read }\n",
            (6, 9),
            "a step needs one of call, cache, parallel",
        ),
        (
            "      - { call: a, bogus: 1 }\n",
            (6, 9),
            "unknown step field 'bogus'",
        ),
        (
            "      - { call: a, call: b }\n",
            (6, 9),
            "duplicate step field 'call'",
        ),
        ("      - { call: a, repeat: -1 }\n", (6, 28), "-1"),
        ("      - [a, b]\n", (6, 9), "expected a step"),
    ] {
        let err = Config::from_yaml(&format!("{head}{step}")).unwrap_err();
        assert_eq!((err.line, err.col), (Some(line), Some(col)), "{step}{err}");
        assert!(err.msg.contains(want), "{step}want '{want}' in: {err}");
    }
}

#[test]
fn endpoint_entries_are_strict() {
    let head = "version: 1\nnodes: {}\nendpoints:\n";
    let err = Config::from_yaml(&format!("{head}  \"GET /x\": {{ bogus: 1 }}\n")).unwrap_err();
    assert!(err.msg.contains("unknown field `bogus`"), "{err}");
    assert_eq!(err.line, Some(4), "{err}");
    let err =
        Config::from_yaml(&format!("{head}  \"GET /x\": {{}}\n  \"GET /x\": {{}}\n")).unwrap_err();
    assert!(err.msg.contains("duplicate endpoint 'GET /x'"), "{err}");
    assert_eq!(
        err.line,
        Some(4),
        "D19: placed at the start of the map: {err}"
    );
    let err = Config::from_yaml("version: 1\nnodes: {}\nendpoints: 5\n").unwrap_err();
    assert!(
        err.msg
            .contains("a path to an endpoints file or a map of endpoints"),
        "{err}"
    );
    assert_eq!((err.line, err.col), (Some(3), Some(12)), "{err}");
    let err = Config::from_yaml(&format!(
        "{head}  \"GET /x\": {{ error_map: {{ db: {{ status: 503, extra: 1 }} }} }}\n"
    ))
    .unwrap_err();
    assert!(err.msg.contains("unknown field `extra`"), "{err}");
    let err = Config::from_yaml("version: 1\nnodes: {}\ndeterminism: random\n").unwrap_err();
    assert!(
        err.msg.contains("controlled") && err.msg.contains("realistic"),
        "{err}"
    );
}

#[test]
fn chains_belong_to_services_and_monoliths() {
    Config::from_yaml("version: 1\nnodes:\n  m: { type: monolith, chains: { cron: [ x ] } }\n")
        .unwrap();
    let err =
        Config::from_yaml("version: 1\nnodes:\n  d: { type: database, chains: {} }\n").unwrap_err();
    assert!(err.msg.contains("unknown field 'chains'"), "{err}");
}

#[test]
fn endpoints_file_parses_and_places_shape_errors() {
    let m = parse_endpoints("# entries\n\"GET /x\": { op: read }\n").unwrap();
    assert_eq!(m.keys().collect::<Vec<_>>(), ["GET /x"]);
    for empty in ["", "# only a comment\n", "\u{feff}"] {
        assert!(parse_endpoints(empty).unwrap().is_empty(), "D19: {empty:?}");
    }
    let bad = "\"GET /x\":\n  chain:\n    - { cache: c, op: read }\n";
    let e = parse_endpoints(bad).unwrap_err();
    assert_eq!((e.line, e.col), (Some(3), Some(7)), "{e}");
    assert!(
        e.msg
            .contains("a cache step takes no other fields (got 'op')"),
        "{e}"
    );
    let windows = parse_endpoints(&format!("\u{feff}{}", bad.replace('\n', "\r\n"))).unwrap_err();
    assert_eq!(
        (windows.line, windows.col),
        (e.line, e.col),
        "R6.4: {windows}"
    );
    let top = parse_endpoints("- a\n").unwrap_err();
    assert!(top.msg.contains("map of endpoints"), "{top}");
}

#[test]
fn serde_errors_do_not_repeat_their_position_or_offer_routes() {
    // The position is in the error's own line/col; the message must not say it again.
    let e = Config::from_yaml("version: 1\nfoo: 2\n").unwrap_err();
    assert!(!e.msg.contains("at line"), "{}", e.msg);
    assert!(
        e.msg.contains("`endpoints`") && !e.msg.contains("routes"),
        "routes: is removed, so it is not a valid key to suggest: {}",
        e.msg
    );
    let e = parse_endpoints("\"GET /a\":\n  chain:\n    - { cache: c, op: read }\n").unwrap_err();
    assert_eq!((e.line, e.col), (Some(3), Some(7)), "{e}");
    assert!(!e.msg.contains("at line"), "{}", e.msg);
}

#[test]
fn auth_claims_and_scopes_parse() {
    let cfg = Config::from_yaml(
        "version: 1
nodes:
  login:
    type: auth
    token_ttl: 2h
    claims: { aud: my-app, level: 7, beta: true }
    users:
      - { username: alice, password: secret, roles: [admin], scopes: [read, write], claims: { email: a@x.dev } }
      - { username: bob, password: pw }
",
    )
    .unwrap();
    let NodeConfig::Auth(a) = &cfg.nodes["login"] else {
        panic!("auth")
    };
    assert_eq!(a.claims["aud"], json!("my-app"));
    assert_eq!(a.claims["level"], json!(7));
    assert_eq!(a.claims["beta"], json!(true));
    assert_eq!(a.users[0].scopes, ["read", "write"]);
    assert_eq!(a.users[0].claims["email"], json!("a@x.dev"));
    assert!(a.users[1].scopes.is_empty() && a.users[1].claims.is_empty());
    assert_eq!(a.token_ttl, Some(Duration::from_secs(7200)));
}

#[test]
fn reserved_claim_on_a_later_user_points_at_that_claim() {
    // Spec §7: an error on user #57 points at that key, not at `users:` (serde alone would).
    let mut yaml = String::from("version: 1\nnodes:\n  login:\n    type: auth\n    users:\n");
    for i in 0..58 {
        let extra = if i == 57 { ", claims: { iss: x }" } else { "" };
        let _ = writeln!(yaml, "      - {{ username: u{i}, password: p{extra} }}");
    }
    let err = Config::from_yaml(&yaml).unwrap_err();
    assert_eq!((err.line, err.col), (Some(63), Some(49)), "{err}");
    assert!(
        err.msg
            .starts_with("node 'login': users[57].claims: 'iss' is set by Kaanal"),
        "{err}"
    );
}

#[test]
fn auth_rules_are_checked_after_parsing_and_placed() {
    let long = "x".repeat(257);
    let many_roles = (0..33)
        .map(|i| format!("r{i}"))
        .collect::<Vec<_>>()
        .join(", ");
    let many_claims = (0..33)
        .map(|i| format!("c{i}: 1"))
        .collect::<Vec<_>>()
        .join(", ");
    let cases: Vec<(String, (usize, usize), &str)> = vec![
        (
            "    claims: { sub: x }\n".into(),
            (5, 15),
            "claims: 'sub' is set by Kaanal",
        ),
        (
            "    claims: { a: [1] }\n".into(),
            (5, 18),
            "claims.a: must be a string",
        ),
        (
            "    claims: { a: null }\n".into(),
            (5, 18),
            "claims.a: must be a string",
        ),
        (
            format!("    claims: {{ a: {long} }}\n"),
            (5, 18),
            "claims.a: must be a string (at most 256 bytes)",
        ),
        (
            format!("    claims: {{ {many_claims} }}\n"),
            (5, 5),
            "claims: at most 32 claims (got 33)",
        ),
        (
            "    users:\n      - { username: \"\", password: p }\n".into(),
            (6, 21),
            "users[0].username: must be 1 to 256 bytes",
        ),
        (
            format!("    users:\n      - {{ username: a, password: {long} }}\n"),
            (6, 34),
            "users[0].password: must be at most 256 bytes",
        ),
        (
            format!("    users:\n      - {{ username: a, password: p, roles: [{many_roles}] }}\n"),
            (6, 37),
            "users[0].roles: at most 32 entries (got 33)",
        ),
        (
            "    users:\n      - { username: a, password: p, scopes: [\"bad scope\"] }\n".into(),
            (6, 46),
            "users[0].scopes[0]: must be 1 to 256 printable ASCII characters",
        ),
        (
            "    users:\n      - { username: a, password: p, scopes: [\"\"] }\n".into(),
            (6, 46),
            "users[0].scopes[0]: must be 1 to 256 printable ASCII characters",
        ),
    ];
    for (body, (line, col), want) in cases {
        let yaml = format!("version: 1\nnodes:\n  login:\n    type: auth\n{body}");
        let err = Config::from_yaml(&yaml).unwrap_err();
        assert_eq!(
            (err.line, err.col),
            (Some(line), Some(col)),
            "{body}: {err}"
        );
        assert!(err.msg.contains(want), "{body}: want '{want}' in: {err}");
        // A rejected value (a password above all) is never echoed.
        assert!(!err.msg.contains("xxxx"), "{body}: value echoed: {err}");
    }
    // 1000 users load; 1001 are rejected at `users:`.
    let users = |n: usize| {
        let mut s = String::new();
        for i in 0..n {
            let _ = writeln!(s, "      - {{ username: u{i}, password: p }}");
        }
        s
    };
    let head = "version: 1\nnodes:\n  login:\n    type: auth\n    users:\n";
    Config::from_yaml(&format!("{head}{}", users(1000))).unwrap();
    let err = Config::from_yaml(&format!("{head}{}", users(1001))).unwrap_err();
    assert_eq!((err.line, err.col), (Some(5), Some(5)), "{err}");
    assert!(err.msg.contains("at most 1000 users (got 1001)"), "{err}");
}

#[test]
fn a_non_text_password_is_rejected_at_its_value_without_echo() {
    // serde alone would say `invalid type: integer `4711`` at the node's key.
    let cases = [
        ("4711", "a number"),
        ("1.5", "a number"),
        ("true", "a boolean"),
        ("null", "null"),
    ];
    for (pw, kind) in cases {
        for body in [
            format!("      - {{ username: a, password: {pw} }}\n"),
            format!("      - username: a\n        password: {pw}\n"),
        ] {
            let yaml = format!("version: 1\nnodes:\n  login:\n    type: auth\n    users:\n{body}");
            let err = Config::from_yaml(&yaml).unwrap_err();
            let (line, col) = if body.starts_with("      - {") {
                (6, 34)
            } else {
                (7, 19)
            };
            assert_eq!((err.line, err.col), (Some(line), Some(col)), "{pw}: {err}");
            assert!(
                err.msg.contains(&format!(
                    "users[0].password: must be text, not {kind} (put it in quotes)"
                )),
                "{pw}: {err}"
            );
            // (`null` names its own kind in the message; the others must not appear at all.)
            assert!(
                pw == "null" || !err.msg.contains(pw),
                "{pw}: value echoed: {err}"
            );
        }
    }
    // A quoted number is text and loads; so does a number with a leading zero (this YAML
    // reader keeps `0123` and `007` as text), zeros intact.
    for (written, want) in [("\"4711\"", "4711"), ("0123", "0123"), ("007", "007")] {
        let cfg = Config::from_yaml(&format!(
            "version: 1\nnodes:\n  login: {{ type: auth, users: [ {{ username: a, password: {written} }} ] }}\n"
        ))
        .unwrap();
        let Some(NodeConfig::Auth(a)) = cfg.nodes.get("login") else {
            panic!("login is an auth node")
        };
        assert_eq!(a.users[0].password, want, "{written}");
    }
}

#[test]
fn token_ttl_must_be_whole_seconds_of_at_least_one() {
    // P1: placed at the value (5:16) by the per-field probe, like the 1-day cap.
    for bad in ["1500ms", "0s", "500ms"] {
        let err = Config::from_yaml(&format!(
            "version: 1\nnodes:\n  login:\n    type: auth\n    token_ttl: {bad}\n"
        ))
        .unwrap_err();
        assert_eq!((err.line, err.col), (Some(5), Some(16)), "{bad}: {err}");
        assert!(
            err.msg
                .contains("token_ttl must be whole seconds, at least 1s"),
            "{bad}: {err}"
        );
    }
    for ok in ["1s", "90s", "24h"] {
        Config::from_yaml(&format!(
            "version: 1\nnodes:\n  login: {{ type: auth, token_ttl: {ok} }}\n"
        ))
        .unwrap();
    }
}

#[test]
fn user_debug_hides_the_password() {
    let cfg = Config::from_yaml(
        "version: 1\nnodes:\n  login: { type: auth, users: [ { username: alice, password: hunter2-pw } ] }\n",
    )
    .unwrap();
    let text = format!("{cfg:?}");
    assert!(
        text.contains("alice") && text.contains("<redacted>"),
        "{text}"
    );
    assert!(!text.contains("hunter2"), "{text}");
}
