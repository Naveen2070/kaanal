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

use crate::chain::Endpoints;
use crate::config::{Config, DbEngine, Hosting, NodeConfig, NodeKind, StorageClass, StorageFlavor};
use crate::prices;
use crate::topology::Topology;

/// Hours in a month for turning a GB-month into an hour: a year of 8,760 hours over 12.
pub const HOURS_PER_MONTH: f64 = 730.0;

/// The factor a Multi-AZ database costs over a single-AZ one: AWS prices the standby as a second
/// instance, so exactly 2 on RDS PostgreSQL, MySQL, MariaDB and Oracle (checked against the price
/// catalogue: 5,413 of 5,560 PostgreSQL region and class pairs are 2.00, the rest 1.98 to 2.01). For the
/// licence-included Oracle and SQL Server editions it is the common case, not the rule: 3,640 of 6,181
/// region and class pairs are 2.00 and the rest run from 1.23 to 2.59 (RULES 131).
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
    /// What a reader should know about a price that is there (a size assumed, a cloud assumed).
    pub note: Option<String>,
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
    /// Data sent out to the internet, one line per cloud the entry nodes are in (empty for a system
    /// priced without a run). Already in `total_micro_usd_hr`.
    pub egress: Vec<EgressCost>,
}

/// What a run sends out to the internet through the entry nodes of one cloud, an hour (RULES 131).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EgressCost {
    /// The cloud whose price was used.
    pub provider: String,
    /// Its region.
    pub region: String,
    /// Response bytes a second the run's streams send (rate x `resp_bytes`), rounded.
    pub bytes_per_s: u64,
    /// The fee an hour, micro-USD; `None` when the cloud has no price for the region.
    pub micro_usd_hr: Option<u64>,
    /// What a reader should know: priced at AWS's rate because the entry node names no cloud, or why
    /// there is no price.
    pub note: Option<String>,
}

/// Response bytes a second that leave through the entry nodes of one cloud (`provider` is empty when
/// the entry node names no cloud: self-hosted, or your own instance).
#[derive(Debug, Clone, PartialEq)]
pub struct EgressFlow {
    /// `aws`, `azure`, `gcp`, or empty.
    pub provider: String,
    /// Bytes a second.
    pub bytes_per_s: f64,
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
    estimate_with_egress(cfg, visits, &[])
}

