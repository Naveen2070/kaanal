//! Database-family candidates (RULES 141, plan Task 3): what a database node would look like as a
//! database of the other family (relational against document, key-value, wide-column and graph), so
//! the planner can offer "a different database" next to "a bigger one".
//!
//! A flavor changes the simulation only through its preset ops ([`crate::presets::db_ops`]); ops the
//! developer wrote are kept whatever the flavor says, so a node with written ops gets no
//! alternative and a reason. Hosting follows the new flavor: a managed node stays managed where the
//! new flavor has a managed engine and the class it names is priced for that engine, and becomes
//! `self` (class turned into its compute twin, `aws/db.m5.large` to `aws/m5.large`, else no
//! instance) where it has none (Cassandra, `DynamoDB`), with a note. An alternative whose config
//! fails [`Config::validate_candidate`] (which includes the hosting rules) or whose class has no
//! price for the new engine is dropped, with the reason in [`Alternatives::dropped`].

use crate::config::{Config, DbFlavor, Hosting, NodeConfig};
use crate::instances;

/// The two families the planner tries against each other.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    /// SQL databases: PostgreSQL, MySQL, MariaDB, SQL Server, Oracle.
    Relational,
    /// Everything else: MongoDB, `DynamoDB`, Cassandra, Neo4j.
    NonRelational,
}

/// The family of `flavor`.
#[must_use]
pub const fn family_of(flavor: DbFlavor) -> Family {
    match flavor {
        DbFlavor::Postgres
        | DbFlavor::Mysql
        | DbFlavor::Mariadb
        | DbFlavor::Sqlserver
        | DbFlavor::Oracle => Family::Relational,
        DbFlavor::Mongodb | DbFlavor::Dynamodb | DbFlavor::Cassandra | DbFlavor::Neo4j => {
            Family::NonRelational
        }
    }
}

/// **Private.** Every flavor of a family, in the order the planner tries them.
const fn flavors_of(f: Family) -> &'static [DbFlavor] {
    match f {
        Family::Relational => &[
            DbFlavor::Postgres,
            DbFlavor::Mysql,
            DbFlavor::Mariadb,
            DbFlavor::Sqlserver,
            DbFlavor::Oracle,
        ],
        Family::NonRelational => &[
            DbFlavor::Mongodb,
            DbFlavor::Dynamodb,
            DbFlavor::Cassandra,
            DbFlavor::Neo4j,
        ],
    }
}

/// One database of the other family, as a whole candidate config.
#[derive(Debug, Clone)]
pub struct Alternative {
    /// The node.
    pub node: String,
    /// The new flavor.
    pub flavor: DbFlavor,
    /// A clone of the design with the node changed (nothing else touched).
    pub config: Config,
    /// What else changed because of the flavor (hosting, class), in plain words.
    pub notes: Vec<String>,
}

/// The alternatives for one node: those that hold, and those dropped with the reason.
#[derive(Debug, Clone, Default)]
pub struct Alternatives {
    /// Candidates that pass the checks.
    pub kept: Vec<Alternative>,
    /// Flavors tried and dropped, with why.
    pub dropped: Vec<(DbFlavor, String)>,
}

/// The flavors of the **other** family for database `node` (`neo4j` only when `with_graph`).
///
/// # Errors
/// The reason there is nothing to try: not a database, not in the design, in `keep`, or it has
/// `ops` the developer wrote (a flavor would change nothing in the simulation).
pub fn alternatives(
    cfg: &Config,
    node: &str,
    keep: &[String],
    with_graph: bool,
) -> Result<Alternatives, &'static str> {
    if keep.iter().any(|k| k == node) {
        return Err("kept by you");
    }
    let n = cfg.nodes.get(node).ok_or("not in the design")?;
    let NodeConfig::Database(d) = n else {
        return Err("not a database");
    };
    if d.ops.is_some() {
        return Err("your own ops are kept: a flavor changes nothing in the simulation");
    }
    let from = d.flavor.ok_or("no flavor written, so no family to leave")?;
    let other = match family_of(from) {
        Family::Relational => Family::NonRelational,
        Family::NonRelational => Family::Relational,
    };
    let mut out = Alternatives::default();
    for &flavor in flavors_of(other) {
        if flavor == DbFlavor::Neo4j && !with_graph {
            continue;
        }
        match build(cfg, node, flavor) {
            Ok(a) => out.kept.push(a),
            Err(why) => out.dropped.push((flavor, why)),
        }
    }
    Ok(out)
}

