//! Parsing of `kaanal.yaml` into typed config.
//!
//! `nodes:` is a map of `type`-tagged nodes (rule R1.15). Each kind is its own struct
//! with `deny_unknown_fields`, so a field on the wrong kind fails at load. This module
//! parses and checks single values (ranges, `p50 ≤ p99`, URL shape); graph rules
//! (optional gateway, edge table, DAG, ops that presets must complete) live in
//! [`crate::topology`], route/spec wiring in the server crate. The editor JSON Schema is
//! derived from these types ([`crate::schema`]).

use std::fmt;
use std::num::NonZeroU32;
use std::time::Duration;

use indexmap::IndexMap;
use schemars::JsonSchema;
use serde::de::Error as _;
use serde::{Deserialize, Deserializer};

/// The parsed `kaanal.yaml` file.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Config {
    /// Config schema version. Must be `1`.
    pub version: u32,
    /// Master RNG seed: same seed → same data, timings and failure patterns.
    #[serde(default)]
    pub seed: u64,
    /// Port for the HTTP server (mock + control API share it). Defaults to 8080.
    #[serde(default = "default_port")]
    pub port: u16,
    /// Nodes by name, in file order (deterministic error messages).
    #[serde(deserialize_with = "unique_nodes")]
    pub nodes: IndexMap<String, NodeConfig>,
    /// Dependency edges, caller → dependency.
    #[serde(default)]
    pub edges: Vec<EdgeConfig>,
    /// Static route overrides, keyed by `"METHOD /path"`.
    #[serde(default)]
    pub routes: IndexMap<String, RouteOverride>,
}

/// **Private.** serde default for [`Config::port`].
fn default_port() -> u16 {
    8080
}

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

    /// `OpenAPI` spec file, relative to the config's directory (service, monolith).
    pub fn spec(&self) -> Option<&str> {
        match self {
            NodeConfig::Service(n) => n.spec.as_deref(),
            NodeConfig::Monolith(n) => n.spec.as_deref(),
            _ => None,
        }
    }

    /// Most own hops that may run at once; `None` = unlimited.
    pub fn concurrency(&self) -> Option<NonZeroU32> {
        match self {
            NodeConfig::Gateway(n) => n.concurrency,
            NodeConfig::Service(n) => n.concurrency,
            NodeConfig::Monolith(n) => n.concurrency,
            NodeConfig::Database(n) => n.concurrency,
            NodeConfig::Cache(n) => n.concurrency,
            NodeConfig::ExternalApi(n) => n.concurrency,
            NodeConfig::Auth(n) => n.concurrency,
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
    /// Most requests this node serves at once; absent = unlimited. The simulator honours
    /// it; live mode warns and ignores it for now.
    pub concurrency: Option<NonZeroU32>,
}

/// A backend service. Without a gateway, requests for the routes in its `spec` enter here.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ServiceConfig {
    /// `OpenAPI` 3.0 spec file, relative to this config file.
    pub spec: Option<String>,
    /// Processing time per request.
    pub proc: Option<DistConfig>,
    /// Most requests this node serves at once; absent = unlimited. The simulator honours
    /// it; live mode warns and ignores it for now.
    pub concurrency: Option<NonZeroU32>,
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
    /// Inline read/write/scan costs; any op left out comes from `flavor`. Used once
    /// endpoint chains land.
    pub ops: Option<OpsConfig>,
    /// Preset that fills inline ops the user left out (estimates).
    pub flavor: Option<DbFlavor>,
    /// Most requests this node serves at once; absent = unlimited. The simulator honours
    /// it; live mode warns and ignores it for now.
    pub concurrency: Option<NonZeroU32>,
}

