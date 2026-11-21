//! The planner's move rules (RULES 141, Step 9c): which size knob each node offers, and that a figure
//! the developer wrote by hand is never overridden. The oracle is the rule table and the AWS lineup
//! (m5.large has 2 vCPU, m5.xlarge 4), not a number the code printed.

use kaanal_core::Config;
use kaanal_core::balance::MAX_REPLICAS;
use kaanal_core::config::CpuConfig;
use kaanal_core::instances::{lookup, series};
use kaanal_core::resize::{
    Move, MoveKind, apply, copies_only_reason, moves_for, moves_for_oom, why_fixed,
};

fn cfg(nodes: &str) -> Config {
    Config::from_yaml(&format!("version: 1\nnodes:\n{nodes}")).unwrap_or_else(|e| panic!("{e}"))
}

fn kinds(c: &Config, node: &str) -> Vec<MoveKind> {
    moves_for(c, node, &[])
        .into_iter()
        .map(|m| m.kind)
        .collect()
}

fn copies(from: u16, to: u16) -> MoveKind {
    MoveKind::Copies { from, to }
}

fn class(from: &str, to: &str) -> MoveKind {
    MoveKind::Class {
        from: from.into(),
        to: to.into(),
    }
}

fn is_size(k: &MoveKind) -> bool {
    matches!(
        k,
        MoveKind::Class { .. } | MoveKind::Cores { .. } | MoveKind::Memory { .. }
    )
}

#[test]
fn a_node_with_an_instance_moves_by_class_and_copies_never_cores() {
    let c = cfg("  app: { type: service, proc: 5ms, instance: aws/m5.large, replicas: 2 }\n");
    assert_eq!(
        kinds(&c, "app"),
        vec![class("aws/m5.large", "aws/m5.xlarge"), copies(2, 3)]
    );
    assert_eq!(why_fixed(&c, "app", &[]), None);
    assert_eq!(copies_only_reason(&c, "app", &[]), None);
    // memory on request: the next class up again, never a raw limit on a node whose limit is the class's
    let c = cfg(
        "  app: { type: service, proc: 5ms, instance: aws/m5.large, memory: { per_request: 1MiB } }\n",
    );
    let oom: Vec<MoveKind> = moves_for_oom(&c, "app", &[])
        .into_iter()
        .map(|m| m.kind)
        .collect();
    assert_eq!(
        oom,
        vec![class("aws/m5.large", "aws/m5.xlarge"), copies(1, 2)]
    );
}

#[test]
fn a_node_with_no_instance_moves_by_cores_and_copies() {
    let c = cfg("  app: { type: service, proc: 5ms, cpu: { cores: 2 } }\n");
    assert_eq!(
        kinds(&c, "app"),
        vec![MoveKind::Cores { from: 2.0, to: 4.0 }, copies(1, 2)]
    );
    // a fraction steps to the next whole step, the largest step has no next
    let c = cfg("  app: { type: service, proc: 5ms, cpu: { cores: 0.5 } }\n");
    assert_eq!(kinds(&c, "app")[0], MoveKind::Cores { from: 0.5, to: 1.0 });
    let c = cfg("  app: { type: service, proc: 5ms, cpu: { cores: 64 } }\n");
    assert_eq!(kinds(&c, "app"), vec![copies(1, 2)]);
    // no cpu written: the cores are not limited, so a core count would only cap it
    let c = cfg("  app: { type: service, proc: 5ms }\n");
    assert_eq!(kinds(&c, "app"), vec![copies(1, 2)]);
    assert!(copies_only_reason(&c, "app", &[]).is_some());
    // memory on request: the limit doubled
    let c = cfg(
        "  app: { type: service, proc: 5ms, cpu: { cores: 2 }, memory: { limit: 512MiB, per_request: 1MiB } }\n",
    );
    let oom: Vec<MoveKind> = moves_for_oom(&c, "app", &[])
        .into_iter()
        .map(|m| m.kind)
        .collect();
    assert_eq!(
        oom,
        vec![
            MoveKind::Memory {
                from: 512 << 20,
                to: 1024 << 20
            },
            copies(1, 2)
        ]
    );
}

#[test]
fn a_cache_moves_by_copies_only() {
    let c = cfg(
        "  c: { type: cache, flavor: redis, hosting: managed, instance: aws/cache.m5.large, replicas: 2 }\n",
    );
    assert_eq!(kinds(&c, "c"), vec![copies(2, 3)]);
    let oom: Vec<MoveKind> = moves_for_oom(&c, "c", &[])
        .into_iter()
        .map(|m| m.kind)
        .collect();
    assert_eq!(oom, vec![copies(2, 3)]);
    assert!(copies_only_reason(&c, "c", &[]).is_some());
}

