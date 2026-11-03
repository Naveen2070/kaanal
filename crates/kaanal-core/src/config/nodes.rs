//! The node kinds and their per-kind config structs (gateway, service, monolith, database, cache, external API, auth) with the flavor enums.

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
}

impl NodeKind {
    /// Every kind, in the order Studio lists them. A new kind goes here too: the draft view's
    /// `kinds` and `kind_defaults` are built from this list.
    pub const ALL: [NodeKind; 7] = [
        NodeKind::Gateway,
        NodeKind::Service,
        NodeKind::Monolith,
        NodeKind::Database,
        NodeKind::Cache,
        NodeKind::ExternalApi,
        NodeKind::Auth,
    ];

    /// Whether the kind has one `proc` service time (the others price their work with `ops`,
    /// `hit` and `miss`). Studio shows a Time control only for these.
    pub fn has_proc(self) -> bool {
        match self {
            NodeKind::Gateway
            | NodeKind::Service
            | NodeKind::Monolith
            | NodeKind::ExternalApi
            | NodeKind::Auth => true,
            NodeKind::Database | NodeKind::Cache => false,
        }
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
        }
    }

    /// Processing time per request (gateway, service, monolith, external-api, auth).
    pub fn proc(&self) -> Option<&DistConfig> {
        match self {
            NodeConfig::Gateway(n) => n.proc.as_ref(),
            NodeConfig::Service(n) => n.proc.as_ref(),
            NodeConfig::Monolith(n) => n.proc.as_ref(),
            NodeConfig::ExternalApi(n) => n.proc.as_ref(),
            NodeConfig::Auth(n) => n.proc.as_ref(),
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
        }
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
            _ => None,
        }
    }
}

/// A reverse proxy / API gateway / load balancer (nginx, Envoy). Optional, at most one.
/// With one, every request pays its `proc` first and it answers static routes.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GatewayConfig {
    /// Cosmetic label shown in status output (`nginx`, `envoy`); not a preset.
    pub flavor: Option<String>,
    /// Processing time per request.
    pub proc: Option<DistConfig>,
    /// Most requests this node serves at once, as a number (the rest wait without limit) or a
    /// map `{ limit, queue, overflow_status }` (the rest wait up to `queue`, then get
    /// `overflow_status`). Absent = unlimited.
    #[serde(default)]
    #[schemars(schema_with = "crate::schema::concurrency")]
    pub concurrency: Option<ConcurrencyConfig>,
    /// Token-bucket limit on how fast requests may reach this node: `rps` refilled per second,
    /// at most `burst` at once; beyond it the request fails with 429.
    pub rate_limit: Option<RateLimitConfig>,
    /// Deadline for a call to this node: queue wait plus its own work plus everything it calls
    /// downstream. When it passes, the caller gets a 504 (`timeout`) while the work already
    /// running keeps its slot until it ends. `0 < timeout <= 5 min`; absent = no deadline.
    #[serde(default, deserialize_with = "call_timeout")]
    #[schemars(schema_with = "crate::schema::duration_text")]
    pub timeout: Option<Duration>,
    /// Copies of this node, 1 to 64 (default 1). Each has its own slots, queue, pool and rate
    /// limit; `balance` picks one per request.
    #[serde(default)]
    #[schemars(extend("minimum" = 1, "maximum" = 64))]
    pub replicas: Option<u16>,
    /// How traffic entering this node picks a replica (default `round-robin`); a calling edge
    /// may override it.
    #[serde(default)]
    pub balance: Option<Balance>,
    /// Extra processing time per KB of payload (request and response each, once per body), a
    /// duration like `proc`. Absent = none.
    pub per_kb: Option<DistConfig>,
    /// Locale pack for the data this node's routes generate, e.g. `en-US`; beats the top-level
    /// `locale`.
    #[serde(default)]
    #[schemars(extend("pattern" = LOCALE_PATTERN))]
    pub locale: Option<String>,
}

