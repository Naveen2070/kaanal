//! Flavor presets (rule R1.25): default op costs for database flavors, and default hit
//! ratio and hit/miss costs for cache flavors.
//!
//! **All numbers are estimates**: typical single-node figures for a warm instance with
//! small rows, excluding the network (edges carry `net`). Precedence is user value >
//! preset > error; [`crate::topology`] raises the error when something is still missing.

use std::time::Duration;

use crate::config::{CacheConfig, CacheFlavor, DbFlavor, DistConfig, NodeKind, OpsConfig};

/// **Private.** A lognormal `{ p50, p99 }` given in microseconds.
const fn pct(p50_us: u64, p99_us: u64) -> DistConfig {
    DistConfig::Pct {
        p50: Duration::from_micros(p50_us),
        p99: Duration::from_micros(p99_us),
    }
}

/// Preset read, write and scan costs of a database flavor (estimates): a primary-key
/// read, a single-row commit, an indexed list of about 100 rows.
pub fn db_ops(flavor: DbFlavor) -> [DistConfig; 3] {
    match flavor {
        DbFlavor::Postgres => [pct(300, 2_000), pct(1_000, 8_000), pct(3_000, 25_000)],
        // MariaDB is InnoDB underneath, like MySQL: the same figures
        DbFlavor::Mysql | DbFlavor::Mariadb => {
            [pct(300, 2_500), pct(1_200, 10_000), pct(3_000, 30_000)]
        }
        DbFlavor::Mongodb => [pct(400, 3_000), pct(800, 6_000), pct(4_000, 35_000)],
        // estimates, no published figure: an indexed node lookup, a transaction commit (fsync), a
        // traversal of about 100 nodes
        DbFlavor::Neo4j => [pct(500, 4_000), pct(2_000, 15_000), pct(6_000, 50_000)],
        // estimates, no published figure: an LSM write is the cheap one, a read may touch several
        // SSTables, a scan is a partition slice
        DbFlavor::Cassandra => [pct(1_000, 8_000), pct(500, 4_000), pct(8_000, 60_000)],
        // AWS states "single-digit millisecond" latency for reads and writes; the p50 and p99 here
        // are inside that statement (a GetItem, a PutItem, a Query of about 100 items)
        DbFlavor::Dynamodb => [pct(3_000, 9_000), pct(5_000, 15_000), pct(10_000, 40_000)],
    }
}

/// A cache's hit ratio and hit/miss costs.
#[derive(Debug, Clone, PartialEq)]
pub struct CacheParams {
    /// Share of lookups that hit, 0..=1.
    pub hit_ratio: f64,
    /// Cost of a hit.
    pub hit: DistConfig,
    /// Cost of finding out it is a miss (the fallback call is the chain's job).
    pub miss: DistConfig,
}

/// Preset hit ratio and costs of a cache flavor (estimates).
pub fn cache_defaults(flavor: CacheFlavor) -> CacheParams {
    match flavor {
        CacheFlavor::Redis => CacheParams {
            hit_ratio: 0.9,
            hit: pct(200, 1_000),
            miss: pct(200, 1_000),
        },
        CacheFlavor::Memcached => CacheParams {
            hit_ratio: 0.9,
            hit: pct(150, 800),
            miss: pct(150, 800),
        },
    }
}

