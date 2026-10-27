use std::fmt::Write as _;

use kaanal_core::config::CacheFlavor;
use kaanal_core::topology::CacheModel;
use kaanal_core::{Admission, Config, Dist, NodeKind, Topology, TopologyError, presets};

const BASE: &str = r"
version: 1
nodes:
  nginx: { type: gateway }
  app:   { type: service, spec: openapi.yaml }
  db:    { type: database, ops: { read: 4ms, write: 8ms, scan: 30ms } }
edges:
  - { from: nginx, to: app }
  - { from: app, to: db }
";

fn build(yaml: &str) -> Result<Topology, TopologyError> {
    Topology::build(&Config::from_yaml(yaml).unwrap())
}

#[test]
fn builds_base() {
    let t = build(BASE).unwrap();
    assert_eq!(t.nodes.len(), 3);
    assert_eq!(t.nodes[t.gateway.unwrap()].name, "nginx");
    let app = t.id("app").unwrap();
    let db = t.id("db").unwrap();
    assert_eq!(t.nodes[db].kind, NodeKind::Database);
    assert!(t.edge(t.gateway.unwrap(), app).is_some());
    assert_eq!(t.out_edges(app).map(|e| e.to).collect::<Vec<_>>(), vec![db]);
}

#[test]
fn rejects_cycle() {
    let yaml = format!("{BASE}  - {{ from: db, to: app }}\n");
    assert_eq!(
        build(&yaml).unwrap_err(),
        TopologyError::Cycle(vec!["app".into(), "db".into(), "app".into()])
    );
}

#[test]
fn rejects_unknown_node_in_edge() {
    let yaml = format!("{BASE}  - {{ from: app, to: redis }}\n");
    assert_eq!(
        build(&yaml).unwrap_err(),
        TopologyError::UnknownNode {
            edge: 2,
            name: "redis".into()
        }
    );
}

#[test]
fn second_gateway_is_rejected_by_name() {
    let yaml = BASE.replace("nodes:\n", "nodes:\n  lb: { type: gateway }\n");
    assert_eq!(
        build(&yaml).unwrap_err(),
        TopologyError::SecondGateway("nginx".into())
    );
}

#[test]
fn gateway_is_optional() {
    // R1.23: service -> database with no gateway, and a lone monolith, both build.
    let t = build(
        "version: 1\nnodes:\n  app: { type: service, spec: openapi.yaml }\n  db: { type: database, flavor: postgres }\nedges:\n  - { from: app, to: db }\n",
    )
    .unwrap();
    assert_eq!(t.gateway, None);
    let t = build(
        "version: 1\nnodes:\n  shop: { type: monolith, spec: openapi.yaml, flavor: mysql }\n",
    )
    .unwrap();
    assert_eq!(t.gateway, None);
    assert!(
        t.nodes[0].ops.is_some(),
        "flavor gives the monolith inline ops"
    );
}

#[test]
fn rejects_database_without_ops() {
    let yaml = BASE.replace(
        "db:    { type: database, ops: { read: 4ms, write: 8ms, scan: 30ms } }",
        "db: { type: database }",
    );
    assert_eq!(
        build(&yaml).unwrap_err(),
        TopologyError::MissingOps("db".into())
    );
}

#[test]
fn rejects_service_spec_without_gateway_edge() {
    let yaml = BASE.replace("  - { from: nginx, to: app }\n", "");
    assert_eq!(
        build(&yaml).unwrap_err(),
        TopologyError::MissingGatewayEdge("app".into())
    );
}

#[test]
fn services_may_call_services() {
    let yaml = BASE.replace("nodes:\n", "nodes:\n  users: { type: service }\n")
        + "  - { from: app, to: users }\n";
    let t = build(&yaml).unwrap();
    assert_eq!(t.out_edges(t.id("app").unwrap()).count(), 2);
}

#[test]
fn errors_display_human_text() {
    let e = TopologyError::Cycle(vec!["a".into(), "b".into(), "a".into()]);
    assert_eq!(e.to_string(), "cycle: a -> b -> a");
}

#[test]
fn rejects_duplicate_edge() {
    let yaml = format!("{BASE}  - {{ from: app, to: db }}\n");
    assert!(matches!(
        build(&yaml).unwrap_err(),
        TopologyError::DuplicateEdge { edge: 2, .. }
    ));
}

#[test]
fn rejects_edges_the_executor_would_ignore() {
    let yaml = format!("{BASE}  - {{ from: nginx, to: db }}\n");
    assert!(matches!(
        build(&yaml).unwrap_err(),
        TopologyError::ForbiddenEdge { from, to_kind: NodeKind::Database, .. } if from == "nginx"
    ));
}