/// Review Focus 3: explicit cpu or memory on a node with a class, or a custom instance, keeps the
/// figure: no class or core move, copies only, and the reason is given.
#[test]
fn a_hand_written_figure_is_never_overridden() {
    let cases = [
        "version: 1\nnodes:\n  app: { type: service, proc: 5ms, instance: aws/m5.large, cpu: { cores: 3 } }\n",
        "version: 1\nnodes:\n  app: { type: service, proc: 5ms, instance: aws/m5.large, memory: { limit: 1GiB, per_request: 1MiB } }\n",
        "version: 1\ninstances:\n  rack: { cores: 8, memory: 32GiB }\nnodes:\n  app: { type: service, proc: 5ms, instance: rack }\n",
        "version: 1\ninstances:\n  rack: { cores: 8, memory: 32GiB }\nnodes:\n  app: { type: service, proc: 5ms, instance: rack, cpu: { cores: 4 }, memory: { per_request: 1MiB } }\n",
    ];
    for y in cases {
        let c = Config::from_yaml(y).unwrap_or_else(|e| panic!("{e}"));
        let moves = kinds(&c, "app");
        let oom: Vec<MoveKind> = moves_for_oom(&c, "app", &[])
            .into_iter()
            .map(|m| m.kind)
            .collect();
        assert_eq!(moves, vec![copies(1, 2)], "{y}");
        assert_eq!(oom, vec![copies(1, 2)], "{y}");
        assert!(!moves.iter().any(is_size), "{y}");
        assert!(copies_only_reason(&c, "app", &[]).is_some(), "{y}");
        assert_eq!(why_fixed(&c, "app", &[]), None, "{y}");
    }
    // and with no copy to add either, the node is fixed and says why
    let c = cfg(&format!(
        "  app: {{ type: service, proc: 5ms, instance: aws/m5.large, cpu: {{ cores: 3 }}, replicas: {MAX_REPLICAS} }}\n"
    ));
    assert_eq!(moves_for(&c, "app", &[]), []);
    assert!(why_fixed(&c, "app", &[]).is_some());
}

#[test]
fn a_node_in_keep_has_no_moves_and_a_reason() {
    let c = cfg("  app: { type: service, proc: 5ms, instance: aws/m5.large }\n");
    let keep = vec!["app".to_string()];
    assert_eq!(moves_for(&c, "app", &keep), []);
    assert_eq!(moves_for_oom(&c, "app", &keep), []);
    assert_eq!(why_fixed(&c, "app", &keep), Some("kept by you"));
}

#[test]
fn copies_are_refused_on_a_node_with_a_rate_limit() {
    // a copy has its own limiter, so it would raise the limit the developer set
    let c = cfg(
        "  app: { type: service, proc: 5ms, instance: aws/m5.large, rate_limit: { rps: 100, burst: 10 } }\n",
    );
    let k = kinds(&c, "app");
    assert_eq!(k, vec![class("aws/m5.large", "aws/m5.xlarge")]);
    assert!(copies_only_reason(&c, "app", &[]).is_none() && why_fixed(&c, "app", &[]).is_none());
}

#[test]
fn copies_stop_at_the_maximum() {
    let c = cfg(&format!(
        "  app: {{ type: service, proc: 5ms, cpu: {{ cores: 2 }}, replicas: {} }}\n",
        MAX_REPLICAS - 1
    ));
    assert_eq!(kinds(&c, "app")[1], copies(MAX_REPLICAS - 1, MAX_REPLICAS));
    let c = cfg(&format!(
        "  app: {{ type: service, proc: 5ms, cpu: {{ cores: 2 }}, replicas: {MAX_REPLICAS} }}\n"
    ));
    assert_eq!(
        kinds(&c, "app"),
        vec![MoveKind::Cores { from: 2.0, to: 4.0 }]
    );
}

#[test]
fn copies_are_refused_where_weighted_balancing_ties_them() {
    // on the node
    let c = cfg(
        "  app: { type: service, proc: 5ms, cpu: { cores: 2 }, replicas: 2, balance: { weighted: [3, 1] } }\n",
    );
    assert_eq!(
        kinds(&c, "app"),
        vec![MoveKind::Cores { from: 2.0, to: 4.0 }]
    );
    // on an edge into it
    let c = Config::from_yaml(
        "version: 1\nnodes:\n  gw: { type: service, proc: 1ms }\n  app: { type: service, proc: 5ms, replicas: 2 }\nedges:\n  - { from: gw, to: app, balance: { weighted: [1, 1] } }\n",
    )
    .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(moves_for(&c, "app", &[]), []);
    assert!(why_fixed(&c, "app", &[]).is_some());
}