/// The node a user adds in Studio, as flow-style YAML: its `type`, a default capacity so the
/// bottleneck report can rank it (gateway 256, service 32, monolith 64, database pool 20,
/// external-api pool 10, auth 32; a cache has none) and a service time `proc` (gateway 2 ms,
/// service 25, monolith 40, external-api 100, auth 10; a database and a cache take theirs from
/// the flavor), because a node with slots and no time has infinite capacity and can never
/// saturate, so offered load would break nothing. And what that kind needs to be valid
/// (a database and a cache take a flavor, whose preset fills the costs). **Estimates**, like
/// every preset here: typical sizes for a small deployment, to be edited.
///
/// A storage node (an object store) starts as `flavor: s3`, `first_byte: 25ms`
/// and `throughput: 80MB/s`. **Placeholders to edit, not published figures**: no provider
/// publishes a first-byte latency for standard object storage (RULES 61), so these are estimates.
/// It has no slot limit: its request-rate figures belong to `rate_limit`, which is off unless set.
///
/// Every node with a slot limit is written with a bounded queue, `concurrency: { limit: N,
/// queue: 100 }` (F3, RULES 56), and the gateway, the entry node, with a client `timeout: 30s`
/// (the low end of the 30 to 60 s where real clients and proxies give up). Both are written
/// into the file only: the engine default stays "unset means unlimited / never".
pub fn starter(kind: NodeKind) -> &'static str {
    match kind {
        NodeKind::Gateway => {
            "{ type: gateway, concurrency: { limit: 256, queue: 100 }, proc: 2ms, timeout: 30s }"
        }
        NodeKind::Service => {
            "{ type: service, concurrency: { limit: 32, queue: 100 }, proc: 25ms }"
        }
        NodeKind::Monolith => {
            "{ type: monolith, concurrency: { limit: 64, queue: 100 }, proc: 40ms }"
        }
        NodeKind::Database => "{ type: database, flavor: postgres, pool: { size: 20 } }",
        NodeKind::Cache => "{ type: cache, flavor: redis }",
        NodeKind::ExternalApi => "{ type: external-api, pool: { size: 10 }, proc: 100ms }",
        NodeKind::Auth => "{ type: auth, concurrency: { limit: 32, queue: 100 }, proc: 10ms }",
        NodeKind::LoadBalancer => {
            "{ type: load-balancer, concurrency: { limit: 1000, queue: 100 }, proc: 200us }"
        }
        NodeKind::Storage => "{ type: storage, flavor: s3, first_byte: 25ms, throughput: 80MB/s }",
    }
}

/// Read, write and scan costs: the user's ops, gaps filled from `flavor`. `None` while
/// any op is still missing.
pub fn resolve_ops(flavor: Option<DbFlavor>, ops: Option<&OpsConfig>) -> Option<[DistConfig; 3]> {
    let preset = flavor.map(db_ops);
    let user = ops.cloned().unwrap_or_default();
    let pick =
        |mine: Option<DistConfig>, i: usize| mine.or_else(|| preset.as_ref().map(|p| p[i].clone()));
    Some([
        pick(user.read, 0)?,
        pick(user.write, 1)?,
        pick(user.scan, 2)?,
    ])
}

/// A cache's parameters: the user's values, gaps filled from its `flavor`. `None` while
/// any is still missing.
pub fn resolve_cache(c: &CacheConfig) -> Option<CacheParams> {
    let preset = c.flavor.map(cache_defaults);
    Some(CacheParams {
        hit_ratio: c
            .hit_ratio
            .or_else(|| preset.as_ref().map(|p| p.hit_ratio))?,
        hit: c
            .hit
            .clone()
            .or_else(|| preset.as_ref().map(|p| p.hit.clone()))?,
        miss: c
            .miss
            .clone()
            .or_else(|| preset.as_ref().map(|p| p.miss.clone()))?,
    })
}

#[cfg(test)]
mod starter_tests {
    use super::*;
    use crate::config::{ConcurrencyConfig, Config};

    #[test]
    fn slot_limited_starters_carry_a_queue_of_100_and_the_gateway_a_30s_timeout() {
        for (kind, slots) in [
            (NodeKind::Gateway, 256),
            (NodeKind::Service, 32),
            (NodeKind::Monolith, 64),
            (NodeKind::Auth, 32),
            (NodeKind::LoadBalancer, 1000),
        ] {
            let v: serde_yaml_ng::Value = serde_yaml_ng::from_str(starter(kind)).unwrap();
            assert_eq!(v["concurrency"]["limit"], slots, "{kind:?}");
            assert_eq!(v["concurrency"]["queue"], 100, "{kind:?}");
        }
        let gw: serde_yaml_ng::Value = serde_yaml_ng::from_str(starter(NodeKind::Gateway)).unwrap();
        assert_eq!(gw["timeout"], "30s");
    }

    #[test]
    fn a_gateway_and_service_from_studio_pass_the_config_checks() {
        let text = format!(
            "version: 1\nnodes:\n  gw: {}\n  app: {}\nedges:\n  - {{ from: gw, to: app }}\n",
            starter(NodeKind::Gateway),
            starter(NodeKind::Service)
        );
        let cfg = Config::from_yaml(&text).unwrap();
        let gw = &cfg.nodes["gw"];
        assert_eq!(gw.timeout(), Some(Duration::from_secs(30)));
        assert!(cfg.nodes["app"].timeout().is_none());
        let ConcurrencyConfig::Bounded(b) = cfg.nodes["app"].concurrency().unwrap() else {
            panic!("service is not bounded")
        };
        assert_eq!((b.limit.get(), b.queue), (32, 100));
    }
}