const KINDS: &str = "version: 1
nodes:
  gw:     { type: gateway }
  svc:    { type: service }
  svc2:   { type: service }
  mono:   { type: monolith }
  mono2:  { type: monolith }
  db:     { type: database, flavor: postgres }
  db2:    { type: database, flavor: mysql }
  cache:  { type: cache, flavor: redis }
  cache2: { type: cache, flavor: memcached }
  ext:    { type: external-api }
  ext2:   { type: external-api }
  auth:   { type: auth }
";

/// `KINDS` plus one edge `from -> to`.
fn with_edge(from: &str, to: &str) -> Result<Topology, TopologyError> {
    build(&format!(
        "{KINDS}edges:\n  - {{ from: {from}, to: {to} }}\n"
    ))
}

#[test]
fn edge_table_matches_r1_24() {
    let kinds = ["gw", "svc", "mono", "db", "cache", "ext", "auth"];
    let allowed = |from: &str, to: &str| match from {
        "gw" => matches!(to, "svc" | "mono" | "auth"),
        "svc" | "mono" => matches!(to, "svc" | "mono" | "db" | "cache" | "ext" | "auth"),
        "auth" => matches!(to, "svc" | "mono" | "db" | "cache" | "ext"),
        _ => false,
    };
    for from in kinds {
        for to in kinds {
            if matches!((from, to), ("gw", "gw") | ("auth", "auth")) {
                continue; // one gateway and one auth node only: see the self-loop and second-node tests
            }
            // Twins (svc2, …) stand in for "another node of the same kind".
            let target = if from == to {
                format!("{to}2")
            } else {
                to.to_string()
            };
            let got = with_edge(from, &target);
            if allowed(from, to) {
                assert!(got.is_ok(), "{from} -> {target}: {got:?}");
            } else {
                assert!(
                    matches!(got, Err(TopologyError::ForbiddenEdge { .. })),
                    "{from} -> {target}: {got:?}"
                );
            }
        }
    }
}

#[test]
fn forbidden_edge_names_what_is_allowed() {
    assert_eq!(
        with_edge("db", "svc").unwrap_err().to_string(),
        "edge #0: database 'db' cannot call service 'svc'; database may call: nothing"
    );
    let e = with_edge("gw", "db").unwrap_err();
    assert!(
        e.to_string()
            .ends_with("gateway may call: service, monolith, auth"),
        "{e}"
    );
}

#[test]
fn a_topology_needs_somewhere_to_enter() {
    assert_eq!(
        build("version: 1\nnodes: {}\n").unwrap_err(),
        TopologyError::NoEntry
    );
    assert_eq!(
        build("version: 1\nnodes:\n  db: { type: database, flavor: postgres }\n").unwrap_err(),
        TopologyError::NoEntry
    );
    // A gateway alone is fine: it answers static routes.
    assert!(build("version: 1\nnodes:\n  gw: { type: gateway }\n").is_ok());
}

#[test]
fn self_and_duplicate_edges_are_rejected() {
    assert_eq!(
        with_edge("svc", "svc").unwrap_err(),
        TopologyError::SelfLoop {
            edge: 0,
            name: "svc".into()
        }
    );
    let yaml = format!("{KINDS}edges:\n  - {{ from: svc, to: db }}\n  - {{ from: svc, to: db }}\n");
    assert_eq!(
        build(&yaml).unwrap_err(),
        TopologyError::DuplicateEdge {
            edge: 1,
            from: "svc".into(),
            to: "db".into()
        }
    );
}

#[test]
fn cycle_through_auth_is_rejected() {
    let yaml =
        format!("{KINDS}edges:\n  - {{ from: svc, to: auth }}\n  - {{ from: auth, to: svc }}\n");
    assert_eq!(
        build(&yaml).unwrap_err(),
        TopologyError::Cycle(vec!["svc".into(), "auth".into(), "svc".into()])
    );
}