#[test]
fn an_external_api_has_no_machine_moves() {
    let c = cfg(
        "  pay: { type: external-api, proc: 50ms }\n  files: { type: storage, first_byte: 20ms, throughput: 100MB/s }\n",
    );
    for n in ["pay", "files"] {
        assert!(moves_for(&c, n, &[]).is_empty(), "{n}");
        assert!(moves_for_oom(&c, n, &[]).is_empty(), "{n}");
    }
    assert_eq!(why_fixed(&c, "pay", &[]), Some("an external service"));
    assert_eq!(why_fixed(&c, "files", &[]), Some("an object store"));
    assert_eq!(why_fixed(&c, "nope", &[]), Some("not in the design"));
}

#[test]
fn apply_sets_the_field_and_leaves_the_config_alone_on_a_failed_check() {
    let c = cfg(
        "  app: { type: service, proc: 5ms, instance: aws/m5.large }\n  w: { type: service, proc: 5ms, cpu: { cores: 2 }, memory: { limit: 512MiB, per_request: 1MiB } }\n",
    );
    for m in moves_for(&c, "app", &[])
        .into_iter()
        .chain(moves_for(&c, "w", &[]))
        .chain(moves_for_oom(&c, "w", &[]))
    {
        let mut d = c.clone();
        apply(&mut d, &m).unwrap_or_else(|e| panic!("{m:?}: {e}"));
        let n = &d.nodes[&m.node];
        match &m.kind {
            MoveKind::Class { to, .. } => assert_eq!(n.instance_name(), Some(to.as_str())),
            MoveKind::Cores { to, .. } => {
                assert_eq!(
                    n.cpu().map(CpuConfig::millicores),
                    Some(CpuConfig { cores: *to }.millicores())
                );
            }
            MoveKind::Memory { to, .. } => assert_eq!(n.memory().map(|m| m.limit), Some(*to)),
            MoveKind::Copies { to, .. } => assert_eq!(n.replicas(), *to),
        }
        d.validate_candidate().unwrap();
    }
    // one copy more breaks a weighted balance: refused, and the config is as it was
    let c =
        cfg("  app: { type: service, proc: 5ms, replicas: 2, balance: { weighted: [3, 1] } }\n");
    let mut d = c.clone();
    let mv = Move {
        node: "app".into(),
        kind: copies(2, 3),
    };
    let e = apply(&mut d, &mv).unwrap_err();
    assert!(e.contains("app"), "{e}");
    assert_eq!(d.nodes["app"].replicas(), 2);
    // a move whose `from` is not what the node has is stale: refused
    let mut d = c.clone();
    let stale = Move {
        node: "app".into(),
        kind: copies(5, 6),
    };
    assert!(apply(&mut d, &stale).is_err());
    assert_eq!(d.nodes["app"].replicas(), 2);
}

#[test]
fn class_moves_stay_in_the_same_series_and_the_regions_price_file() {
    for (region, node) in [
        ("{ aws: us-east-1 }", "aws/m5.large"),
        ("{ aws: eu-west-1 }", "aws/m5.large"),
        ("{ gcp: us-central1 }", "gcp/n2-standard-2"),
        ("{ azure: us-east }", "azure/standard_d2s_v5"),
    ] {
        let mut c = Config::from_yaml(&format!(
            "version: 1\nregion: {region}\nnodes:\n  app: {{ type: service, proc: 5ms, instance: {node} }}\n"
        ))
        .unwrap_or_else(|e| panic!("{e}"));
        let provider = node.split('/').next().unwrap();
        let r = kaanal_core::cost::regions_of(&c)[provider].clone();
        // walk the whole ladder: every rung is the same series, priced in the region, and larger
        let mut rungs = 0;
        while let Some(m) = moves_for(&c, "app", &[])
            .into_iter()
            .find(|m| matches!(m.kind, MoveKind::Class { .. }))
        {
            let MoveKind::Class { from, to } = &m.kind else {
                unreachable!()
            };
            assert_eq!(series(to), series(node), "{to}");
            assert!(
                kaanal_core::prices::price_micro_usd_hr(&r, to).is_some(),
                "{to} in {r}"
            );
            assert!(lookup(to).unwrap().vcpu >= lookup(from).unwrap().vcpu);
            apply(&mut c, &m).unwrap();
            rungs += 1;
            assert!(rungs < 64, "the ladder ends");
        }
        assert!(rungs > 0, "{node} has a larger class in {r}");
    }
}

/// The `Class` move of `node` for load (or `oom`), as `(from, to)`.
fn class_move(c: &Config, node: &str, oom: bool) -> Option<(String, String)> {
    let ms = if oom {
        moves_for_oom(c, node, &[])
    } else {
        moves_for(c, node, &[])
    };
    ms.into_iter().find_map(|m| match m.kind {
        MoveKind::Class { from, to } => Some((from, to)),
        _ => None,
    })
}

