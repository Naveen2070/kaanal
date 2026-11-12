use kaanal_core::config::{CacheConfig, CacheFlavor, DbFlavor, DistConfig, OpsConfig};
use kaanal_core::presets::{self, CacheParams};
use kaanal_core::{Config, Dist, Topology};
use std::time::Duration;

fn fixed(ms: u64) -> DistConfig {
    DistConfig::Fixed(Duration::from_millis(ms))
}

#[test]
fn flavor_fills_missing_ops_and_user_values_win() {
    let ops = OpsConfig {
        write: Some(fixed(9)),
        ..OpsConfig::default()
    };
    let [read, write, scan] = presets::resolve_ops(Some(DbFlavor::Postgres), Some(&ops)).unwrap();
    let preset = presets::db_ops(DbFlavor::Postgres);
    assert_eq!(read, preset[0]);
    assert_eq!(write, fixed(9), "the user's value wins");
    assert_eq!(scan, preset[2]);
    // Through the topology: a database with only a flavor gets the whole preset.
    let cfg = Config::from_yaml(
        "version: 1\nnodes:\n  gw: { type: gateway }\n  db: { type: database, flavor: mongodb }\n",
    )
    .unwrap();
    let t = Topology::build(&cfg).unwrap();
    let want = presets::db_ops(DbFlavor::Mongodb).map(|d| Dist::from_config(Some(&d)));
    assert_eq!(t.nodes[t.id("db").unwrap()].ops.as_ref(), Some(&want));
}

#[test]
fn without_flavor_all_three_ops_are_needed() {
    let partial = OpsConfig {
        read: Some(fixed(1)),
        ..OpsConfig::default()
    };
    assert!(presets::resolve_ops(None, Some(&partial)).is_none());
    assert!(presets::resolve_ops(None, None).is_none());
    let full = OpsConfig {
        read: Some(fixed(1)),
        write: Some(fixed(2)),
        scan: Some(fixed(3)),
    };
    assert_eq!(
        presets::resolve_ops(None, Some(&full)),
        Some([fixed(1), fixed(2), fixed(3)])
    );
}

#[test]
fn unknown_or_wrong_family_flavor_lists_valid_names() {
    let db = &[
        "postgres",
        "mysql",
        "mariadb",
        "mongodb",
        "neo4j",
        "cassandra",
        "dynamodb",
    ][..];
    let cache = &["redis", "memcached"][..];
    for (node, names) in [
        ("{ type: database, flavor: redis }", db),
        ("{ type: database, flavor: oracle }", db),
        ("{ type: monolith, flavor: memcached }", db),
        ("{ type: cache, flavor: postgres }", cache),
    ] {
        let err = Config::from_yaml(&format!("version: 1\nnodes:\n  n: {node}\n")).unwrap_err();
        for name in names {
            assert!(err.msg.contains(name), "{node}: {err}");
        }
        assert_eq!(err.line, Some(3), "{node}: {err}");
    }
}

#[test]
fn cache_flavor_fills_the_gaps() {
    let c = CacheConfig {
        flavor: Some(CacheFlavor::Memcached),
        hit_ratio: Some(0.5),
        ..CacheConfig::default()
    };
    let d = presets::cache_defaults(CacheFlavor::Memcached);
    assert_eq!(
        presets::resolve_cache(&c),
        Some(CacheParams {
            hit_ratio: 0.5,
            hit: d.hit.clone(),
            miss: d.miss.clone()
        })
    );
    let no_flavor = CacheConfig {
        hit: Some(fixed(1)),
        miss: Some(fixed(2)),
        ..CacheConfig::default()
    };
    assert!(
        presets::resolve_cache(&no_flavor).is_none(),
        "hit_ratio is missing"
    );
}

/// B19: Valkey is a Redis fork with the same protocol and no published figure that separates the two,
/// so it takes the Redis preset (an estimate, like every preset): by decision, not by accident.
#[test]
fn valkey_takes_the_redis_preset() {
    assert_eq!(
        presets::cache_defaults(CacheFlavor::Valkey),
        presets::cache_defaults(CacheFlavor::Redis)
    );
    assert_ne!(
        presets::cache_defaults(CacheFlavor::Valkey),
        presets::cache_defaults(CacheFlavor::Memcached)
    );
}

#[test]
fn presets_are_sane() {
    let mut all: Vec<DistConfig> = [DbFlavor::Postgres, DbFlavor::Mysql, DbFlavor::Mongodb]
        .into_iter()
        .flat_map(presets::db_ops)
        .collect();
    for f in [
        CacheFlavor::Redis,
        CacheFlavor::Memcached,
        CacheFlavor::Valkey,
    ] {
        let p = presets::cache_defaults(f);
        assert!((0.0..=1.0).contains(&p.hit_ratio), "{p:?}");
        all.extend([p.hit, p.miss]);
    }
    for d in all {
        let DistConfig::Pct { p50, p99 } = d else {
            panic!("presets are percentile pairs: {d:?}")
        };
        assert!(
            p50 > Duration::ZERO && p50 <= p99 && p99 <= Duration::from_secs(1),
            "{d:?}"
        );
    }
}