/// A data store with read/write/scan costs.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DatabaseConfig {
    /// Read/write/scan costs. Required in full unless `flavor` fills the gaps.
    pub ops: Option<OpsConfig>,
    /// Preset that fills ops the user left out (estimates).
    pub flavor: Option<DbFlavor>,
    /// Connection pool. Accepted and validated; no effect yet (capacity limits come later).
    pub pool: Option<PoolConfig>,
    /// Most requests this node serves at once; absent = unlimited. The simulator honours
    /// it; live mode warns and ignores it for now.
    pub concurrency: Option<NonZeroU32>,
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
    /// Most requests this node serves at once; absent = unlimited. The simulator honours
    /// it; live mode warns and ignores it for now.
    pub concurrency: Option<NonZeroU32>,
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
    /// Token-bucket limit. Accepted and validated; no effect yet.
    pub rate_limit: Option<RateLimitConfig>,
    /// Real upstream URL to forward to. Accepted and validated; no effect yet.
    #[serde(default, deserialize_with = "http_url")]
    #[schemars(extend("pattern" = "^https?://.+"))]
    pub proxy: Option<String>,
    /// Connection pool. Accepted and validated; no effect yet.
    pub pool: Option<PoolConfig>,
    /// Most requests this node serves at once; absent = unlimited. The simulator honours
    /// it; live mode warns and ignores it for now.
    pub concurrency: Option<NonZeroU32>,
}

/// An identity provider. Its login endpoint and token keys come in a later release.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AuthConfig {
    /// Cost of one login or token check.
    pub proc: Option<DistConfig>,
    /// Accounts that can log in; usernames are unique. Accepted and validated; no effect yet.
    #[serde(default, deserialize_with = "unique_users")]
    pub users: Vec<UserConfig>,
    /// Lifetime of issued tokens, at most 1 day (default 1h). Accepted and validated;
    /// no effect yet.
    #[serde(default, deserialize_with = "token_ttl")]
    #[schemars(schema_with = "crate::schema::duration_text")]
    pub token_ttl: Option<Duration>,
    /// Most requests this node serves at once; absent = unlimited. The simulator honours
    /// it; live mode warns and ignores it for now.
    pub concurrency: Option<NonZeroU32>,
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

/// A connection pool (`database`, `external-api`).
#[derive(Debug, Clone, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PoolConfig {
    /// Connections in the pool, at least 1.
    pub size: NonZeroU32,
    /// Requests that may wait for a free connection (default 0).
    #[serde(default)]
    pub queue: u32,
    /// Status returned when the queue is full, 500–504 (default 503).
    #[serde(default = "default_overflow", deserialize_with = "status_5xx")]
    #[schemars(extend("minimum" = 500, "maximum" = 504))]
    pub overflow_status: u16,
}

/// A token bucket: refills `rps` per second, holds at most `burst`.
#[derive(Debug, Clone, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RateLimitConfig {
    /// Requests per second, above 0.
    #[serde(deserialize_with = "positive_rps")]
    #[schemars(extend("exclusiveMinimum" = 0))]
    pub rps: f64,
    /// Largest burst, at least 1.
    pub burst: NonZeroU32,
}

/// One account on an `auth` node (synthetic accounts only).
#[derive(Debug, Clone, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UserConfig {
    /// Login name, unique per node.
    pub username: String,
    /// Plain-text password.
    pub password: String,
    /// Roles put into issued tokens.
    #[serde(default)]
    pub roles: Vec<String>,
}

/// One entry in `edges:` (caller → dependency).
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EdgeConfig {
    /// Name of the calling node.
    pub from: String,
    /// Name of the node it calls.
    pub to: String,
    /// Network latency on this edge; absent means zero.
    #[serde(default)]
    pub net: Option<DistConfig>,
    /// Calls per request along this edge (call ratio), at least 1. Used once endpoint
    /// chains land.
    #[serde(default = "default_calls")]
    pub calls: NonZeroU32,
}

/// A duration in YAML: `5ms` (fixed), `{ p50: 4ms, p99: 20ms }` (lognormal fitted to both) or `{ mean: 5ms }` (exponential).
#[derive(Debug, Clone, PartialEq)]
pub enum DistConfig {
    /// A single fixed duration, parsed with `humantime`.
    Fixed(Duration),
    /// A percentile pair; fitted to a lognormal by [`crate::Dist::from_p50_p99`].
    Pct {
        /// 50th percentile (median).
        p50: Duration,
        /// 99th percentile; never below `p50`.
        p99: Duration,
    },
    /// Exponential with this mean (`{ mean: 5ms }`); see [`crate::Dist::Exp`].
    Exp {
        /// Mean duration.
        mean: Duration,
    },
}