/// A backend service. Without a gateway, requests for the routes in its `spec` enter here.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ServiceConfig {
    /// `OpenAPI` 3.0 spec file, relative to this config file.
    pub spec: Option<String>,
    /// Processing time per request.
    pub proc: Option<DistConfig>,
    /// Named chains (cron jobs, queue consumers) a step reaches with `with: <name>`.
    #[serde(default)]
    pub chains: IndexMap<String, Vec<StepConfig>>,
    /// Most requests this node serves at once, as a number (the rest wait without limit) or a
    /// map `{ limit, queue, overflow_status }` (the rest wait up to `queue`, then get
    /// `overflow_status`). Absent = unlimited.
    #[serde(default)]
    #[schemars(schema_with = "crate::schema::concurrency")]
    pub concurrency: Option<ConcurrencyConfig>,
    /// Token-bucket limit on how fast requests may reach this node: `rps` refilled per second,
    /// at most `burst` at once; beyond it the request fails with 429.
    pub rate_limit: Option<RateLimitConfig>,
    /// Deadline for a call to this node: queue wait plus its own work plus everything it calls
    /// downstream. When it passes, the caller gets a 504 (`timeout`) while the work already
    /// running keeps its slot until it ends. `0 < timeout <= 5 min`; absent = no deadline.
    #[serde(default, deserialize_with = "call_timeout")]
    #[schemars(schema_with = "crate::schema::duration_text")]
    pub timeout: Option<Duration>,
    /// Copies of this node, 1 to 64 (default 1). Each has its own slots, queue, pool and rate
    /// limit; `balance` picks one per request.
    #[serde(default)]
    #[schemars(extend("minimum" = 1, "maximum" = 64))]
    pub replicas: Option<u16>,
    /// How traffic entering this node picks a replica (default `round-robin`); a calling edge
    /// may override it.
    #[serde(default)]
    pub balance: Option<Balance>,
    /// Extra processing time per KB of payload (request and response each, once per body), a
    /// duration like `proc`. Absent = none.
    pub per_kb: Option<DistConfig>,
    /// Locale pack for the data this node's routes generate, e.g. `en-US`; beats the top-level
    /// `locale`.
    #[serde(default)]
    #[schemars(extend("pattern" = LOCALE_PATTERN))]
    pub locale: Option<String>,
}

/// A service that can also be its own database: with `ops` (or a `flavor`) it has inline
/// read/write/scan costs; with edges to databases it behaves as a service. Both together
/// are allowed.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MonolithConfig {
    /// `OpenAPI` 3.0 spec file, relative to this config file.
    pub spec: Option<String>,
    /// Processing time per request.
    pub proc: Option<DistConfig>,
    /// Named chains (cron jobs, queue consumers) a step reaches with `with: <name>`.
    #[serde(default)]
    pub chains: IndexMap<String, Vec<StepConfig>>,
    /// Inline read/write/scan costs; any op left out comes from `flavor`. Used once
    /// endpoint chains land.
    pub ops: Option<OpsConfig>,
    /// Preset that fills inline ops the user left out (estimates).
    pub flavor: Option<DbFlavor>,
    /// Most requests this node serves at once, as a number (the rest wait without limit) or a
    /// map `{ limit, queue, overflow_status }` (the rest wait up to `queue`, then get
    /// `overflow_status`). Absent = unlimited.
    #[serde(default)]
    #[schemars(schema_with = "crate::schema::concurrency")]
    pub concurrency: Option<ConcurrencyConfig>,
    /// Token-bucket limit on how fast requests may reach this node: `rps` refilled per second,
    /// at most `burst` at once; beyond it the request fails with 429.
    pub rate_limit: Option<RateLimitConfig>,
    /// Deadline for a call to this node: queue wait plus its own work plus everything it calls
    /// downstream. When it passes, the caller gets a 504 (`timeout`) while the work already
    /// running keeps its slot until it ends. `0 < timeout <= 5 min`; absent = no deadline.
    #[serde(default, deserialize_with = "call_timeout")]
    #[schemars(schema_with = "crate::schema::duration_text")]
    pub timeout: Option<Duration>,
    /// Copies of this node, 1 to 64 (default 1). Each has its own slots, queue, pool and rate
    /// limit; `balance` picks one per request.
    #[serde(default)]
    #[schemars(extend("minimum" = 1, "maximum" = 64))]
    pub replicas: Option<u16>,
    /// How traffic entering this node picks a replica (default `round-robin`); a calling edge
    /// may override it.
    #[serde(default)]
    pub balance: Option<Balance>,
    /// Extra processing time per KB of payload (request and response each, once per body), a
    /// duration like `proc`. Absent = none.
    pub per_kb: Option<DistConfig>,
    /// Locale pack for the data this node's routes generate, e.g. `en-US`; beats the top-level
    /// `locale`.
    #[serde(default)]
    #[schemars(extend("pattern" = LOCALE_PATTERN))]
    pub locale: Option<String>,
}

