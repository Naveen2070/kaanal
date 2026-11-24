//! Provider translation (Step 9c-2, plan Task 1): the design moved to another cloud's instance
//! classes, so the planner can price "the same design on GCP" next to the one in force. A pure
//! function of the design and the catalogue: no I/O, the input is never changed.
//!
//! The rule is a substitution by specification, the classic "equivalent instance" match the clouds'
//! own migration tools use (AWS Migration Evaluator, Azure Migrate right-sizing, Google's Migration
//! Center): each table class goes to the **cheapest priced class of the target** whose vCPU and
//! memory are each at least the original's, and whose network rate is at least the original's when
//! both give one (`net_mbps == 0` means the source gives none: not compared). It assumes a vCPU is a
//! vCPU across clouds (the table's own caveat: capacity, not speed); the planner's check runs every
//! candidate, so a wrong guess costs a candidate, never a wrong verdict. Prices are integer
//! micro-USD. Ties go to the first name in the table's order (sorted by name).
//!
//! A managed database class goes to the target's managed class only where the node's flavor runs
//! there (an Azure class is Azure SQL, which runs `sqlserver` only, the rule of the family
//! alternatives; GCP has no managed class in the table), else the whole translation fails with the
//! node named. A node in `keep` that translation would move (a table class) fails it too: it would
//! stay on the provider left while the rest moved, a mixed design, which is out of scope (egress and
//! cross-cloud latency are not modelled). Everything else (a kept node with no table class, a cache,
//! an object store, an external API, a serverless or external node, a node with no class, a custom
//! instance) is kept as written and listed in [`Translation::notes`].

use crate::config::{Config, DbEngine, DbFlavor, Hosting, NodeConfig, NodeKind};
use crate::instances;

/// A design moved to another cloud: the whole candidate config and what was not moved, in words.
#[derive(Debug, Clone)]
pub struct Translation {
    /// A clone of the design with every movable class on the target cloud and its region written.
    pub config: Config,
    /// One line per node: the move made, or "kept as written, not compared (why)".
    pub notes: Vec<String>,
}

/// The providers of the table classes in `cfg` (`aws`, `azure`, `gcp`), sorted, deduplicated. A
/// custom instance (no `/`) names none.
#[must_use]
pub fn providers_of(cfg: &Config) -> Vec<String> {
    let mut v: Vec<String> = cfg
        .nodes
        .values()
        .filter_map(|n| n.instance_name()?.split_once('/'))
        .map(|(p, _)| p.trim().to_ascii_lowercase())
        .collect();
    v.sort_unstable();
    v.dedup();
    v
}

/// `cfg` with every table class moved to the cheapest class of `to` at least as big (see the module
/// doc), nodes in `keep` untouched.
///
/// The region of `to` is the one [`crate::cost::regions_of`] gives (the file's `region:` for `to`,
/// else the cloud's default in [`crate::prices::DEFAULT_REGIONS`]); `region:` is not changed.
///
/// # Errors
/// The reason the whole translation is dropped: the design already uses `to` (`"already on <to>"`),
/// `to` has no price list, a managed database's flavor has no managed class of `to` in the table
/// (`"<node>: no managed <flavor> class for <to> in the price table"`), a node has no class big
/// enough on `to` (`"<node>: no equivalent on <to>"`), a kept node, or one translation cannot move (a cache, say),
/// has a table class (`"<node>: kept on <provider>, so a section on <to> would mix providers"` or
/// `"<node>: <why> on <provider> has no translation, ..."`), or the result fails
/// [`Config::validate_candidate`].
pub fn translate(cfg: &Config, to: &str, keep: &[String]) -> Result<Translation, String> {
    if providers_of(cfg).iter().any(|p| p == to) {
        return Err(format!("already on {to}"));
    }
    // the design spec: the target's region follows `cost::regions_of` (the file's `region:` for
    // `to`, else its default), so nothing is written: the candidate prices where the estimate will
    let region = crate::cost::regions_of(cfg)
        .remove(to)
        .ok_or_else(|| format!("{to} has no price list"))?;
    let mut config = cfg.clone();
    let mut notes = Vec::new();
    for (name, n) in &mut config.nodes {
        let kept = |why: &str| format!("{name}: kept as written, not compared ({why})");
        let mine = keep.iter().any(|k| k == name);
        let from = match (movable(n), n.instance()) {
            (Ok(()), Some(from)) if mine => {
                let p = from.name.split_once('/').map_or(from.name, |(p, _)| p);
                return Err(format!(
                    "{name}: kept on {p}, so a section on {to} would mix providers"
                ));
            }
            _ if mine => {
                notes.push(kept("kept by you"));
                continue;
            }
            // a node translation cannot move, on a class of the cloud being left: the section
            // would price a mix (a cache on aws beside a service on gcp)
            (Err(why), Some(from)) if from.name.contains('/') => {
                let p = from.name.split_once('/').map_or(from.name, |(p, _)| p);
                return Err(format!(
                    "{name}: {why} on {p} has no translation, so a section on {to} would mix providers"
                ));
            }
            (Err(why), _) => {
                notes.push(kept(why));
                continue;
            }
            (Ok(()), None) => {
                notes.push(kept("no class"));
                continue;
            }
            (Ok(()), Some(from)) => from,
        };
        if n.kind() == NodeKind::Database && n.hosting() == Hosting::Managed {
            let was = n.engine_written();
            set_engine(n, to).map_err(|what| {
                format!("{name}: no managed {what} class for {to} in the price table")
            })?;
            if let (Some(a), Some(b)) = (was, n.engine_written())
                && a != b
            {
                notes.push(format!("{name}: engine {} to {}", a.word(), b.word()));
            }
        }
        let class = cheapest(n, from, to, &region)
            .ok_or_else(|| format!("{name}: no equivalent on {to}"))?;
        notes.push(format!("{name}: {} to {class}", from.name));
        *super::moves::instance_slot(n)
            .ok_or_else(|| format!("{name}: its kind has no instance field"))? =
            Some(class.to_string());
    }
    config.validate_candidate()?;
    Ok(Translation { config, notes })
}