#[test]
fn a_managed_database_moves_up_its_class_priced_from_its_own_list() {
    // the compute price file has no db.* rows: the rungs are priced by the engine's own list
    let c = cfg(
        "  db: { type: database, flavor: postgres, hosting: managed, instance: aws/db.m5.large }\n",
    );
    assert_eq!(
        class_move(&c, "db", false),
        Some(("aws/db.m5.large".into(), "aws/db.m5.xlarge".into()))
    );
    // Azure SQL has its own list too
    let c = Config::from_yaml(
        "version: 1\nregion: { azure: us-east }\nnodes:\n  db: { type: database, flavor: postgres, hosting: managed, instance: azure/db.gp_gen5_4 }\n",
    )
    .unwrap_or_else(|e| panic!("{e}"));
    let (from, to) = class_move(&c, "db", false).expect("an Azure SQL class move");
    assert_eq!(from, "azure/db.gp_gen5_4");
    assert!(lookup(&to).unwrap().vcpu > 4, "{to}");
}

/// A config with one service on `instance`, in the region of its cloud.
fn on(region: &str, instance: &str, extra: &str) -> Config {
    Config::from_yaml(&format!(
        "version: 1\nregion: {region}\nnodes:\n  app: {{ type: service, proc: 5ms, instance: {instance}{extra} }}\n"
    ))
    .unwrap_or_else(|e| panic!("{e}"))
}

#[test]
fn a_class_step_never_lowers_memory_or_swaps_a_family_for_the_same_size() {
    // d2_v2 (2 vCPU, 7 GiB): d11_v2 has 2 vCPU too and is a memory family, so the step is d3_v2
    let c = on("{ azure: us-east }", "azure/standard_d2_v2", "");
    assert_eq!(
        class_move(&c, "app", false).map(|m| m.1),
        Some("azure/standard_d3_v2".into())
    );
    // a5 (2 vCPU, 14 GiB): a3 has more vCPU but half the memory, so the step is a6 (4, 28 GiB)
    let c = on("{ azure: us-east }", "azure/standard_a5", "");
    assert_eq!(
        class_move(&c, "app", false).map(|m| m.1),
        Some("azure/standard_a6".into())
    );
}

#[test]
fn a_memory_step_always_adds_memory_and_a_load_step_always_adds_cpu() {
    let c = on(
        "{ azure: us-east }",
        "azure/standard_a5",
        ", memory: { per_request: 1MiB }",
    );
    let (from, to) = class_move(&c, "app", true).expect("a memory class step");
    let (f, t) = (lookup(&from).unwrap(), lookup(&to).unwrap());
    assert!(
        t.memory_mib > f.memory_mib && t.vcpu >= f.vcpu,
        "{from} {to}"
    );
    // a load step walks the whole ladder: vCPU strictly up, memory never down
    let mut c = on("{ aws: us-east-1 }", "aws/t3.micro", "");
    while let Some((from, to)) = class_move(&c, "app", false) {
        let (f, t) = (lookup(&from).unwrap(), lookup(&to).unwrap());
        assert!(
            t.vcpu > f.vcpu && t.memory_mib >= f.memory_mib,
            "{from} {to}"
        );
        let m = Move {
            node: "app".into(),
            kind: MoveKind::Class { from, to },
        };
        apply(&mut c, &m).unwrap();
    }
}

#[test]
fn a_twin_class_is_not_a_step_and_an_unsized_name_says_so() {
    // m5.metal is m5.24xlarge under another name: from the largest VM size there is no step
    let c = on("{ aws: us-east-1 }", "aws/m5.24xlarge", "");
    assert_eq!(class_move(&c, "app", false), None);
    // gcp/e2-micro has no size in its name: no ladder, and the reason is not "the largest class"
    let c = on("{ gcp: us-central1 }", "gcp/e2-micro", "");
    assert_eq!(class_move(&c, "app", false), None);
    let why = copies_only_reason(&c, "app", &[]).expect("copies only");
    assert!(why.contains("no size"), "{why}");
}

#[test]
fn a_stale_class_cores_or_memory_move_is_refused_and_changes_nothing() {
    let c = cfg(
        "  a: { type: service, proc: 5ms, instance: aws/m5.large }\n  b: { type: service, proc: 5ms, cpu: { cores: 2 }, memory: { limit: 1GiB, per_request: 1MiB } }\n",
    );
    for (node, kind) in [
        ("a", class("aws/m5.xlarge", "aws/m5.2xlarge")),
        ("b", MoveKind::Cores { from: 4.0, to: 8.0 }),
        ("b", MoveKind::Memory { from: 1, to: 2 }),
    ] {
        let mut d = c.clone();
        let mv = Move {
            node: node.into(),
            kind,
        };
        assert!(apply(&mut d, &mv).is_err(), "{mv:?}");
        assert_eq!(
            format!("{:?}", d.nodes[node]),
            format!("{:?}", c.nodes[node]),
            "{mv:?}"
        );
    }
}
