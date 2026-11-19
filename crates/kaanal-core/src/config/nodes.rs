//! The node kinds and their per-kind config structs (gateway, service, monolith, database, cache, external API, auth) with the flavor enums.

use super::kinds::{
    AuthConfig, CacheConfig, DatabaseConfig, ExternalApiConfig, GatewayConfig, MonolithConfig,
    ServiceConfig,
};
use super::storage::{StorageConfig, StorageFlavor};
use super::*;
use crate::balance::Balance;

/// The kind of a topology node (`type:` in YAML).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NodeKind {
    /// A reverse proxy / API gateway / load balancer; optional, at most one.
    Gateway,
    /// A backend service; may carry an `OpenAPI` `spec`. Never has `ops`.
    Service,
    /// A service that may also run its own `ops` (an inline database).
    Monolith,
    /// A data store with `ops` costs (read/write/scan).
    Database,
    /// A cache with a hit ratio and hit/miss costs.
    Cache,
    /// A third-party HTTP API.
    ExternalApi,
    /// An identity provider (login and token issue).
    Auth,
    /// A load balancer: picks one of its callee components for each call.
    LoadBalancer,
    /// An object store (S3, GCS, Azure Blob): a leaf with a first-byte time and a throughput.
    Storage,
}

/// What a node's slot covers (`hold:`), for a node that calls others.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Hold {
    /// The slot is held only while the node does its own work. A server that waits on its calls
    /// without a thread (an event loop: Node, Go, async Python). The default.
    #[default]
    Own,
    /// The slot is held from the moment the request is admitted until everything the node calls
    /// has answered. A server with a thread or worker per request (Tomcat, sync Gunicorn, Puma,
    /// PHP-FPM): a slow dependency fills the node's slots and its queue grows.
    Call,
}

impl NodeKind {
    /// Every kind, in the order Studio lists them. A new kind goes here too: the draft view's
    /// `kinds` and `kind_defaults` are built from this list.
    pub const ALL: [NodeKind; 9] = [
        NodeKind::Gateway,
        NodeKind::Service,
        NodeKind::Monolith,
        NodeKind::Database,
        NodeKind::Cache,
        NodeKind::ExternalApi,
        NodeKind::Auth,
        NodeKind::LoadBalancer,
        NodeKind::Storage,
    ];

    /// Whether the kind has one `proc` service time (the others price their work with `ops`,
    /// `hit` and `miss`; storage with `first_byte` and `throughput`). Studio shows a Time control
    /// only for these.
    pub fn has_proc(self) -> bool {
        match self {
            NodeKind::Gateway
            | NodeKind::Service
            | NodeKind::Monolith
            | NodeKind::ExternalApi
            | NodeKind::Auth
            | NodeKind::LoadBalancer => true,
            NodeKind::Database | NodeKind::Cache | NodeKind::Storage => false,
        }
    }

    /// The status a call turned away by this kind's `rate_limit` gets: 503 for storage (S3's
    /// "Slow Down"), 429 for every other kind.
    pub fn rate_limit_status(self) -> u16 {
        if self == NodeKind::Storage { 503 } else { 429 }
    }

    /// The YAML spelling (`external-api`), also used in status output and messages.
    pub fn as_str(self) -> &'static str {
        match self {
            NodeKind::Gateway => "gateway",
            NodeKind::Service => "service",
            NodeKind::Monolith => "monolith",
            NodeKind::Database => "database",
            NodeKind::Cache => "cache",
            NodeKind::ExternalApi => "external-api",
            NodeKind::Auth => "auth",
            NodeKind::LoadBalancer => "load-balancer",
            NodeKind::Storage => "storage",
        }
    }
}

impl NodeKind {
    /// The kind written `name` in YAML (`external-api`), if it is one.
    pub fn from_name(name: &str) -> Option<NodeKind> {
        NodeKind::ALL.into_iter().find(|k| k.as_str() == name)
    }
}