/// A data store with read/write/scan costs.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DatabaseConfig {
    /// Read/write/scan costs. Required in full unless `flavor` fills the gaps.
    pub ops: Option<OpsConfig>,
    /// Preset that fills ops the user left out (estimates).
    pub flavor: Option<DbFlavor>,
    /// Connection pool: the slot limit `size`, a wait queue of `queue` and the status
    /// `overflow_status` when it is full; the same as the map form of `concurrency`; set one
    /// of the two.
    pub pool: Option<PoolConfig>,
    /// Most requests this node serves at once, as a number (the rest wait without limit) or a
    /// map `{ limit, queue, overflow_status }` (the rest wait up to `queue`, then get
    /// `overflow_status`). Absent = unlimited.
    #[serde(default)]
    #[schemars(schema_with = "crate::schema::concurrency")]
    pub concurrency: Option<ConcurrencyConfig>,
    /// Token-bucket limit on how fast requests may reach this node: `rps` refilled per second,
    /// at most `burst` at once; beyond it the request fails with 429.
    pub rate_limit: Option<RateLimitConfig>,
    /// Deadline for a call to this node: queue wait plus its own work plus everything it calls
    /// downstream. When it passes, the caller gets a 504 (`timeout`) while the work already
    /// running keeps its slot until it ends. `0 < timeout <= 5 min`; absent = no deadline.
    #[serde(default, deserialize_with = "call_timeout")]
    #[schemars(schema_with = "crate::schema::duration_text")]
    pub timeout: Option<Duration>,
    /// Copies of this node, 1 to 64 (default 1). Each has its own slots, queue, pool and rate
    /// limit; `balance` picks one per request.
    #[serde(default)]
    #[schemars(extend("minimum" = 1, "maximum" = 64))]
    pub replicas: Option<u16>,
    /// How traffic entering this node picks a replica (default `round-robin`); a calling edge
    /// may override it.
    #[serde(default)]
    pub balance: Option<Balance>,
    /// Extra processing time per KB of payload (request and response each, once per body), a
    /// duration like `proc`. Absent = none.
    pub per_kb: Option<DistConfig>,
    /// Locale pack for the data this node's routes generate, e.g. `en-US`; beats the top-level
    /// `locale`.
    #[serde(default)]
    #[schemars(extend("pattern" = LOCALE_PATTERN))]
    pub locale: Option<String>,
}

