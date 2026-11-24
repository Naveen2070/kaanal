//! Cheaper-move candidates (Step 9c-2, plan Task 2): the oracle is a scan of `instances::all()` and the
//! region's price file, never a figure the code printed.

use kaanal_core::Config;
use kaanal_core::instances::{self, Instance};
use kaanal_core::prices;
use kaanal_core::resize::{Move, MoveKind, apply, cheaper_moves, moves_for, why_fixed};

fn cfg(nodes: &str) -> Config {
    Config::from_yaml(&format!("version: 1\nnodes:\n{nodes}")).unwrap_or_else(|e| panic!("{e}"))
}

fn region() -> &'static str {
    prices::DEFAULT_REGIONS
        .iter()
        .find(|(p, _)| *p == "aws")
        .map(|(_, r)| *r)
        .unwrap()
}

fn price(i: &Instance) -> Option<u32> {
    prices::price_micro_usd_hr(region(), i.name)
}

/// The priced AWS compute classes cheaper than `cur`.
fn cheaper_than(cur: &str) -> Vec<(&'static Instance, u32)> {
    let p = price(instances::lookup(cur).unwrap()).unwrap();
    instances::all()
        .iter()
        .filter(|i| i.name.starts_with("aws/") && !instances::is_managed_class(i.name))
        .filter_map(|i| price(i).map(|q| (i, q)))
        .filter(|(_, q)| *q < p)
        .collect()
}

fn to_class(m: &Move) -> &'static Instance {
    match &m.kind {
        MoveKind::Class { to, .. } => instances::lookup(to).unwrap(),
        k => panic!("not a class move: {k:?}"),
    }
}

const BIG: &str = "  app: { type: service, proc: 5ms, instance: aws/m5.4xlarge }\n";

#[test]
fn the_class_candidates_are_the_pareto_front_and_cheaper_than_now() {
    let c = cfg(BIG);
    let cur = price(instances::lookup("aws/m5.4xlarge").unwrap()).unwrap();
    let got: Vec<&Instance> = cheaper_moves(&c, "app", &[])
        .iter()
        .filter(|m| matches!(m.kind, MoveKind::Class { .. }))
        .map(to_class)
        .collect();
    assert!(got.len() > 1);
    for g in &got {
        assert!(price(g).unwrap() < cur, "{}", g.name);
    }
    let pool = cheaper_than("aws/m5.4xlarge");
    let beats = |a: &Instance, qa: u32, b: &Instance, qb: u32| {
        a.vcpu >= b.vcpu
            && a.memory_mib >= b.memory_mib
            && qa <= qb
            && (a.vcpu, a.memory_mib, qa) != (b.vcpu, b.memory_mib, qb)
    };
    for g in &got {
        let qg = price(g).unwrap();
        assert!(
            !pool.iter().any(|(i, q)| beats(i, *q, g, qg)),
            "{} is dominated",
            g.name
        );
    }
    // maximal: everything left out is dominated by, or a twin of, a candidate
    for (i, q) in &pool {
        let dominated = got.iter().any(|g| {
            let qg = price(g).unwrap();
            beats(g, qg, i, *q) || (g.vcpu, g.memory_mib, qg) == (i.vcpu, i.memory_mib, *q)
        });
        assert!(dominated, "{} was dropped but nothing covers it", i.name);
    }
    // any series: the front is not one family
    let series: std::collections::BTreeSet<_> = got
        .iter()
        .filter_map(|i| instances::series(i.name))
        .collect();
    assert!(series.len() > 1, "{series:?}");
}

#[test]
fn copies_below_the_current_count_are_candidates_in_price_order() {
    let c = cfg("  app: { type: service, proc: 5ms, instance: aws/m5.large, replicas: 4 }\n");
    let ms = cheaper_moves(&c, "app", &[]);
    let copies: Vec<(u16, u16)> = ms
        .iter()
        .filter_map(|m| match m.kind {
            MoveKind::Copies { from, to } => Some((from, to)),
            _ => None,
        })
        .collect();
    assert_eq!(copies, vec![(4, 1), (4, 2), (4, 3)]);
    // ascending by the price of the resulting node (class price times copies)
    let total = |m: &Move| -> u64 {
        let (unit, n) = match &m.kind {
            MoveKind::Class { to, .. } => (price(instances::lookup(to).unwrap()).unwrap(), 4),
            MoveKind::Copies { to, .. } => (
                price(instances::lookup("aws/m5.large").unwrap()).unwrap(),
                *to,
            ),
            k => panic!("{k:?}"),
        };
        u64::from(unit) * u64::from(n)
    };
    assert!(ms.windows(2).all(|w| total(&w[0]) <= total(&w[1])));
    // a node with no class has no price: nothing is offered (RULES 142 e)
    let c = cfg("  app: { type: service, proc: 5ms, cpu: { cores: 6 }, replicas: 2 }\n");
    assert_eq!(cheaper_moves(&c, "app", &[]).len(), 0);
}