impl fmt::Display for NodeKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One node in `nodes:`. `type` selects the kind; each kind rejects fields it does not have.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum NodeConfig {
    /// A reverse proxy / API gateway / load balancer.
    Gateway(GatewayConfig),
    /// A backend service with an optional `OpenAPI` spec; never has `ops`.
    Service(ServiceConfig),
    /// A service that may also run its own read/write/scan (`ops`), or call databases.
    Monolith(MonolithConfig),
    /// A data store with read/write/scan costs.
    Database(DatabaseConfig),
    /// A cache with a hit ratio and hit/miss costs.
    Cache(CacheConfig),
    /// A third-party HTTP API (payments, email, …).
    ExternalApi(ExternalApiConfig),
    /// An identity provider: login and token issue.
    Auth(AuthConfig),
    /// A load balancer: a real hop that spreads calls over several components.
    LoadBalancer(LoadBalancerConfig),
    /// An object store: a leaf with `first_byte` and `throughput`.
    Storage(StorageConfig),
}

impl NodeConfig {
    /// The node's kind.
    pub fn kind(&self) -> NodeKind {
        match self {
            NodeConfig::Gateway(_) => NodeKind::Gateway,
            NodeConfig::Service(_) => NodeKind::Service,
            NodeConfig::Monolith(_) => NodeKind::Monolith,
            NodeConfig::Database(_) => NodeKind::Database,
            NodeConfig::Cache(_) => NodeKind::Cache,
            NodeConfig::ExternalApi(_) => NodeKind::ExternalApi,
            NodeConfig::Auth(_) => NodeKind::Auth,
            NodeConfig::LoadBalancer(_) => NodeKind::LoadBalancer,
            NodeConfig::Storage(_) => NodeKind::Storage,
        }
    }

    /// Processing time per request (gateway, service, monolith, external-api, auth, load
    /// balancer); a storage node's is its `first_byte`.
    pub fn proc(&self) -> Option<&DistConfig> {
        match self {
            NodeConfig::Storage(n) => Some(&n.first_byte),
            NodeConfig::Gateway(n) => n.proc.as_ref(),
            NodeConfig::Service(n) => n.proc.as_ref(),
            NodeConfig::Monolith(n) => n.proc.as_ref(),
            NodeConfig::ExternalApi(n) => n.proc.as_ref(),
            NodeConfig::Auth(n) => n.proc.as_ref(),
            NodeConfig::LoadBalancer(n) => n.proc.as_ref(),
            NodeConfig::Database(_) | NodeConfig::Cache(_) => None,
        }
    }

    /// Named chains (service and monolith).
    pub fn chains(&self) -> Option<&IndexMap<String, Vec<StepConfig>>> {
        match self {
            NodeConfig::Service(n) => Some(&n.chains),
            NodeConfig::Monolith(n) => Some(&n.chains),
            _ => None,
        }
    }

    /// `OpenAPI` spec file, relative to the config's directory (service, monolith).
    pub fn spec(&self) -> Option<&str> {
        match self {
            NodeConfig::Service(n) => n.spec.as_deref(),
            NodeConfig::Monolith(n) => n.spec.as_deref(),
            _ => None,
        }
    }

    /// Slot limit and queue; `None` = unlimited.
    pub fn concurrency(&self) -> Option<&ConcurrencyConfig> {
        match self {
            NodeConfig::Gateway(n) => n.concurrency.as_ref(),
            NodeConfig::Service(n) => n.concurrency.as_ref(),
            NodeConfig::Monolith(n) => n.concurrency.as_ref(),
            NodeConfig::Database(n) => n.concurrency.as_ref(),
            NodeConfig::Cache(n) => n.concurrency.as_ref(),
            NodeConfig::ExternalApi(n) => n.concurrency.as_ref(),
            NodeConfig::Auth(n) => n.concurrency.as_ref(),
            NodeConfig::LoadBalancer(n) => n.concurrency.as_ref(),
            NodeConfig::Storage(n) => n.concurrency.as_ref(),
        }
    }

    /// The connection pool (database, external API).
    pub fn pool(&self) -> Option<&PoolConfig> {
        match self {
            NodeConfig::Database(n) => n.pool.as_ref(),
            NodeConfig::ExternalApi(n) => n.pool.as_ref(),
            _ => None,
        }
    }

