//! Where a component runs (`hosting:`, RULES 122): on a machine you run (`self`, the default), on a
//! provider's managed service (`managed`: a database class like `aws/db.m5.large`, or a managed
//! gateway with no machine to size), as a pay-per-use service (`serverless`), or run by someone
//! else (`external`). What `hosting:` decides is which `instance:` the node may name and how the
//! cost estimate prices it: only `self` takes a VM type, only a `managed` database takes a managed
//! class, and the other hostings take no instance (cores and memory stay yours to write, if you
//! model them).

use super::{Config, DbFlavor, Deserialize, JsonSchema, NodeKind};
use crate::diagnostics::{Located, keys};

/// Where the component runs (`hosting:`).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, JsonSchema)]
pub enum Hosting {
    /// A machine you run (a VM, a container host, a server of your own). `instance:` names its VM
    /// type. The default.
    #[default]
    #[serde(rename = "self")]
    SelfHosted,
    /// The provider runs it and you pick a size: a managed database (`aws/db.m5.large`), or a
    /// managed gateway or load balancer (no size to pick).
    #[serde(rename = "managed")]
    Managed,
    /// A pay-per-use service with no machine to pick (Lambda, Cloud Run, `DynamoDB`, Aurora
    /// Serverless).
    #[serde(rename = "serverless")]
    Serverless,
    /// Run by someone else (a hosted identity provider, a third-party database): only its latency
    /// and limits are yours.
    #[serde(rename = "external")]
    External,
}

impl Hosting {
    /// The word written in the file.
    #[must_use]
    pub const fn word(self) -> &'static str {
        match self {
            Hosting::SelfHosted => "self",
            Hosting::Managed => "managed",
            Hosting::Serverless => "serverless",
            Hosting::External => "external",
        }
    }
}

/// The engine of a managed database (`engine:`): its price depends on it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, JsonSchema)]
pub enum DbEngine {
    /// PostgreSQL (RDS). The default.
    #[default]
    #[serde(rename = "postgresql")]
    Postgresql,
    /// MySQL (RDS).
    #[serde(rename = "mysql")]
    Mysql,
    /// MariaDB (RDS).
    #[serde(rename = "mariadb")]
    Mariadb,
    /// Aurora PostgreSQL.
    #[serde(rename = "aurora-postgresql")]
    AuroraPostgresql,
    /// Aurora MySQL.
    #[serde(rename = "aurora-mysql")]
    AuroraMysql,
}

impl DbEngine {
    /// The word written in the file, which is also the suffix of its price files (`aws-rds-<word>`).
    #[must_use]
    pub const fn word(self) -> &'static str {
        match self {
            DbEngine::Postgresql => "postgresql",
            DbEngine::Mysql => "mysql",
            DbEngine::Mariadb => "mariadb",
            DbEngine::AuroraPostgresql => "aurora-postgresql",
            DbEngine::AuroraMysql => "aurora-mysql",
        }
    }
}

impl DbFlavor {
    /// The managed engines whose classes run this flavor (the first is the default): only the
    /// relational flavors have one in the table (AWS RDS). `MongoDB`, `Neo4j`, `Cassandra` and
    /// `DynamoDB` have none: Amazon `DocumentDB`, Neptune, Keyspaces and Atlas are not in the public
    /// catalogue the table is built from, so a managed one is sized and priced by you.
    #[must_use]
    pub fn engines(self) -> &'static [DbEngine] {
        use DbEngine::{AuroraMysql, AuroraPostgresql, Mariadb, Mysql, Postgresql};
        match self {
            DbFlavor::Postgres => &[Postgresql, AuroraPostgresql],
            DbFlavor::Mysql => &[Mysql, AuroraMysql],
            DbFlavor::Mariadb => &[Mariadb],
            DbFlavor::Mongodb | DbFlavor::Neo4j | DbFlavor::Cassandra | DbFlavor::Dynamodb => &[],
        }
    }
}

impl NodeKind {
    /// The hostings this kind can have, the default first; empty for a kind that has no machine to
    /// choose (cache, external API, storage).
    #[must_use]
    pub fn hostings(self) -> &'static [Hosting] {
        use Hosting::{External, Managed, SelfHosted, Serverless};
        match self {
            NodeKind::Gateway | NodeKind::LoadBalancer => &[SelfHosted, Managed],
            NodeKind::Service | NodeKind::Monolith => &[SelfHosted, Serverless],
            NodeKind::Database => &[SelfHosted, Managed, Serverless, External],
            NodeKind::Auth => &[SelfHosted, External],
            NodeKind::Cache => &[SelfHosted, Managed, Serverless],
            NodeKind::ExternalApi | NodeKind::Storage => &[],
        }
    }
}