#[test]
fn a_kept_node_a_custom_instance_and_an_unpriced_node_have_none() {
    let c = cfg(BIG);
    assert_eq!(cheaper_moves(&c, "app", &["app".to_string()]).len(), 0);
    assert_eq!(cheaper_moves(&c, "nope", &[]).len(), 0);
    let c = Config::from_yaml(
        "version: 1\ninstances:\n  big: { cores: 8, cost_per_hour: 1.5 }\nnodes:\n  app: { type: service, proc: 5ms, instance: big, replicas: 3 }\n",
    )
    .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(cheaper_moves(&c, "app", &[]).len(), 0);
    // a class the region's file does not price: it must exist, or this case checks nothing
    let i = instances::all()
        .iter()
        .find(|i| {
            i.name.starts_with("aws/") && !instances::is_managed_class(i.name) && price(i).is_none()
        })
        .expect("the table has an aws compute class the default region does not price");
    let c = cfg(&format!(
        "  app: {{ type: service, proc: 5ms, instance: {}, replicas: 3 }}\n",
        i.name
    ));
    assert_eq!(cheaper_moves(&c, "app", &[]).len(), 0, "{}", i.name);
    // an external API and an object store have no machine
    let c = cfg(
        "  pay: { type: external-api, proc: 50ms, replicas: 3 }\n  files: { type: storage, first_byte: 20ms, throughput: 100MB/s }\n",
    );
    for n in ["pay", "files"] {
        assert_eq!(cheaper_moves(&c, n, &[]).len(), 0, "{n}");
    }
}

#[test]
fn a_hand_written_figure_keeps_the_class_but_copies_still_shrink() {
    let c = cfg(
        "  app: { type: service, proc: 5ms, instance: aws/m5.4xlarge, replicas: 2, cpu: { cores: 2 } }\n",
    );
    let ms = cheaper_moves(&c, "app", &[]);
    assert_eq!(ms.len(), 1);
    assert_eq!(ms[0].kind, MoveKind::Copies { from: 2, to: 1 });
}

#[test]
fn every_candidate_applies_and_passes_validate_candidate() {
    for nodes in [
        "  app: { type: service, proc: 5ms, instance: aws/m5.4xlarge, replicas: 3 }\n",
        "  db: { type: database, flavor: postgres, hosting: managed, instance: aws/db.m5.4xlarge, replicas: 2 }\n",
    ] {
        let c = cfg(nodes);
        let node = c.nodes.keys().next().unwrap().clone();
        let ms = cheaper_moves(&c, &node, &[]);
        assert!(!ms.is_empty(), "{nodes}");
        for m in ms {
            let mut x = c.clone();
            apply(&mut x, &m).unwrap_or_else(|e| panic!("{m:?}: {e}"));
        }
    }
}

#[test]
fn the_9c_1_moves_are_unchanged() {
    let c = cfg(
        "  app: { type: service, proc: 5ms, instance: aws/m5.large, replicas: 2 }\n  w: { type: service, proc: 5ms, cpu: { cores: 3 } }\n",
    );
    let class = |to: &str| MoveKind::Class {
        from: "aws/m5.large".into(),
        to: to.into(),
    };
    let k = |n| {
        moves_for(&c, n, &[])
            .into_iter()
            .map(|m| m.kind)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        k("app"),
        vec![class("aws/m5.xlarge"), MoveKind::Copies { from: 2, to: 3 }]
    );
    assert_eq!(
        k("w"),
        vec![
            MoveKind::Cores { from: 3.0, to: 4.0 },
            MoveKind::Copies { from: 1, to: 2 }
        ]
    );
}

fn copy_shrinks(c: &Config, node: &str) -> usize {
    cheaper_moves(c, node, &[])
        .iter()
        .filter(|m| matches!(m.kind, MoveKind::Copies { .. }))
        .count()
}

#[test]
fn a_rate_limit_or_a_weighted_balance_blocks_copy_shrinks() {
    let plain = cfg("  app: { type: service, proc: 5ms, instance: aws/m5.large, replicas: 3 }\n");
    assert_eq!(copy_shrinks(&plain, "app"), 2);
    let limited = cfg(
        "  app: { type: service, proc: 5ms, instance: aws/m5.large, replicas: 3, rate_limit: { rps: 100, burst: 10 } }\n",
    );
    assert_eq!(copy_shrinks(&limited, "app"), 0);
    assert!(
        !cheaper_moves(&limited, "app", &[]).is_empty(),
        "the class still shrinks"
    );
    let on_node = cfg(
        "  app: { type: service, proc: 5ms, instance: aws/m5.large, replicas: 2, balance: { weighted: [3, 1] } }\n",
    );
    assert_eq!(copy_shrinks(&on_node, "app"), 0);
    let on_edge = Config::from_yaml(
        "version: 1\nnodes:\n  gw: { type: service, proc: 1ms }\n  app: { type: service, proc: 5ms, instance: aws/m5.large, replicas: 3 }\nedges:\n  - { from: gw, to: app, balance: { weighted: [1, 1, 1] } }\n",
    )
    .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(copy_shrinks(&on_edge, "app"), 0);
}

