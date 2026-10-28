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
        DbFlavor::Mysql => [pct(300, 2_500), pct(1_200, 10_000), pct(3_000, 30_000)],
        DbFlavor::Mongodb => [pct(400, 3_000), pct(800, 6_000), pct(4_000, 35_000)],
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
/// external-api pool 10, auth 32; a cache has none) and what that kind needs to be valid
/// (a database and a cache take a flavor, whose preset fills the costs). **Estimates**, like
/// every preset here: typical sizes for a small deployment, to be edited.
pub fn starter(kind: NodeKind) -> &'static str {
    match kind {
        NodeKind::Gateway => "{ type: gateway, concurrency: 256 }",
        NodeKind::Service => "{ type: service, concurrency: 32 }",
        NodeKind::Monolith => "{ type: monolith, concurrency: 64 }",
        NodeKind::Database => "{ type: database, flavor: postgres, pool: { size: 20 } }",
        NodeKind::Cache => "{ type: cache, flavor: redis }",
        NodeKind::ExternalApi => "{ type: external-api, pool: { size: 10 } }",
        NodeKind::Auth => "{ type: auth, concurrency: 32 }",
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