/// [`estimate`] plus what a run sends out to the internet (`flows`, from [`egress_flows`]).
#[must_use]
pub fn estimate_with_egress(
    cfg: &Config,
    visits: &BTreeMap<String, f64>,
    flows: &[EgressFlow],
) -> CostReport {
    let regions = regions_of(cfg);
    let nodes: Vec<NodeCost> = cfg
        .nodes
        .iter()
        .map(|(name, n)| node_cost(cfg, &regions, name, n, visits))
        .collect();
    let egress: Vec<EgressCost> = flows.iter().map(|f| egress_cost(&regions, f)).collect();
    let total = nodes
        .iter()
        .filter_map(|n| n.total_micro_usd_hr)
        .chain(egress.iter().filter_map(|e| e.micro_usd_hr))
        .sum();
    let priced = nodes
        .iter()
        .filter(|n| n.total_micro_usd_hr.is_some())
        .count();
    CostReport {
        unpriced: nodes.len() - priced,
        priced,
        total_micro_usd_hr: total,
        regions,
        egress,
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
            note: None,
        };
    }
    if let Some(c) = per_use_cost(regions, name, n, visits) {
        return c;
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
        note: None,
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
        // an Azure class is Azure SQL whatever the flavor's first engine (RULES 131)
        let engine = if provider == "azure" {
            DbEngine::AzureSql
        } else {
            n.engine()
        };
        prices::managed_price_micro_usd_hr(engine, region, instance)
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

/// The cloud an entry node is in, from the prefix of its instance (`aws/t3.micro`); empty when it names
/// none (self-hosted without an instance, or your own instance).
fn provider_of(cfg: &Config, node: &str) -> String {
    cfg.nodes
        .get(node)
        .and_then(NodeConfig::instance_name)
        .and_then(|i| i.trim().split_once('/'))
        .map(|(p, _)| p.to_ascii_lowercase())
        .filter(|p| prices::DEFAULT_REGIONS.iter().any(|(d, _)| d == p))
        .unwrap_or_default()
}

/// What the streams of a run send out to the internet, by the cloud of each stream's entry node:
/// `streams` is `(endpoint key, response bytes a second)`, the stream's rate times its `resp_bytes`
/// (RULES 131). A stream whose endpoint has no entry node is left out.
#[must_use]
pub fn egress_flows<'a>(
    cfg: &Config,
    topo: &Topology,
    endpoints: &Endpoints,
    streams: impl IntoIterator<Item = (&'a str, f64)>,
) -> Vec<EgressFlow> {
    let mut by: BTreeMap<String, f64> = BTreeMap::new();
    for (key, bytes_per_s) in streams {
        let Some(entry) = endpoints.get(key).and_then(|e| e.entry_node) else {
            continue;
        };
        *by.entry(provider_of(cfg, &topo.nodes[entry].name))
            .or_default() += bytes_per_s;
    }
    by.into_iter()
        .filter(|(_, b)| *b > 0.0)
        .map(|(provider, bytes_per_s)| EgressFlow {
            provider,
            bytes_per_s,
        })
        .collect()
}

/// One cloud's egress fee an hour: the GB a month this rate sends, less the free allowance, at the
/// first-tier price. A flow with no cloud is priced at AWS's rate and says so.
fn egress_cost(regions: &BTreeMap<String, String>, f: &EgressFlow) -> EgressCost {
    let (provider, mut note) = if f.provider.is_empty() {
        (
            "aws".to_string(),
            Some("the entry node names no cloud: priced at AWS's rate, an estimate".to_string()),
        )
    } else {
        (f.provider.clone(), None)
    };
    let region = regions.get(&provider).cloned().unwrap_or_default();
    let micro = prices::egress_price(&provider, &region).map(|p| {
        let gb_month = f.bytes_per_s * 3600.0 * HOURS_PER_MONTH / (1024.0 * 1024.0 * 1024.0);
        let billable = (gb_month - f64::from(p.free_gb)).max(0.0);
        micro_usd_of_micro(billable * f64::from(p.micro_usd_per_gb) / HOURS_PER_MONTH)
    });
    if micro.is_none() {
        note = Some(format!("no egress price in {region}"));
    }
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let bytes_per_s = f.bytes_per_s.round() as u64;
    EgressCost {
        provider,
        region,
        bytes_per_s,
        micro_usd_hr: micro,
        note,
    }
}

/// Lambda's default memory when the node sets none: 128 MB, the smallest size and the console default.
const LAMBDA_DEFAULT_MEMORY_BYTES: f64 = 128.0 * 1024.0 * 1024.0;

/// A node billed by use (RULES 131): a serverless service or monolith as AWS Lambda (requests plus
/// GB-seconds of its own `proc` at its `memory:`), a managed gateway as API Gateway (a REST API's
/// requests). Priced from the run's visits a second; every copy's visits are in the one figure, so
/// `copies` is 1. `None` for any other node. Billed duration is the node's own mean `proc`: time spent
/// waiting on what it calls is not in it, so a Lambda that calls others costs more than this says.
fn per_use_cost(
    regions: &BTreeMap<String, String>,
    name: &str,
    n: &NodeConfig,
    visits: &BTreeMap<String, f64>,
) -> Option<NodeCost> {
    let kind = n.kind();
    let hosting = n.hosting();
    let lambda =
        hosting == Hosting::Serverless && matches!(kind, NodeKind::Service | NodeKind::Monolith);
    let gateway = hosting == Hosting::Managed && kind == NodeKind::Gateway;
    if !lambda && !gateway {
        return None;
    }
    let made = |micro: Option<u64>, why: Option<String>, note: Option<String>| NodeCost {
        name: name.to_string(),
        instance: None,
        copies: 1,
        unit_micro_usd_hr: micro,
        total_micro_usd_hr: micro,
        why,
        note,
    };
    let Some(rate) = visits.get(name).copied() else {
        let what = if lambda { "serverless" } else { "managed" };
        return Some(made(
            None,
            Some(format!(
                "{what}: priced by use, after a run (it needs the node's visits a second)"
            )),
            None,
        ));
    };
    let region = regions.get("aws").map_or("", String::as_str);
    let requests_hr = rate * 3600.0;
    let assumed = "priced as AWS Lambda and API Gateway (the node names no cloud): an estimate";
    if gateway {
        let Some(per_m) = prices::per_use_price("apigw_request", region) else {
            return Some(made(
                None,
                Some(format!("no API Gateway price in {region}")),
                None,
            ));
        };
        let micro = micro_usd_of_micro(requests_hr * f64::from(per_m) / 1e6);
        return Some(made(Some(micro), None, Some(assumed.to_string())));
    }
    let (Some(per_m), Some(per_k)) = (
        prices::per_use_price("lambda_request", region),
        prices::per_use_price("lambda_gb_second", region),
    ) else {
        return Some(made(
            None,
            Some(format!("no Lambda price in {region}")),
            None,
        ));
    };
    let (memory, mut note) = match n.memory() {
        Some(m) =>
        {
            #[allow(clippy::cast_precision_loss)]
            (m.limit as f64, String::from(assumed))
        }
        None => (
            LAMBDA_DEFAULT_MEMORY_BYTES,
            format!("{assumed}; no memory: set, so Lambda's 128 MB default"),
        ),
    };
    let secs = crate::dist::Dist::from_config(n.proc())
        .mean()
        .as_secs_f64();
    let gb_seconds_hr = requests_hr * secs * memory / (1024.0 * 1024.0 * 1024.0);
    let micro = micro_usd_of_micro(
        requests_hr * f64::from(per_m) / 1e6 + gb_seconds_hr * f64::from(per_k) / 1e3,
    );
    note.push('.');
    Some(made(Some(micro), None, Some(note)))
}
