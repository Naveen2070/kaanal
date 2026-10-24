//! Concurrency, rate limits and capacity checks.

use crate::common::*;
use kaanal_core::Config;
use kaanal_core::config::ConcurrencyConfig;
use std::num::NonZeroU32;

#[test]
fn concurrency_takes_a_number_or_a_map() {
    let yaml = "version: 1\nnodes:\n  a: { type: service, concurrency: 4 }\n  b: { type: service, concurrency: { limit: 2, queue: 5, overflow_status: 504 } }\n  c: { type: service, concurrency: { limit: 3 } }\n";
    let cfg = Config::from_yaml(yaml).unwrap();
    assert_eq!(
        cfg.nodes["a"].concurrency(),
        Some(&ConcurrencyConfig::Slots(NonZeroU32::new(4).unwrap()))
    );
    let Some(ConcurrencyConfig::Bounded(b)) = cfg.nodes["b"].concurrency() else {
        panic!("b")
    };
    assert_eq!((b.limit.get(), b.queue, b.overflow_status), (2, 5, 504));
    let Some(ConcurrencyConfig::Bounded(c)) = cfg.nodes["c"].concurrency() else {
        panic!("c")
    };
    assert_eq!(
        (c.queue, c.overflow_status),
        (0, 503),
        "map defaults match pool"
    );
    assert_eq!(cfg.nodes["b"].concurrency().unwrap().slots().get(), 2);
}

#[test]
fn bad_concurrency_values_are_placed_and_named() {
    for (value, want) in [
        ("0", "at least 1"),
        ("-1", "at least 1"),
        ("1.5", "positive number or a map"),
        ("lots", "positive number or a map"),
        ("{ limit: 0 }", "nonzero"),
        ("{ queue: 1 }", "limit"),
        ("{ limit: 1, overflow_status: 418 }", "between 500 and 504"),
        ("{ limit: 1, bogus: 1 }", "unknown field"),
    ] {
        let src = format!("version: 1\nnodes:\n  gw: {{ type: gateway, concurrency: {value} }}\n");
        let e = Config::from_yaml(&src).unwrap_err();
        assert_eq!(e.line, Some(3), "{value}: {e}");
        assert!(e.msg.contains(want), "{value}: {e}");
    }
}

#[test]
fn rate_limit_parses_on_every_node_kind() {
    let rl = "rate_limit: { rps: 10, burst: 3 }";
    for node in [
        format!("{{ type: gateway, {rl} }}"),
        format!("{{ type: service, {rl} }}"),
        format!("{{ type: monolith, {rl} }}"),
        format!("{{ type: database, flavor: postgres, {rl} }}"),
        format!("{{ type: cache, flavor: redis, {rl} }}"),
        format!("{{ type: external-api, {rl} }}"),
        format!("{{ type: auth, {rl} }}"),
    ] {
        let cfg = Config::from_yaml(&format!("version: 1\nnodes:\n  n: {node}\n")).unwrap();
        assert_eq!(
            cfg.nodes["n"].rate_limit().unwrap().burst.get(),
            3,
            "{node}"
        );
    }
}

#[test]
fn capacity_limits_are_checked_at_load() {
    let both = "version: 1\nnodes:\n  db:\n    type: database\n    flavor: postgres\n    pool: { size: 2 }\n    concurrency: 4\n";
    let e = Config::from_yaml(both).unwrap_err();
    assert!(
        e.msg
            .contains("node 'db' sets both pool and concurrency; use one"),
        "{e}"
    );
    assert!(at(both, &e).starts_with("db:"), "{e}");

    let fast =
        "version: 1\nnodes:\n  gw: { type: gateway, rate_limit: { rps: 2000000000, burst: 1 } }\n";
    let e = Config::from_yaml(fast).unwrap_err();
    assert!(e.msg.contains("at most 1000000000"), "{e}");
    assert_eq!(e.line, Some(3));

    let slow = "version: 1\nnodes:\n  gw:\n    type: gateway\n    rate_limit: { rps: 0.000001, burst: 4294967295 }\n";
    let e = Config::from_yaml(slow).unwrap_err();
    assert!(e.msg.contains("burst / rps"), "{e}");
    assert!(at(slow, &e).starts_with('{'), "points at the value: {e}");

    // The extremes that are allowed still load.
    Config::from_yaml("version: 1\nnodes:\n  gw: { type: gateway, rate_limit: { rps: 1000000000, burst: 4294967295 }, concurrency: { limit: 4294967295, queue: 4294967295 } }\n").unwrap();
}