    /// The token-bucket rate limit.
    pub fn rate_limit(&self) -> Option<&RateLimitConfig> {
        match self {
            NodeConfig::Gateway(n) => n.rate_limit.as_ref(),
            NodeConfig::Service(n) => n.rate_limit.as_ref(),
            NodeConfig::Monolith(n) => n.rate_limit.as_ref(),
            NodeConfig::Database(n) => n.rate_limit.as_ref(),
            NodeConfig::Cache(n) => n.rate_limit.as_ref(),
            NodeConfig::ExternalApi(n) => n.rate_limit.as_ref(),
            NodeConfig::Auth(n) => n.rate_limit.as_ref(),
            NodeConfig::LoadBalancer(n) => n.rate_limit.as_ref(),
            NodeConfig::Storage(n) => n.rate_limit.as_ref(),
        }
    }

    /// The per-call deadline (`timeout:`).
    pub fn timeout(&self) -> Option<Duration> {
        match self {
            NodeConfig::Gateway(n) => n.timeout,
            NodeConfig::Service(n) => n.timeout,
            NodeConfig::Monolith(n) => n.timeout,
            NodeConfig::Database(n) => n.timeout,
            NodeConfig::Cache(n) => n.timeout,
            NodeConfig::ExternalApi(n) => n.timeout,
            NodeConfig::Auth(n) => n.timeout,
            NodeConfig::LoadBalancer(n) => n.timeout,
            NodeConfig::Storage(n) => n.timeout,
        }
    }

    /// Copies of this node as written (`replicas:`); 1 when absent. Range-checked at load.
    pub fn replicas(&self) -> u16 {
        match self {
            NodeConfig::Gateway(n) => n.replicas,
            NodeConfig::Service(n) => n.replicas,
            NodeConfig::Monolith(n) => n.replicas,
            NodeConfig::Database(n) => n.replicas,
            NodeConfig::Cache(n) => n.replicas,
            NodeConfig::ExternalApi(n) => n.replicas,
            NodeConfig::Auth(n) => n.replicas,
            NodeConfig::LoadBalancer(n) => n.replicas,
            NodeConfig::Storage(n) => n.replicas,
        }
        .unwrap_or(1)
    }

    /// How traffic entering this node picks a replica, if the node sets one.
    pub fn balance(&self) -> Option<&Balance> {
        match self {
            NodeConfig::Gateway(n) => n.balance.as_ref(),
            NodeConfig::Service(n) => n.balance.as_ref(),
            NodeConfig::Monolith(n) => n.balance.as_ref(),
            NodeConfig::Database(n) => n.balance.as_ref(),
            NodeConfig::Cache(n) => n.balance.as_ref(),
            NodeConfig::ExternalApi(n) => n.balance.as_ref(),
            NodeConfig::Auth(n) => n.balance.as_ref(),
            NodeConfig::LoadBalancer(n) => n.balance.as_ref(),
            NodeConfig::Storage(n) => n.balance.as_ref(),
        }
    }

    /// The node's CPUs (`cpu:`); `None` = no sharing. Only the kinds whose compute the user sizes
    /// have the field: gateway, service, monolith, database, auth and load balancer.
    pub fn cpu(&self) -> Option<CpuConfig> {
        // a cache's instance is a size note and a price: its hits are not CPU-shared, so it fills no cores
        self.own_cpu().or_else(|| {
            self.instance()
                .filter(|_| !matches!(self, NodeConfig::Cache(_)))
                .map(|i| CpuConfig {
                    cores: f64::from(i.vcpu),
                })
        })
    }

    /// **Private.** The `cpu:` the node itself sets.
    fn own_cpu(&self) -> Option<CpuConfig> {
        match self {
            NodeConfig::Gateway(n) => n.cpu,
            NodeConfig::Service(n) => n.cpu,
            NodeConfig::Monolith(n) => n.cpu,
            NodeConfig::Database(n) => n.cpu,
            NodeConfig::Auth(n) => n.cpu,
            NodeConfig::LoadBalancer(n) => n.cpu,
            NodeConfig::Cache(_) | NodeConfig::ExternalApi(_) | NodeConfig::Storage(_) => None,
        }
    }