/// **Private.** `cfg` with `node` turned into `flavor`, hosting following the flavor.
fn build(cfg: &Config, node: &str, flavor: DbFlavor) -> Result<Alternative, String> {
    let mut config = cfg.clone();
    let mut notes = Vec::new();
    let Some(NodeConfig::Database(d)) = config.nodes.get_mut(node) else {
        return Err("not a database".into());
    };
    d.flavor = Some(flavor);
    let engines = flavor.engines();
    if d.hosting == Some(Hosting::Managed) {
        if engines.is_empty() {
            // no managed class runs it: the node becomes a machine you run (spec: hosting follows)
            d.hosting = Some(Hosting::SelfHosted);
            d.engine = None;
            d.multi_az = None;
            let was = d.instance.take();
            // your own instance (no '/') is a valid machine for a self-hosted node: keep it
            let twin = was.as_deref().and_then(|w| {
                if w.contains('/') {
                    compute_twin(w).map(str::to_string)
                } else {
                    Some(w.to_string())
                }
            });
            notes.push(format!(
                "{} has no managed class in the table: hosting moves from managed to self{}; engine and multi_az are dropped",
                flavor.as_str(),
                match (&was, &twin) {
                    (Some(w), Some(t)) if w != t => format!(", class {w} becomes {t}"),
                    (Some(w), Some(_)) => format!(", your instance {w} is kept"),
                    (Some(w), None) => format!(", class {w} has no compute twin so no instance is set"),
                    (None, _) => String::new(),
                }
            ));
            d.instance = twin;
        } else if d.engine.is_some_and(|e| !engines.contains(&e)) {
            d.engine = None;
            notes.push(format!(
                "engine moves to the default for {}",
                flavor.as_str()
            ));
        }
    }
    config.validate_candidate()?;
    price_check(&config, node, flavor)?;
    Ok(Alternative {
        node: node.to_string(),
        flavor,
        config,
        notes,
    })
}

/// **Private.** The compute class of a managed database class: the same name without `db.`, when
/// the table has it.
fn compute_twin(class: &str) -> Option<&'static str> {
    let (provider, ty) = class.split_once('/')?;
    let twin = format!("{provider}/{}", ty.strip_prefix("db.")?);
    instances::lookup(&twin).map(|i| i.name)
}

/// **Private.** A managed database that keeps a table class needs a price for it under the new
/// engine (the plan prices every candidate, and its class ladder is priced by the same list).
fn price_check(cfg: &Config, node: &str, flavor: DbFlavor) -> Result<(), String> {
    let n = &cfg.nodes[node];
    let Some(class) = n
        .instance_name()
        .filter(|_| n.hosting() == Hosting::Managed)
    else {
        return Ok(());
    };
    let Some((provider, _)) = class.split_once('/') else {
        return Ok(()); // your own instance: its cost_per_hour
    };
    let regions = crate::cost::regions_of(cfg);
    let region = regions.get(provider).map_or("", String::as_str);
    if provider == "azure"
        && !flavor
            .engines()
            .contains(&crate::config::DbEngine::AzureSql)
    {
        return Err(format!(
            "{class} is an Azure SQL class and {} does not run on it",
            flavor.as_str()
        ));
    }
    if crate::cost::class_price(n, provider, region, class).is_none() {
        return Err(format!(
            "{class} has no {} price in {region}",
            flavor.as_str()
        ));
    }
    Ok(())
}