#[test]
fn a_hand_written_memory_limit_skips_class_candidates() {
    let c = cfg(
        "  app: { type: service, proc: 5ms, instance: aws/m5.4xlarge, replicas: 2, memory: { limit: 1GiB, per_request: 1MiB } }\n",
    );
    let ms = cheaper_moves(&c, "app", &[]);
    assert_eq!(ms.len(), 1);
    assert_eq!(ms[0].kind, MoveKind::Copies { from: 2, to: 1 });
}

#[test]
fn a_managed_cache_gets_cache_classes_that_apply() {
    let c = cfg(
        "  c: { type: cache, flavor: redis, hosting: managed, instance: aws/cache.m5.4xlarge, replicas: 2 }\n",
    );
    let ms = cheaper_moves(&c, "c", &[]);
    let classes: Vec<&Move> = ms
        .iter()
        .filter(|m| matches!(m.kind, MoveKind::Class { .. }))
        .collect();
    assert!(classes.len() > 1);
    for m in &classes {
        assert!(instances::is_cache_class(to_class(m).name), "{m:?}");
    }
    for m in &ms {
        let mut x = c.clone();
        apply(&mut x, m).unwrap_or_else(|e| panic!("{m:?}: {e}"));
    }
}

#[test]
fn twins_with_one_size_and_price_keep_the_first_name() {
    let pool: Vec<(&'static Instance, u32)> = instances::all()
        .iter()
        .filter(|i| i.name.starts_with("aws/") && !instances::is_managed_class(i.name))
        .filter_map(|i| price(i).map(|q| (i, q)))
        .collect();
    let (top, _) = pool.iter().max_by_key(|(i, q)| (*q, i.name)).unwrap();
    let twins: Vec<&str> = pool
        .iter()
        .filter(|(a, qa)| {
            pool.iter().any(|(b, qb)| {
                a.name != b.name && (a.vcpu, a.memory_mib, *qa) == (b.vcpu, b.memory_mib, *qb)
            })
        })
        .filter(|(i, q)| *q < price(top).unwrap() && i.vcpu > 0)
        .map(|(i, _)| i.name)
        .collect();
    assert!(
        !twins.is_empty(),
        "the table has no same-size same-price pair"
    );
    let c = cfg(&format!(
        "  app: {{ type: service, proc: 5ms, instance: {} }}\n",
        top.name
    ));
    let got: Vec<&str> = cheaper_moves(&c, "app", &[])
        .iter()
        .map(|m| to_class(m).name)
        .collect();
    for t in &twins {
        let (ti, tq) = pool.iter().find(|(i, _)| i.name == *t).unwrap();
        let same: Vec<&str> = pool
            .iter()
            .filter(|(i, q)| (i.vcpu, i.memory_mib, *q) == (ti.vcpu, ti.memory_mib, *tq))
            .map(|(i, _)| i.name)
            .collect();
        let first = same.iter().min().unwrap();
        assert!(!got.contains(t) || t == first, "{t} listed beside {first}");
        assert!(
            got.iter().filter(|g| same.contains(g)).count() <= 1,
            "{same:?}"
        );
    }
    assert_eq!(
        got,
        cheaper_moves(&c, "app", &[])
            .iter()
            .map(|m| to_class(m).name)
            .collect::<Vec<_>>()
    );
}

#[test]
fn the_9c_1_copy_reasons_are_unchanged() {
    // size knob blocked (own instance), so why_fixed gives the copies reason
    let rack = "version: 1\ninstances:\n  rack: { cores: 8, memory: 32GiB }\nnodes:\n";
    let limited = Config::from_yaml(&format!(
        "{rack}  app: {{ type: service, proc: 5ms, instance: rack, rate_limit: {{ rps: 100, burst: 10 }} }}\n"
    ))
    .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(moves_for(&limited, "app", &[]), []);
    assert_eq!(
        why_fixed(&limited, "app", &[]),
        Some("it has a rate limit you set, and each copy would add its own")
    );
    let weighted = Config::from_yaml(&format!(
        "{rack}  app: {{ type: service, proc: 5ms, instance: rack, replicas: 2, balance: {{ weighted: [3, 1] }} }}\n"
    ))
    .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(moves_for(&weighted, "app", &[]), []);
    assert_eq!(
        why_fixed(&weighted, "app", &[]),
        Some("a weighted balance names each copy: one more needs a weight from you")
    );
}