    /// The node's network card (`bandwidth:`) as `(send, receive)` bytes per second; `None` =
    /// unlimited that way. A cache has none (put the bandwidth on its edge).
    pub fn bandwidth(&self) -> (Option<u64>, Option<u64>) {
        let own = self.own_bandwidth();
        match (own, self.instance()) {
            // an instance fills the side the node leaves out, as it does for `cpu`
            ((None, None), Some(i)) if !matches!(self, NodeConfig::Cache(_)) => {
                // a type whose source gives no network figure leaves the card unlimited, as absent does
                let b = i.net_bytes_per_s();
                (b, b)
            }
            _ => own,
        }
    }

    /// **Private.** The `bandwidth:` the node itself sets.
    fn own_bandwidth(&self) -> (Option<u64>, Option<u64>) {
        match self {
            NodeConfig::Gateway(n) => n.bandwidth,
            NodeConfig::Service(n) => n.bandwidth,
            NodeConfig::Monolith(n) => n.bandwidth,
            NodeConfig::Database(n) => n.bandwidth,
            NodeConfig::ExternalApi(n) => n.bandwidth,
            NodeConfig::Auth(n) => n.bandwidth,
            NodeConfig::LoadBalancer(n) => n.bandwidth,
            NodeConfig::Storage(n) => n.bandwidth,
            NodeConfig::Cache(_) => None,
        }
        .map_or((None, None), NodeBandwidth::pair)
    }

