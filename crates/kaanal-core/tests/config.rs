use kaanal_core::config::{CacheFlavor, DbFlavor, DistConfig, NodeConfig};
use kaanal_core::{Config, NodeKind, Topology};
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
routes:
  "GET /health": { body: { ok: true } }
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
    assert_eq!(cfg.routes["GET /health"].status, 200);
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