/// **Private.** Whether `name` is the managed class of `kind`'s service: a database class for a
/// database, a cache class for a cache.
fn is_own_class(kind: NodeKind, name: &str) -> bool {
    match kind {
        NodeKind::Database => crate::instances::is_db_class(name),
        NodeKind::Cache => crate::instances::is_cache_class(name),
        _ => false,
    }
}

/// Checks each node's `hosting:`, `engine:` and `instance:` agree: the kind allows the hosting, a
/// VM type goes with `self` only, a managed class with a `managed` database only, `engine:` with a
/// managed database. A custom instance (a name with no `/`) goes with `self` or `managed`.
///
/// # Errors
/// A [`Located`] at the first node that disagrees.
pub(super) fn check(cfg: &Config) -> Result<(), Located> {
    for (node, n) in &cfg.nodes {
        let kind = n.kind();
        let hosting = n.hosting();
        let at = |field: &str| keys(&["nodes", node.as_str(), field]);
        if let Some(h) = n.hosting_written() {
            let ok = kind.hostings();
            if !ok.contains(&h) {
                let allowed = if ok.is_empty() {
                    "this kind has no hosting to choose".to_string()
                } else {
                    format!(
                        "a {} takes {}",
                        kind_word(kind),
                        ok.iter().map(|x| x.word()).collect::<Vec<_>>().join(", ")
                    )
                };
                return Err(Located::at_value(
                    at("hosting"),
                    format!(
                        "node '{node}': hosting '{}' is not allowed ({allowed})",
                        h.word()
                    ),
                ));
            }
        }
        if let Some(i) = n.instance_name() {
            let managed_class = crate::instances::is_managed_class(i);
            let custom = !i.contains('/');
            let bad = match (hosting, kind) {
                (Hosting::SelfHosted, _) => managed_class.then(|| {
                    format!("instance '{i}' is a managed class: set hosting: managed on a database or a cache, or name a VM type")
                }),
                (Hosting::Managed, NodeKind::Database | NodeKind::Cache) => {
                    (!is_own_class(kind, i) && !custom).then(|| {
                        let (what, eg) = if kind == NodeKind::Database {
                            ("database", "aws/db.m5.large")
                        } else {
                            ("cache", "aws/cache.m5.large")
                        };
                        format!("instance '{i}' is not a {what} class: a managed {what} takes a class like {eg}")
                    })
                }
                _ => Some(format!(
                    "hosting: {} has no instance to pick (remove instance: '{i}', or use hosting: self)",
                    hosting.word()
                )),
            };
            if let Some(why) = bad {
                return Err(Located::at_value(
                    at("instance"),
                    format!("node '{node}': {why}"),
                ));
            }
        }
        if kind == NodeKind::Database
            && hosting == Hosting::Managed
            && let Some(flavor) = n.db_flavor()
        {
            let fits = flavor.engines();
            let class = n.instance_name().is_some_and(crate::instances::is_db_class);
            if class && fits.is_empty() {
                return Err(Located::at_value(
                    at("instance"),
                    format!(
                        "node '{node}': {} has no managed class in the table (aws/db.* are RDS: postgres, mysql, mariadb); use your own instance with cost_per_hour, or leave instance out",
                        flavor.as_str()
                    ),
                ));
            }
            if let Some(e) = n.engine_written()
                && !fits.contains(&e)
            {
                let list: Vec<_> = fits.iter().map(|x| x.word()).collect();
                let msg = if fits.is_empty() {
                    format!(
                        "node '{node}': {} has no managed engine in the table, so engine: does not apply",
                        flavor.as_str()
                    )
                } else {
                    format!(
                        "node '{node}': engine '{}' does not run {} (use {})",
                        e.word(),
                        flavor.as_str(),
                        list.join(" or ")
                    )
                };
                return Err(Located::at_value(at("engine"), msg));
            }
        }
        if n.engine_written().is_some()
            && !(kind == NodeKind::Database && hosting == Hosting::Managed)
        {
            return Err(Located::at_value(
                at("engine"),
                format!("node '{node}': engine goes with a database that has hosting: managed"),
            ));
        }
    }
    Ok(())
}

/// **Private.** A kind's name for a message.
fn kind_word(k: NodeKind) -> &'static str {
    match k {
        NodeKind::Gateway => "gateway",
        NodeKind::Service => "service",
        NodeKind::Monolith => "monolith",
        NodeKind::Database => "database",
        NodeKind::Cache => "cache",
        NodeKind::ExternalApi => "external API",
        NodeKind::Auth => "auth node",
        NodeKind::LoadBalancer => "load balancer",
        NodeKind::Storage => "storage node",
    }
}