    /// The cloud instance the node names (`instance:`), when the table has it. An unknown name is
    /// a load error ([`Self::instance_name`] is what was written), so this is `None` only for a
    /// node with no `instance:`.
    pub fn instance(&self) -> Option<&'static crate::instances::Instance> {
        self.instance_name().and_then(crate::instances::lookup)
    }

    /// The `instance:` as written; only the kinds with `cpu:` have the field.
    pub fn instance_name(&self) -> Option<&str> {
        match self {
            NodeConfig::Gateway(n) => n.instance.as_deref(),
            NodeConfig::Service(n) => n.instance.as_deref(),
            NodeConfig::Monolith(n) => n.instance.as_deref(),
            NodeConfig::Database(n) => n.instance.as_deref(),
            NodeConfig::Auth(n) => n.instance.as_deref(),
            NodeConfig::LoadBalancer(n) => n.instance.as_deref(),
            NodeConfig::Cache(n) => n.instance.as_deref(),
            NodeConfig::ExternalApi(_) | NodeConfig::Storage(_) => None,
        }
    }

    /// The `hosting:` as written; only the kinds with a machine to choose have the field.
    pub fn hosting_written(&self) -> Option<crate::config::Hosting> {
        match self {
            NodeConfig::Gateway(n) => n.hosting,
            NodeConfig::Service(n) => n.hosting,
            NodeConfig::Monolith(n) => n.hosting,
            NodeConfig::Database(n) => n.hosting,
            NodeConfig::Auth(n) => n.hosting,
            NodeConfig::LoadBalancer(n) => n.hosting,
            NodeConfig::Cache(n) => n.hosting,
            NodeConfig::ExternalApi(_) | NodeConfig::Storage(_) => None,
        }
    }

    /// Where the node runs: the `hosting:` it writes, else `self`.
    pub fn hosting(&self) -> crate::config::Hosting {
        self.hosting_written().unwrap_or_default()
    }

    /// The database's `engine:` as written; `None` for every other kind.
    pub fn engine_written(&self) -> Option<crate::config::DbEngine> {
        match self {
            NodeConfig::Database(n) => n.engine,
            _ => None,
        }
    }

    /// The gateway's `api:` as written; `None` for every other kind.
    pub fn gateway_api_written(&self) -> Option<crate::config::GatewayApi> {
        match self {
            NodeConfig::Gateway(n) => n.api,
            _ => None,
        }
    }

    /// The database's `multi_az:` as written; `None` for every other kind.
    pub fn multi_az_written(&self) -> Option<bool> {
        match self {
            NodeConfig::Database(n) => n.multi_az,
            _ => None,
        }
    }

    /// The database's `flavor:`, when it names one.
    pub fn db_flavor(&self) -> Option<DbFlavor> {
        match self {
            NodeConfig::Database(n) => n.flavor,
            _ => None,
        }
    }

    /// The engine of a managed database: the `engine:` it writes, else the first engine that runs
    /// its `flavor:`, else PostgreSQL.
    pub fn engine(&self) -> crate::config::DbEngine {
        self.engine_written()
            .or_else(|| self.db_flavor().and_then(|f| f.engines().first().copied()))
            .unwrap_or_default()
    }

    /// The node's `memory:` as written (the `limit` may still be missing: see
    /// [`Self::memory`]).
    pub fn memory_config(&self) -> Option<crate::config::MemoryConfig> {
        match self {
            NodeConfig::Gateway(n) => n.memory,
            NodeConfig::Service(n) => n.memory,
            NodeConfig::Monolith(n) => n.memory,
            NodeConfig::Database(n) => n.memory,
            NodeConfig::Auth(n) => n.memory,
            NodeConfig::LoadBalancer(n) => n.memory,
            NodeConfig::Cache(_) | NodeConfig::ExternalApi(_) | NodeConfig::Storage(_) => None,
        }
    }

    /// The node's memory limit settled and checked (`limit` from the `instance:` when left out);
    /// `None` without a `memory:`. Valid once the config has loaded (the load checks it).
    pub fn memory(&self) -> Option<crate::config::Memory> {
        self.memory_config().and_then(|m| {
            m.resolve(
                self.instance()
                    .map(crate::instances::Instance::memory_bytes),
            )
            .ok()
        })
    }

    /// How a load balancer picks one of its callee components (`spread:`); `None` for every other
    /// kind and for a balancer that sets none (round-robin then).
    pub fn spread(&self) -> Option<&Balance> {
        match self {
            NodeConfig::LoadBalancer(n) => n.spread.as_ref(),
            _ => None,
        }
    }

    /// How long a load balancer keeps sending to a downed copy before it notices (`detect:`); zero for
    /// every other kind and for a balancer that sets none.
    pub fn detect(&self) -> Duration {
        match self {
            NodeConfig::LoadBalancer(n) => n.detect.unwrap_or_default(),
            _ => Duration::ZERO,
        }
    }

    /// What a slot of this node covers (`hold:`); only a node that calls others has the field.
    pub fn hold(&self) -> Hold {
        match self {
            NodeConfig::Gateway(n) => n.hold,
            NodeConfig::Service(n) => n.hold,
            NodeConfig::Monolith(n) => n.hold,
            NodeConfig::Auth(n) => n.hold,
            NodeConfig::LoadBalancer(n) => n.hold,
            NodeConfig::Database(_)
            | NodeConfig::Cache(_)
            | NodeConfig::ExternalApi(_)
            | NodeConfig::Storage(_) => None,
        }
        .unwrap_or_default()
    }

    /// The per-KB payload cost (`per_kb:`), if the node sets one.
    pub fn per_kb(&self) -> Option<&DistConfig> {
        match self {
            NodeConfig::Gateway(n) => n.per_kb.as_ref(),
            NodeConfig::Service(n) => n.per_kb.as_ref(),
            NodeConfig::Monolith(n) => n.per_kb.as_ref(),
            NodeConfig::Database(n) => n.per_kb.as_ref(),
            NodeConfig::Cache(n) => n.per_kb.as_ref(),
            NodeConfig::ExternalApi(n) => n.per_kb.as_ref(),
            NodeConfig::Auth(n) => n.per_kb.as_ref(),
            NodeConfig::LoadBalancer(n) => n.per_kb.as_ref(),
            // a storage node's size cost comes from `throughput` ([`StorageConfig::per_kb`])
            NodeConfig::Storage(_) => None,
        }
    }

    /// The node's own `locale:` tag, if set.
    pub fn locale(&self) -> Option<&str> {
        match self {
            NodeConfig::Gateway(n) => n.locale.as_deref(),
            NodeConfig::Service(n) => n.locale.as_deref(),
            NodeConfig::Monolith(n) => n.locale.as_deref(),
            NodeConfig::Database(n) => n.locale.as_deref(),
            NodeConfig::Cache(n) => n.locale.as_deref(),
            NodeConfig::ExternalApi(n) => n.locale.as_deref(),
            NodeConfig::Auth(n) => n.locale.as_deref(),
            NodeConfig::LoadBalancer(n) => n.locale.as_deref(),
            NodeConfig::Storage(n) => n.locale.as_deref(),
        }
    }

    /// The `flavor` label: free text on a gateway, a preset name on databases,
    /// monoliths and caches.
    pub fn flavor(&self) -> Option<&str> {
        match self {
            NodeConfig::Gateway(n) => n.flavor.as_deref(),
            NodeConfig::Monolith(n) => n.flavor.map(DbFlavor::as_str),
            NodeConfig::Database(n) => n.flavor.map(DbFlavor::as_str),
            NodeConfig::Cache(n) => n.flavor.map(CacheFlavor::as_str),
            NodeConfig::Storage(n) => n.flavor.map(StorageFlavor::as_str),
            _ => None,
        }
    }
}