/// A cache in front of slower nodes. Its costs are used once endpoint chains land.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CacheConfig {
    /// Preset for `hit_ratio`, `hit` and `miss` (estimates).
    pub flavor: Option<CacheFlavor>,
    /// Share of lookups that hit, 0 to 1.
    #[serde(default, deserialize_with = "ratio")]
    #[schemars(extend("minimum" = 0, "maximum" = 1))]
    pub hit_ratio: Option<f64>,
    /// Cost of a hit.
    pub hit: Option<DistConfig>,
    /// Cost of finding out it is a miss.
    pub miss: Option<DistConfig>,
    /// Entry lifetime. Accepted and validated; no effect yet.
    #[serde(default, with = "humantime_serde")]
    #[schemars(schema_with = "crate::schema::duration_text")]
    pub ttl: Option<Duration>,
    /// Most requests this node serves at once, as a number (the rest wait without limit) or a
    /// map `{ limit, queue, overflow_status }` (the rest wait up to `queue`, then get
    /// `overflow_status`). Absent = unlimited.
    #[serde(default)]
    #[schemars(schema_with = "crate::schema::concurrency")]
    pub concurrency: Option<ConcurrencyConfig>,
    /// Token-bucket limit on how fast requests may reach this node: `rps` refilled per second,
    /// at most `burst` at once; beyond it the request fails with 429.
    pub rate_limit: Option<RateLimitConfig>,
    /// Deadline for a call to this node: queue wait plus its own work plus everything it calls
    /// downstream. When it passes, the caller gets a 504 (`timeout`) while the work already
    /// running keeps its slot until it ends. `0 < timeout <= 5 min`; absent = no deadline.
    #[serde(default, deserialize_with = "call_timeout")]
    #[schemars(schema_with = "crate::schema::duration_text")]
    pub timeout: Option<Duration>,
    /// Copies of this node, 1 to 64 (default 1). Each has its own slots, queue, pool and rate
    /// limit; `balance` picks one per request.
    #[serde(default)]
    #[schemars(extend("minimum" = 1, "maximum" = 64))]
    pub replicas: Option<u16>,
    /// How traffic entering this node picks a replica (default `round-robin`); a calling edge
    /// may override it.
    #[serde(default)]
    pub balance: Option<Balance>,
    /// Extra processing time per KB of payload (request and response each, once per body), a
    /// duration like `proc`. Absent = none.
    pub per_kb: Option<DistConfig>,
    /// Locale pack for the data this node's routes generate, e.g. `en-US`; beats the top-level
    /// `locale`.
    #[serde(default)]
    #[schemars(extend("pattern" = LOCALE_PATTERN))]
    pub locale: Option<String>,
}

/// A third-party HTTP API (payments, email). Targets come from config only.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExternalApiConfig {
    /// Response time of the API.
    pub proc: Option<DistConfig>,
    /// Baseline share of failing calls, 0 to 1. Accepted and validated; no effect yet.
    #[serde(default, deserialize_with = "ratio")]
    #[schemars(extend("minimum" = 0, "maximum" = 1))]
    pub error_rate: Option<f64>,
    /// Real upstream URL to forward to. Accepted and validated; no effect yet.
    #[serde(default, deserialize_with = "http_url")]
    #[schemars(extend("pattern" = "^https?://.+"))]
    pub proxy: Option<String>,
    /// Connection pool: the slot limit `size`, a wait queue of `queue` and the status
    /// `overflow_status` when it is full; the same as the map form of `concurrency`; set one
    /// of the two.
    pub pool: Option<PoolConfig>,
    /// Most requests this node serves at once, as a number (the rest wait without limit) or a
    /// map `{ limit, queue, overflow_status }` (the rest wait up to `queue`, then get
    /// `overflow_status`). Absent = unlimited.
    #[serde(default)]
    #[schemars(schema_with = "crate::schema::concurrency")]
    pub concurrency: Option<ConcurrencyConfig>,
    /// Token-bucket limit on how fast requests may reach this node: `rps` refilled per second,
    /// at most `burst` at once; beyond it the request fails with 429.
    pub rate_limit: Option<RateLimitConfig>,
    /// Deadline for a call to this node: queue wait plus its own work plus everything it calls
    /// downstream. When it passes, the caller gets a 504 (`timeout`) while the work already
    /// running keeps its slot until it ends. `0 < timeout <= 5 min`; absent = no deadline.
    #[serde(default, deserialize_with = "call_timeout")]
    #[schemars(schema_with = "crate::schema::duration_text")]
    pub timeout: Option<Duration>,
    /// Copies of this node, 1 to 64 (default 1). Each has its own slots, queue, pool and rate
    /// limit; `balance` picks one per request.
    #[serde(default)]
    #[schemars(extend("minimum" = 1, "maximum" = 64))]
    pub replicas: Option<u16>,
    /// How traffic entering this node picks a replica (default `round-robin`); a calling edge
    /// may override it.
    #[serde(default)]
    pub balance: Option<Balance>,
    /// Extra processing time per KB of payload (request and response each, once per body), a
    /// duration like `proc`. Absent = none.
    pub per_kb: Option<DistConfig>,
    /// Locale pack for the data this node's routes generate, e.g. `en-US`; beats the top-level
    /// `locale`.
    #[serde(default)]
    #[schemars(extend("pattern" = LOCALE_PATTERN))]
    pub locale: Option<String>,
}

