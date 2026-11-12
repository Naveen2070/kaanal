//! The cost estimate as arithmetic (RULES 121, 123, 124, 125, 126): copies x the price an hour of each
//! node, added up. Money is whole micro-USD (1,000,000 = 1 dollar) so a sum is exact. One function
//! for the server's run record and the CLI; Studio's cost card does the same sums on the page
//! (`ui/src/panels/cost/cost.ts`), and `tests/cost.rs` holds the two to the same figures.
//!
//! An estimate for a what-if, never a quote: on-demand list prices of one region per cloud (the
//! file's `region:`, else the cloud's default), instances and object stores. Network egress,
//! licences, reserved and spot prices and per-use services (serverless, managed gateways) are not
//! in it, and a node that falls in one of those says so in `why` instead of showing zero.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::config::{Config, DbEngine, Hosting, NodeConfig, NodeKind, StorageClass, StorageFlavor};
use crate::prices;

/// Hours in a month for turning a GB-month into an hour: a year of 8,760 hours over 12.
pub const HOURS_PER_MONTH: f64 = 730.0;

/// The factor a Multi-AZ database costs over a single-AZ one: AWS prices the standby as a second
/// instance, so exactly 2 on RDS PostgreSQL, MySQL, MariaDB and Oracle (checked against the price
/// catalogue: 5,413 of 5,560 PostgreSQL region and class pairs are 2.00, the rest 1.98 to 2.01).
pub const MULTI_AZ_FACTOR: u64 = 2;

/// Dollars as whole micro-USD. The inputs are checked at load (`cost_per_hour` is 0 to 1,000,000), so
/// the value is finite, not negative and far below `u64::MAX`.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn micro_usd(dollars: f64) -> u64 {
    (dollars * 1e6).round() as u64
}

/// A figure already in micro-USD, rounded. Never negative: its terms are products of unsigned prices
/// and a non-negative size and rate.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn micro_usd_of_micro(micro: f64) -> u64 {
    micro.round() as u64
}

/// What one node costs.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct NodeCost {
    /// The node's name.
    pub name: String,
    /// The instance the node names, `None` for none (a store names none).
    pub instance: Option<String>,
    /// Copies of the node (`replicas`); a store is always one bill.
    pub copies: u32,
    /// One copy an hour, micro-USD; `None` when there is no price (see `why`).
    pub unit_micro_usd_hr: Option<u64>,
    /// All copies an hour, micro-USD.
    pub total_micro_usd_hr: Option<u64>,
    /// Why there is no price, in words.
    pub why: Option<String>,
}

/// The estimate of a whole system.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CostReport {
    /// Each node, in the file's order.
    pub nodes: Vec<NodeCost>,
    /// Everything priced, an hour, micro-USD.
    pub total_micro_usd_hr: u64,
    /// Nodes with a price.
    pub priced: usize,
    /// Nodes left out, each with its `why`.
    pub unpriced: usize,
    /// The region each cloud was priced in.
    pub regions: BTreeMap<String, String>,
}

/// The region each cloud is priced in: the file's `region:`, else the cloud's default.
#[must_use]
pub fn regions_of(cfg: &Config) -> BTreeMap<String, String> {
    prices::DEFAULT_REGIONS
        .iter()
        .map(|(p, r)| {
            let r = cfg.region.get(*p).map_or(*r, String::as_str);
            ((*p).to_string(), r.to_string())
        })
        .collect()
}

/// The estimate of `cfg`. `visits` is each node's visits a second (a run's `visits_per_s`, or the live
/// rate), which prices an object store's requests; a store with no entry is priced for its data alone.
#[must_use]
pub fn estimate(cfg: &Config, visits: &BTreeMap<String, f64>) -> CostReport {
    let regions = regions_of(cfg);
    let nodes: Vec<NodeCost> = cfg
        .nodes
        .iter()
        .map(|(name, n)| node_cost(cfg, &regions, name, n, visits))
        .collect();
    let total = nodes.iter().filter_map(|n| n.total_micro_usd_hr).sum();
    let priced = nodes
        .iter()
        .filter(|n| n.total_micro_usd_hr.is_some())
        .count();
    CostReport {
        unpriced: nodes.len() - priced,
        priced,
        total_micro_usd_hr: total,
        regions,
        nodes,
    }
}

fn node_cost(
    cfg: &Config,
    regions: &BTreeMap<String, String>,
    name: &str,
    n: &NodeConfig,
    visits: &BTreeMap<String, f64>,
) -> NodeCost {
    if let NodeConfig::Storage(s) = n {
        let (micro, why) = storage_cost(s, regions, visits.get(name).copied());
        return NodeCost {
            name: name.to_string(),
            instance: None,
            copies: 1,
            unit_micro_usd_hr: micro,
            total_micro_usd_hr: micro,
            why,
        };
    }
    let copies = u32::from(n.replicas().max(1));
    let instance = n
        .instance_name()
        .map(str::trim)
        .filter(|i| !i.is_empty())
        .map(str::to_string);
    let (unit, why) = match unit_price(cfg, regions, n, instance.as_deref()) {
        Ok(u) => (Some(u), None),
        Err(w) => (None, Some(w)),
    };
    NodeCost {
        name: name.to_string(),
        instance,
        copies,
        unit_micro_usd_hr: unit,
        total_micro_usd_hr: unit.map(|u| u * u64::from(copies)),
        why,
    }
}