/// **Private.** Whether node `n` is one translation moves, `keep` aside, or why it is kept as written.
fn movable(n: &NodeConfig) -> Result<(), &'static str> {
    match n.kind() {
        NodeKind::Cache => return Err("a cache"),
        NodeKind::Storage => return Err("an object store"),
        NodeKind::ExternalApi => return Err("an external service"),
        _ => {}
    }
    match n.hosting() {
        Hosting::Serverless => return Err("serverless"),
        Hosting::External => return Err("run by someone else"),
        Hosting::Managed if n.kind() != NodeKind::Database => return Err("a managed service"),
        _ => {}
    }
    if n.instance_name().is_some_and(|i| !i.contains('/')) {
        return Err("your own instance");
    }
    Ok(())
}

/// **Private.** Points a managed database's engine at one that runs on `to`, or gives the flavor's
/// word (`postgres` when none is written, the default) when the table has no class of `to` for it. An Azure class runs Azure SQL only (RULES 131, as [`super::family`]
/// checks); an AWS class runs every engine but Azure SQL. A written engine is rewritten so the cost
/// estimate and the hosting check read the new cloud's; an unwritten one already follows the flavor.
fn set_engine(n: &mut NodeConfig, to: &str) -> Result<(), &'static str> {
    let NodeConfig::Database(d) = n else {
        return Err("database");
    };
    let what = d.flavor.map_or("postgres", DbFlavor::as_str);
    let runs = |e: DbEngine| {
        d.flavor
            .map_or(e == DbEngine::Postgresql, |f| f.engines().contains(&e))
    };
    let engine = match to {
        "azure" => (runs(DbEngine::AzureSql) || d.engine == Some(DbEngine::AzureSql))
            .then_some(DbEngine::AzureSql),
        "aws" => match d.engine {
            Some(e) if e != DbEngine::AzureSql => Some(e),
            _ => match d.flavor {
                Some(f) => f
                    .engines()
                    .iter()
                    .copied()
                    .find(|e| *e != DbEngine::AzureSql),
                None => Some(DbEngine::Postgresql),
            },
        },
        _ => None, // no other cloud has a managed database class in the table (GCP Cloud SQL is not priced)
    }
    .ok_or(what)?;
    if d.engine.is_some() {
        d.engine = Some(engine);
    }
    Ok(())
}

/// **Private.** The cheapest class of `to` of the same sort as `from` (a managed database class
/// for a managed database, else a compute class), priced from the list the node's hosting and engine
/// use in `region`, at least as big as `from`. O(table) with no allocation. Ties on price go to the
/// smaller class (fewer vCPU, then less memory), then the first name: the key is total, so the
/// pick is deterministic.
fn cheapest(
    n: &NodeConfig,
    from: &instances::Instance,
    to: &str,
    region: &str,
) -> Option<&'static str> {
    let db = n.hosting() == Hosting::Managed;
    instances::all()
        .iter()
        .filter(|i| {
            i.name.strip_prefix(to).is_some_and(|r| r.starts_with('/'))
                && instances::is_db_class(i.name) == db
                && !instances::is_cache_class(i.name)
                && i.vcpu >= from.vcpu
                && i.memory_mib >= from.memory_mib
                && (i.net_mbps == 0 || from.net_mbps == 0 || i.net_mbps >= from.net_mbps)
        })
        .filter_map(|i| crate::cost::class_price(n, to, region, i.name).map(|p| (p, i)))
        .min_by_key(|(p, i)| (*p, i.vcpu, i.memory_mib, i.name))
        .map(|(_, i)| i.name)
}