/// An identity provider: serves `POST /login` (a signed RS256 token for one of its `users`)
/// and `GET /.well-known/jwks.json` (the public key). At most one per topology.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AuthConfig {
    /// Cost of one login, one JWKS request or one gateway token check.
    pub proc: Option<DistConfig>,
    /// Accounts that can log in (synthetic only); usernames are unique, at most 1000 users.
    #[serde(default, deserialize_with = "unique_users")]
    pub users: Vec<UserConfig>,
    /// Lifetime of issued tokens: whole seconds, 1s to 1 day (default 1h).
    #[serde(default, deserialize_with = "token_ttl")]
    #[schemars(schema_with = "crate::schema::duration_text")]
    pub token_ttl: Option<Duration>,
    /// Extra claims in every token this node issues (string, number or boolean; at most 32).
    /// A user's own `claims` win. Reserved: sub, iss, exp, iat, jti, roles, scope.
    #[serde(default)]
    #[schemars(schema_with = "crate::schema::claims")]
    pub claims: IndexMap<String, serde_json::Value>,
    /// Most requests this node serves at once, as a number (the rest wait without limit) or a
    /// map `{ limit, queue, overflow_status }` (the rest wait up to `queue`, then get
    /// `overflow_status`). Absent = unlimited.
    #[serde(default)]
    #[schemars(schema_with = "crate::schema::concurrency")]
    pub concurrency: Option<ConcurrencyConfig>,
    /// Token-bucket limit on how fast requests may reach this node: `rps` refilled per second,
    /// at most `burst` at once; beyond it the request fails with 429.
    pub rate_limit: Option<RateLimitConfig>,
    /// Deadline for a call to this node: queue wait plus its own work plus everything it calls
    /// downstream. When it passes, the caller gets a 504 (`timeout`) while the work already
    /// running keeps its slot until it ends. `0 < timeout <= 5 min`; absent = no deadline.
    #[serde(default, deserialize_with = "call_timeout")]
    #[schemars(schema_with = "crate::schema::duration_text")]
    pub timeout: Option<Duration>,
    /// Copies of this node, 1 to 64 (default 1). Each has its own slots, queue, pool and rate
    /// limit; `balance` picks one per request.
    #[serde(default)]
    #[schemars(extend("minimum" = 1, "maximum" = 64))]
    pub replicas: Option<u16>,
    /// How traffic entering this node picks a replica (default `round-robin`); a calling edge
    /// may override it.
    #[serde(default)]
    pub balance: Option<Balance>,
    /// Extra processing time per KB of payload (request and response each, once per body), a
    /// duration like `proc`. Absent = none.
    pub per_kb: Option<DistConfig>,
    /// Locale pack for the data this node's routes generate, e.g. `en-US`; beats the top-level
    /// `locale`.
    #[serde(default)]
    #[schemars(extend("pattern" = LOCALE_PATTERN))]
    pub locale: Option<String>,
}

/// Read/write/scan costs. Without a `flavor`, all three are required.
#[derive(Debug, Clone, Default, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OpsConfig {
    /// Cost of reading one row (`GET /users/{id}`).
    pub read: Option<DistConfig>,
    /// Cost of an insert, update or delete.
    pub write: Option<DistConfig>,
    /// Cost of a list or search (`GET /users`).
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
}

impl DbFlavor {
    /// The YAML spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            DbFlavor::Postgres => "postgres",
            DbFlavor::Mysql => "mysql",
            DbFlavor::Mongodb => "mongodb",
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
}

impl CacheFlavor {
    /// The YAML spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            CacheFlavor::Redis => "redis",
            CacheFlavor::Memcached => "memcached",
        }
    }
}