impl<'de> Deserialize<'de> for DistConfig {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        // A plain untagged enum reports "data did not match any variant"; say what is expected instead.
        /// **Private.** `{ p50, p99 }`. Unknown keys rejected, so a mixed map can't half-match.
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct PctRaw {
            #[serde(with = "humantime_serde")]
            p50: Duration,
            #[serde(with = "humantime_serde")]
            p99: Duration,
        }
        /// **Private.** `{ mean }`. Unknown keys rejected, so `{ mean, p99 }` is an error.
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct ExpRaw {
            #[serde(with = "humantime_serde")]
            mean: Duration,
        }
        /// **Private.** Parsing shim for the three YAML shapes of a duration.
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Fixed(#[serde(with = "humantime_serde")] Duration),
            Pct(PctRaw),
            Exp(ExpRaw),
        }
        let c = Raw::deserialize(d)
            .map(|r| match r {
                Raw::Fixed(x) => DistConfig::Fixed(x),
                Raw::Pct(PctRaw { p50, p99 }) => DistConfig::Pct { p50, p99 },
                Raw::Exp(ExpRaw { mean }) => DistConfig::Exp { mean },
            })
            .map_err(|_| {
                D::Error::custom(
                    "expected a duration like `5ms`, `{ p50: 1ms, p99: 3ms }` or `{ mean: 5ms }`",
                )
            })?;
        // Closes the M0 review question: `{ p50: 100h, p99: 1ms }` used to slip past the cap.
        if let DistConfig::Pct { p50, p99 } = &c
            && p50 > p99
        {
            return Err(D::Error::custom(format!(
                "p50 ({p50:?}) is above p99 ({p99:?}); p50 must be at most p99"
            )));
        }
        let longest = match &c {
            DistConfig::Fixed(x) => *x,
            DistConfig::Pct { p99, .. } => *p99,
            DistConfig::Exp { mean } => *mean,
        };
        if longest > MAX_DURATION {
            return Err(D::Error::custom(
                "durations must be at most 1h (a request's hops are summed, so huge values overflow)",
            ));
        }
        Ok(c)
    }
}

/// **Private.** Longest configurable cost. Samples are capped at 10 × p99, so one hop is
/// at most 10 h and a chain's sum stays far from `Duration`'s limit.
const MAX_DURATION: Duration = Duration::from_secs(3600);

/// **Private.** Longest `token_ttl`.
const MAX_TOKEN_TTL: Duration = Duration::from_hours(24);

/// A static response override for a `"METHOD /path"` key in `routes:`.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RouteOverride {
    /// HTTP status to return. Defaults to 200.
    #[serde(default = "default_status")]
    pub status: u16,
    /// Response body, returned verbatim.
    #[serde(default)]
    pub body: serde_json::Value,
}

/// **Private.** serde default for [`RouteOverride::status`].
fn default_status() -> u16 {
    200
}

/// **Private.** serde default for [`PoolConfig::overflow_status`].
fn default_overflow() -> u16 {
    503
}

/// **Private.** serde default for [`EdgeConfig::calls`].
fn default_calls() -> NonZeroU32 {
    NonZeroU32::MIN
}

/// **Private.** `hit_ratio` and `error_rate`: a probability in 0..=1.
fn ratio<'de, D: Deserializer<'de>>(d: D) -> Result<Option<f64>, D::Error> {
    let v = f64::deserialize(d)?;
    if (0.0..=1.0).contains(&v) {
        Ok(Some(v))
    } else {
        Err(D::Error::custom(format!(
            "must be between 0 and 1 (got {v})"
        )))
    }
}

/// **Private.** `rate_limit.rps`: finite and above 0.
fn positive_rps<'de, D: Deserializer<'de>>(d: D) -> Result<f64, D::Error> {
    let v = f64::deserialize(d)?;
    if v > 0.0 && v.is_finite() {
        Ok(v)
    } else {
        Err(D::Error::custom(format!("rps must be above 0 (got {v})")))
    }
}

/// **Private.** `pool.overflow_status`: a 5xx Kaanal can answer with.
fn status_5xx<'de, D: Deserializer<'de>>(d: D) -> Result<u16, D::Error> {
    let s = u16::deserialize(d)?;
    if (500..=504).contains(&s) {
        Ok(s)
    } else {
        Err(D::Error::custom(format!(
            "overflow_status must be between 500 and 504 (got {s})"
        )))
    }
}