/// One copy's price an hour, or the reason there is none.
fn unit_price(
    cfg: &Config,
    regions: &BTreeMap<String, String>,
    n: &NodeConfig,
    instance: Option<&str>,
) -> Result<u64, String> {
    let kind = n.kind();
    let hosting = n.hosting();
    match hosting {
        Hosting::Serverless => return Err("serverless: priced per use, not estimated".into()),
        Hosting::External => return Err("run by someone else: not estimated".into()),
        Hosting::Managed if !matches!(kind, NodeKind::Database | NodeKind::Cache) => {
            return Err("managed: priced by use, not estimated".into());
        }
        _ => {}
    }
    let Some(instance) = instance else {
        return Err(if hosting == Hosting::Managed {
            format!(
                "pick a managed {} class",
                if kind == NodeKind::Cache {
                    "cache"
                } else {
                    "database"
                }
            )
        } else {
            "no instance type".into()
        });
    };
    // your own instance: its cost_per_hour (a name has no slash)
    if let Some(mine) = cfg.instances.get(instance) {
        return mine
            .cost_per_hour
            .map(micro_usd)
            .ok_or_else(|| "your instance has no cost_per_hour".to_string());
    }
    let provider = instance
        .split('/')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let region = regions.get(&provider).map_or("", String::as_str);
    let found = if hosting == Hosting::Managed && kind == NodeKind::Database {
        prices::managed_price_micro_usd_hr(n.engine(), region, instance)
    } else if hosting == Hosting::Managed {
        let flavor = match n {
            NodeConfig::Cache(c) => c.flavor,
            _ => None,
        };
        prices::managed_cache_price_micro_usd_hr(flavor, region, instance)
    } else {
        prices::price_micro_usd_hr(region, instance)
    };
    let unit = found.map(u64::from).ok_or_else(|| {
        format!(
            "no price in {}",
            if region.is_empty() {
                "this region"
            } else {
                region
            }
        )
    })?;
    // a standby in a second zone is a second instance (never Aurora: checked at load)
    let multi_az = n.multi_az_written() == Some(true)
        && !matches!(
            n.engine(),
            DbEngine::AuroraPostgresql | DbEngine::AuroraMysql
        );
    Ok(if multi_az {
        unit * MULTI_AZ_FACTOR
    } else {
        unit
    })
}

/// An object store's price an hour (RULES 123): the data kept over a month plus its requests at
/// `visits` a second, a `write_share` of them at the write price and the rest at the read price.
fn storage_cost(
    s: &crate::config::StorageConfig,
    regions: &BTreeMap<String, String>,
    visits: Option<f64>,
) -> (Option<u64>, Option<String>) {
    let provider = StorageFlavor::provider(s.flavor);
    // the store's own `location:` beats the region of its cloud
    let place = s
        .location
        .as_deref()
        .or_else(|| regions.get(provider).map(String::as_str))
        .unwrap_or_default();
    let class = s.class.unwrap_or_default();
    let Some(p) = prices::storage_class_price(provider, place, class) else {
        let what = if class == StorageClass::Standard {
            "object-store".to_string()
        } else {
            format!("{} object-store", class.word())
        };
        let at = if place.is_empty() {
            "this region"
        } else {
            place
        };
        return (None, Some(format!("no {what} price in {at}")));
    };
    let rate = visits.unwrap_or(0.0);
    if s.stored.is_none() && s.retrieved.is_none() && rate <= 0.0 {
        return (
            None,
            Some("set stored: (the data kept); requests are priced after a run".into()),
        );
    }
    #[allow(clippy::cast_precision_loss)]
    let gb = s.stored.unwrap_or(0) as f64 / (1024.0 * 1024.0 * 1024.0);
    let w = s.write_share.unwrap_or(0.0);
    let per_request =
        ((1.0 - w) * f64::from(p.read_per_1k) + w * f64::from(p.write_per_1k)) / 1000.0;
    // the data read back in a month (a colder class bills each GB), spread over the month's hours
    #[allow(clippy::cast_precision_loss)]
    let retrieved_gb = s.retrieved.unwrap_or(0) as f64 / (1024.0 * 1024.0 * 1024.0);
    let micro = (gb * f64::from(p.gb_month) + retrieved_gb * f64::from(p.retrieval_per_gb))
        / HOURS_PER_MONTH
        + rate * 3600.0 * per_request;
    (Some(micro_usd_of_micro(micro)), None)
}
