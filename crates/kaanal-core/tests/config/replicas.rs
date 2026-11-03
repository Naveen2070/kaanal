//! `replicas` and `balance`: how many copies a node has and how traffic picks one.

use kaanal_core::{Balance, Config, NodeKind, Topology};

fn load(nodes: &str, edges: &str) -> Result<Config, String> {
    Config::from_yaml(&format!("version: 1\nnodes:\n{nodes}edges:\n{edges}"))
        .map_err(|e| e.to_string())
}

const DB: &str = "  app: { type: service }\n";

fn db(fields: &str) -> String {
    format!("{DB}  db: {{ type: database, flavor: postgres{fields} }}\n")
}

const EDGE: &str = "  - { from: app, to: db }\n";

#[test]
fn replicas_range_is_1_to_64() {
    for bad in ["0", "65", "1000"] {
        let err = load(&db(&format!(", replicas: {bad}")), EDGE).unwrap_err();
        assert!(
            err.contains("replicas must be between 1 and 64"),
            "{bad}: {err}"
        );
        assert!(
            err.starts_with("4:"),
            "{bad} is positioned on its line: {err}"
        );
    }
    for ok in ["1", "64"] {
        load(&db(&format!(", replicas: {ok}")), EDGE).unwrap();
    }
}

#[test]
fn balance_parses_its_three_forms() {
    let b = |s: &str| {
        let c = load(&db(&format!(", replicas: 3, balance: {s}")), EDGE).unwrap();
        c.nodes["db"].balance().cloned()
    };
    assert_eq!(b("round-robin"), Some(Balance::RoundRobin));
    assert_eq!(b("random"), Some(Balance::Random));
    assert_eq!(
        b("{ weighted: [3, 1, 1] }"),
        Some(Balance::Weighted(vec![3, 1, 1]))
    );
    assert_eq!(load(&db(""), EDGE).unwrap().nodes["db"].balance(), None);
}

#[test]
fn a_bad_balance_says_what_is_expected() {
    for bad in [
        "sticky",
        "weighted",
        "[1, 2]",
        "{ weighted: [1], extra: 1 }",
    ] {
        let err = load(&db(&format!(", replicas: 1, balance: {bad}")), EDGE).unwrap_err();
        assert!(
            err.contains("round-robin") || err.contains("weights") || err.contains("unknown field"),
            "{bad}: {err}"
        );
    }
}

#[test]
fn weighted_length_must_equal_replicas() {
    let err = load(&db(", replicas: 3, balance: { weighted: [1, 1] }"), EDGE).unwrap_err();
    assert!(err.contains("2 weights for 3 replicas"), "{err}");
    assert!(err.starts_with("4:"), "{err}");
}

#[test]
fn weighted_needs_one_positive_weight() {
    let err = load(&db(", replicas: 2, balance: { weighted: [0, 0] }"), EDGE).unwrap_err();
    assert!(err.contains("at least one weight above 0"), "{err}");
    load(&db(", replicas: 2, balance: { weighted: [0, 5] }"), EDGE).unwrap();
}

#[test]
fn balance_without_replicas_is_accepted_and_inert() {
    let c = load(&db(", balance: random"), EDGE).unwrap();
    assert_eq!(c.nodes["db"].replicas(), 1);
    let t = Topology::build(&c).unwrap();
    assert_eq!(
        (t.nodes[t.id("db").unwrap()].replicas, &t.nodes[1].balance),
        (1, &Balance::Random)
    );
}

#[test]
fn an_edge_balance_overrides_the_node_and_is_checked_against_the_callee() {
    let nodes = db(", replicas: 2, balance: random");
    let c = load(
        &nodes,
        "  - { from: app, to: db, balance: { weighted: [1, 3] } }\n",
    )
    .unwrap();
    let t = Topology::build(&c).unwrap();
    assert_eq!(t.edges[0].balance, Some(Balance::Weighted(vec![1, 3])));
    let err = load(
        &nodes,
        "  - { from: app, to: db, balance: { weighted: [1, 3, 5] } }\n",
    )
    .unwrap_err();
    assert!(err.contains("3 weights for 2 replicas"), "{err}");
    assert!(err.starts_with("6:"), "{err}");
    let t = Topology::build(&load(&nodes, EDGE).unwrap()).unwrap();
    assert_eq!(t.edges[0].balance, None);
}

#[test]
fn replicas_parse_on_every_node_kind() {
    let kinds = [
        ("gateway", "", NodeKind::Gateway),
        ("service", "", NodeKind::Service),
        ("monolith", ", flavor: mysql", NodeKind::Monolith),
        ("database", ", flavor: postgres", NodeKind::Database),
        ("cache", ", flavor: redis", NodeKind::Cache),
        ("external-api", "", NodeKind::ExternalApi),
        ("auth", "", NodeKind::Auth),
    ];
    for (kind, extra, want) in kinds {
        let src = format!("  n: {{ type: {kind}{extra}, replicas: 3, balance: random }}\n");
        let c = load(&src, "").unwrap_or_else(|e| panic!("{kind}: {e}"));
        let n = &c.nodes["n"];
        assert_eq!((n.kind(), n.replicas()), (want, 3), "{kind}");
        assert_eq!(n.balance(), Some(&Balance::Random), "{kind}");
    }
}

#[test]
fn least_conn_and_p2c_parse_as_balances_and_a_spread_cannot_use_them() {
    let nodes =
        "  gw: { type: gateway }\n  app: { type: service, replicas: 2, balance: least-conn }\n";
    let c = Config::from_yaml(&format!(
        "version: 1\nnodes:\n{nodes}edges:\n  - {{ from: gw, to: app, balance: p2c }}\n"
    ))
    .unwrap();
    assert_eq!(c.nodes["app"].balance(), Some(&Balance::LeastConn));
    assert_eq!(c.edges[0].balance, Some(Balance::P2c));
    assert!(Balance::P2c.is_dynamic() && !Balance::Random.is_dynamic());
    let bad = "version: 1\nnodes:\n  gw: { type: gateway }\n  lb: { type: load-balancer, spread: least-conn }\n  a: { type: service }\nedges:\n  - { from: gw, to: lb }\n  - { from: lb, to: a }\n";
    let err = Config::from_yaml(bad).unwrap_err().to_string();
    assert!(
        err.contains("spread") && err.contains("least-conn"),
        "{err}"
    );
}