/// Read/write/scan costs. Without a `flavor`, all three are required.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OpsConfig {
    /// Cost of reading one row (`GET /users/{id}`). Absent = the node's `flavor` preset; with no flavor either, the file is refused.
    pub read: Option<DistConfig>,
    /// Cost of an insert, update or delete. Absent = the node's `flavor` preset; with no flavor either, the file is refused.
    pub write: Option<DistConfig>,
    /// Cost of a list or search (`GET /users`). Absent = the node's `flavor` preset; with no flavor either, the file is refused.
    pub scan: Option<DistConfig>,
}

/// Database flavors with preset op costs (estimates, see [`crate::presets`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum DbFlavor {
    /// `PostgreSQL`.
    Postgres,
    /// `MySQL` (`InnoDB`).
    Mysql,
    /// `MongoDB`.
    Mongodb,
    /// `MariaDB` (`InnoDB`): the same costs as `MySQL`.
    Mariadb,
    /// `Microsoft SQL Server` (and Azure SQL Database): placeholder costs, no published figure.
    Sqlserver,
    /// `Oracle Database`: placeholder costs, no published figure.
    Oracle,
    /// `Neo4j`, a graph database: a read is an indexed node lookup with a hop, a scan is a
    /// traversal.
    Neo4j,
    /// `Apache Cassandra`: cheap writes (commit log and memtable), reads that may touch several
    /// `SSTables`, no efficient scan across partitions.
    Cassandra,
    /// `Amazon DynamoDB`: AWS states single-digit-millisecond latency at any scale.
    Dynamodb,
}

impl DbFlavor {
    /// The YAML spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            DbFlavor::Postgres => "postgres",
            DbFlavor::Mysql => "mysql",
            DbFlavor::Mongodb => "mongodb",
            DbFlavor::Mariadb => "mariadb",
            DbFlavor::Sqlserver => "sqlserver",
            DbFlavor::Oracle => "oracle",
            DbFlavor::Neo4j => "neo4j",
            DbFlavor::Cassandra => "cassandra",
            DbFlavor::Dynamodb => "dynamodb",
        }
    }
}

/// Cache flavors with a preset hit ratio and costs (estimates, see [`crate::presets`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum CacheFlavor {
    /// Redis.
    Redis,
    /// Memcached.
    Memcached,
    /// Valkey, the open fork of Redis. Behaves as Redis here (the same preset: no published
    /// figure separates them) and is priced as its own engine on a managed cache (20% below Redis OSS).
    Valkey,
}

impl CacheFlavor {
    /// The YAML spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            CacheFlavor::Redis => "redis",
            CacheFlavor::Memcached => "memcached",
            CacheFlavor::Valkey => "valkey",
        }
    }
}