/// **Private.** `proxy`: `http://` or `https://` followed by something.
fn http_url<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    let s = String::deserialize(d)?;
    let rest = s
        .strip_prefix("https://")
        .or_else(|| s.strip_prefix("http://"));
    if rest.is_some_and(|r| !r.is_empty()) {
        Ok(Some(s))
    } else {
        Err(D::Error::custom(format!(
            "proxy must be an http(s) URL like https://api.example.com (got '{s}')"
        )))
    }
}

/// **Private.** `token_ttl`: a duration of at most 1 day.
fn token_ttl<'de, D: Deserializer<'de>>(d: D) -> Result<Option<Duration>, D::Error> {
    let v: Duration = humantime_serde::deserialize(d)?;
    if v <= MAX_TOKEN_TTL {
        Ok(Some(v))
    } else {
        Err(D::Error::custom("token_ttl must be at most 1 day"))
    }
}

/// **Private.** `nodes`: a map in file order where each name is defined once (a plain
/// map would silently keep the last definition).
fn unique_nodes<'de, D: Deserializer<'de>>(d: D) -> Result<IndexMap<String, NodeConfig>, D::Error> {
    struct Nodes;
    impl<'de> serde::de::Visitor<'de> for Nodes {
        type Value = IndexMap<String, NodeConfig>;

        fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("a map of node names to nodes")
        }

        fn visit_map<A: serde::de::MapAccess<'de>>(
            self,
            mut m: A,
        ) -> Result<Self::Value, A::Error> {
            let mut out = IndexMap::new();
            while let Some((name, node)) = m.next_entry::<String, NodeConfig>()? {
                if out.contains_key(&name) {
                    return Err(A::Error::custom(format!("duplicate node '{name}'")));
                }
                out.insert(name, node);
            }
            Ok(out)
        }
    }
    d.deserialize_map(Nodes)
}

/// **Private.** `users`: no two accounts share a username.
fn unique_users<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<UserConfig>, D::Error> {
    let users = Vec::<UserConfig>::deserialize(d)?;
    for (i, u) in users.iter().enumerate() {
        if users[..i].iter().any(|o| o.username == u.username) {
            return Err(D::Error::custom(format!(
                "duplicate username '{}'",
                u.username
            )));
        }
    }
    Ok(users)
}

/// A config parse error with an optional YAML position.
///
/// `Display` renders `line:col: msg` when a position is known, so the caller can
/// print `file:line:col: msg` (rule R6.1).
#[derive(Debug, Clone, PartialEq)]
pub struct ConfigError {
    /// 1-based line, when the parser reported one.
    pub line: Option<usize>,
    /// 1-based column, when the parser reported one.
    pub col: Option<usize>,
    /// Human-readable description of what is wrong.
    pub msg: String,
}

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (self.line, self.col) {
            (Some(l), Some(c)) => write!(f, "{l}:{c}: {}", self.msg),
            _ => write!(f, "{}", self.msg),
        }
    }
}

impl std::error::Error for ConfigError {}

impl Config {
    /// Parses `kaanal.yaml` source text.
    ///
    /// A UTF-8 BOM is stripped first (Windows editors add one; YAML rejects it —
    /// rule R6.4), and `version` must be `1`.
    ///
    /// # Errors
    /// Returns [`ConfigError`] (with line/column when the parser knows them) for
    /// malformed YAML, unknown fields, bad values, or an unsupported `version`.
    /// When parsing fails inside `nodes:`, the error is re-placed at the real node and
    /// field (see [`crate::diagnostics`]).
    pub fn from_yaml(src: &str) -> Result<Config, ConfigError> {
        // Windows editors may save a UTF-8 BOM; YAML parsers reject it.
        let src = src.strip_prefix('\u{feff}').unwrap_or(src);
        let cfg: Config = serde_yaml_ng::from_str(src).map_err(|e| {
            let loc = e.location();
            let serde_err = ConfigError {
                line: loc.as_ref().map(serde_yaml_ng::Location::line),
                col: loc.as_ref().map(serde_yaml_ng::Location::column),
                msg: e.to_string(),
            };
            // R1.15: serde loses positions inside tagged nodes; recover them (error path only).
            crate::diagnostics::explain(src, serde_err)
        })?;
        if cfg.version != 1 {
            return Err(crate::diagnostics::Located::new(
                crate::diagnostics::keys(&["version"]),
                format!("unsupported config version {} (expected 1)", cfg.version),
            )
            .in_source(src));
        }
        Ok(cfg)
    }
}