#[test]
fn a_long_chain_of_services_does_not_overflow_the_stack() {
    // R6.12: the cycle check is iterative. 20 000 services in a line, built from YAML text
    // (a flat mapping is not deeply nested), then closed into a cycle.
    let n = 20_000;
    let mut yaml = String::from("version: 1\nnodes:\n");
    for i in 0..n {
        writeln!(yaml, "  s{i}: {{ type: service }}").unwrap();
    }
    yaml.push_str("edges:\n");
    for i in 0..n - 1 {
        writeln!(yaml, "  - {{ from: s{i}, to: s{} }}", i + 1).unwrap();
    }
    let ok = Config::from_yaml(&yaml).unwrap();
    writeln!(yaml, "  - {{ from: s{}, to: s0 }}", n - 1).unwrap();
    let cyclic = Config::from_yaml(&yaml).unwrap();
    // A 256 KiB stack: a recursive check (one frame per edge) overflows it deterministically,
    // in debug and release alike; the iterative one does not care.
    std::thread::Builder::new()
        .stack_size(256 * 1024)
        .spawn(move || {
            assert!(Topology::build(&ok).is_ok());
            let Err(TopologyError::Cycle(path)) = Topology::build(&cyclic) else {
                panic!("expected a cycle");
            };
            assert_eq!(path.len(), n + 1);
            assert_eq!((path[0].as_str(), path[n].as_str()), ("s0", "s0"));
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn cycle_path_starts_at_the_loop_not_the_dfs_root() {
    // x -> y -> z -> y: the DFS starts at x, but the reported cycle is y -> z -> y.
    let yaml = "version: 1\nnodes:\n  x: { type: service }\n  y: { type: service }\n  z: { type: service }\nedges:\n  - { from: x, to: y }\n  - { from: y, to: z }\n  - { from: z, to: y }\n";
    assert_eq!(
        build(yaml).unwrap_err(),
        TopologyError::Cycle(vec!["y".into(), "z".into(), "y".into()])
    );
}

#[test]
fn a_cycle_only_reachable_from_a_later_start_node_is_found() {
    // a is a clean chain; the loop sits in a separate component that starts at node c.
    let yaml = "version: 1\nnodes:\n  a: { type: service }\n  b: { type: service }\n  c: { type: service }\n  d: { type: service }\nedges:\n  - { from: a, to: b }\n  - { from: c, to: d }\n  - { from: d, to: c }\n";
    assert_eq!(
        build(yaml).unwrap_err(),
        TopologyError::Cycle(vec!["c".into(), "d".into(), "c".into()])
    );
}

#[test]
fn monolith_works_both_ways() {
    let base = "version: 1\nnodes:\n  db: { type: database, flavor: postgres }\n";
    // Inline ops only.
    let t = build(&format!(
        "{base}  shop: {{ type: monolith, ops: {{ read: 1ms, write: 2ms, scan: 3ms }} }}\n"
    ))
    .unwrap();
    assert!(t.nodes[t.id("shop").unwrap()].ops.is_some());
    // Calls a database, no ops of its own.
    let t = build(&format!(
        "{base}  shop: {{ type: monolith }}\nedges:\n  - {{ from: shop, to: db }}\n"
    ))
    .unwrap();
    let shop = t.id("shop").unwrap();
    assert!(t.nodes[shop].ops.is_none());
    assert_eq!(t.out_edges(shop).count(), 1);
    // Both: allowed in A1; A2's chain rules pick which one an endpoint uses.
    let t = build(&format!(
        "{base}  shop: {{ type: monolith, flavor: mysql }}\nedges:\n  - {{ from: shop, to: db }}\n"
    ))
    .unwrap();
    let shop = t.id("shop").unwrap();
    assert!(t.nodes[shop].ops.is_some() && t.out_edges(shop).count() == 1);
    // Partial ops without a flavor are incomplete.
    assert_eq!(
        build(&format!(
            "{base}  shop: {{ type: monolith, ops: {{ read: 1ms }} }}\n"
        ))
        .unwrap_err(),
        TopologyError::MissingOps("shop".into())
    );
}

#[test]
fn cache_costs_come_from_the_flavor_or_the_user() {
    let t = build(
        "version: 1\nnodes:\n  app: { type: service }\n  c: { type: cache, flavor: redis, hit_ratio: 0.5 }\nedges:\n  - { from: app, to: c }\n",
    )
    .unwrap();
    let preset = presets::cache_defaults(CacheFlavor::Redis);
    assert_eq!(
        t.nodes[t.id("c").unwrap()].cache,
        Some(CacheModel {
            hit_ratio: 0.5,
            hit: Dist::from_config(Some(&preset.hit)),
            miss: Dist::from_config(Some(&preset.miss)),
        })
    );
    assert_eq!(
        build("version: 1\nnodes:\n  app: { type: service }\n  c: { type: cache, hit: 1ms }\n")
            .unwrap_err(),
        TopologyError::MissingCache("c".into())
    );
}

#[test]
fn edge_call_ratio_is_carried() {
    let t = build(&format!(
        "{KINDS}edges:\n  - {{ from: svc, to: db, calls: 3 }}\n  - {{ from: svc, to: cache }}\n"
    ))
    .unwrap();
    assert_eq!(
        t.edges.iter().map(|e| e.calls.get()).collect::<Vec<_>>(),
        [3, 1]
    );
}

#[test]
fn second_auth_is_rejected_by_name() {
    let yaml = "version: 1\nnodes:\n  app: { type: service }\n  login: { type: auth }\n  sso: { type: auth }\n";
    let cfg = Config::from_yaml(yaml).unwrap();
    let e = Topology::build(&cfg).unwrap_err();
    assert_eq!(e, TopologyError::SecondAuth("sso".into()));
    assert_eq!(
        e.to_string(),
        "node 'sso': a topology has at most one auth node"
    );
    let placed = e.located(&cfg).in_source(yaml);
    assert_eq!((placed.line, placed.col), (Some(5), Some(3)), "{placed}");
}

#[test]
fn an_auth_only_topology_is_valid() {
    // X11: a frontend plus a login mock needs nothing else.
    let t = build("version: 1\nnodes:\n  login: { type: auth, proc: 5ms }\n").unwrap();
    assert_eq!((t.gateway, t.auth), (None, Some(0)));
    assert_eq!(build(BASE).unwrap().auth, None);
    assert_eq!(
        TopologyError::NoEntry.to_string(),
        "a topology needs a gateway, a service, a monolith or an auth node (requests enter there)"
    );
}

#[test]
fn admission_and_rate_limit_are_built_from_the_config() {
    let yaml = "version: 1\nnodes:\n  a: { type: service, concurrency: 3 }\n  b: { type: service, concurrency: { limit: 2, queue: 4, overflow_status: 502 } }\n  db: { type: database, flavor: postgres, pool: { size: 5, queue: 7 }, rate_limit: { rps: 4, burst: 2 } }\n  c: { type: service }\n";
    let t = Topology::build(&Config::from_yaml(yaml).unwrap()).unwrap();
    let adm = |n: &str| t.nodes[t.id(n).unwrap()].admission.clone();
    assert_eq!(
        adm("a"),
        Some(Admission {
            slots: 3,
            queue: None,
            overflow_status: 503
        })
    );
    assert_eq!(
        adm("b"),
        Some(Admission {
            slots: 2,
            queue: Some(4),
            overflow_status: 502
        })
    );
    assert_eq!(
        adm("db"),
        Some(Admission {
            slots: 5,
            queue: Some(7),
            overflow_status: 503
        })
    );
    assert_eq!(adm("c"), None);
    let rl = t.nodes[t.id("db").unwrap()].rate_limit.unwrap();
    assert_eq!(
        (rl.interval_ns, rl.burst),
        (250_000_000, 2),
        "4 rps is one token per 250 ms"
    );
    assert!(t.nodes[t.id("a").unwrap()].rate_limit.is_none());
}

#[test]
fn fractional_rps_rounds_to_the_nearest_nanosecond() {
    let t = Topology::build(
        &Config::from_yaml(
            "version: 1\nnodes:\n  g: { type: gateway, rate_limit: { rps: 3, burst: 1 } }\n",
        )
        .unwrap(),
    )
    .unwrap();
    assert_eq!(t.nodes[0].rate_limit.unwrap().interval_ns, 333_333_333);
    let t = Topology::build(&Config::from_yaml("version: 1\nnodes:\n  g: { type: gateway, rate_limit: { rps: 1000000000, burst: 1 } }\n").unwrap()).unwrap();
    assert_eq!(t.nodes[0].rate_limit.unwrap().interval_ns, 1);
}

#[test]
fn a_code_built_config_with_a_bad_edge_prob_is_rejected() {
    for bad in [f64::NAN, 0.0, -1.0, 2.0, f64::INFINITY] {
        let mut cfg = Config::from_yaml(
            "version: 1\nnodes:\n  a: { type: service }\n  b: { type: database, flavor: postgres }\nedges:\n  - { from: a, to: b }\n",
        )
        .unwrap();
        cfg.edges[0].prob = Some(bad);
        let err = Topology::build(&cfg).unwrap_err();
        assert!(
            matches!(err, TopologyError::BadProb { edge: 0, .. }),
            "{bad}"
        );
        assert!(
            err.to_string().contains("prob must be above 0"),
            "{bad}: {err}"
        );
    }
}
