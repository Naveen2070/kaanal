//! The per-kind config structs of the seven original node kinds (gateway, service, monolith, database, cache, external API, auth); split out of `nodes.rs`, which reached 800 lines.

use super::nodes::{CacheFlavor, DbFlavor, Hold, OpsConfig};
use super::{
    ConcurrencyConfig, CpuConfig, Deserialize, DistConfig, Duration, IndexMap, JsonSchema,
    LOCALE_PATTERN, NodeBandwidth, PoolConfig, RateLimitConfig, StepConfig, UserConfig,
    call_timeout, http_url, ratio, token_ttl, unique_users,
};
use crate::balance::Balance;

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
    /// What a slot of this node covers (default `own`): `own` holds a slot only while the node
    /// does its own work; `call` holds it until everything the node calls has answered, as a
    /// thread-per-request server does, so a slow dependency fills this node's slots too.
    #[serde(default)]
    pub hold: Option<Hold>,
    /// Extra processing time per KB of payload (request and response each, once per body), a
    /// duration like `proc`. Absent = none.
    pub per_kb: Option<DistConfig>,
    /// This node's network card, in bytes per second (at least 1): one number for both
    /// directions or `{ out, in }`. Each copy has its own send and receive pipe; payloads share
    /// the rate fairly, so calls to different services share the uplink. Absent = no limit.
    #[serde(default)]
    pub bandwidth: Option<NodeBandwidth>,
    /// CPUs this copy's own work (`proc` and database ops) shares: `{ cores: N }`, N above 0 and at most
    /// 4096, fractions like `0.5` allowed (a container's CPU limit). More work running at once than
    /// cores slows each piece down (processor sharing); `concurrency` still caps how many run. Each
    /// copy has its own cores. Absent = no sharing, every call takes its drawn time.
    #[serde(default)]
    pub cpu: Option<CpuConfig>,
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
    /// What a slot of this node covers (default `own`): `own` holds a slot only while the node
    /// does its own work; `call` holds it until everything the node calls has answered, as a
    /// thread-per-request server does, so a slow dependency fills this node's slots too.
    #[serde(default)]
    pub hold: Option<Hold>,
    /// Extra processing time per KB of payload (request and response each, once per body), a
    /// duration like `proc`. Absent = none.
    pub per_kb: Option<DistConfig>,
    /// This node's network card, in bytes per second (at least 1): one number for both
    /// directions or `{ out, in }`. Each copy has its own send and receive pipe; payloads share
    /// the rate fairly, so calls to different services share the uplink. Absent = no limit.
    #[serde(default)]
    pub bandwidth: Option<NodeBandwidth>,
    /// CPUs this copy's own work (`proc` and database ops) shares: `{ cores: N }`, N above 0 and at most
    /// 4096, fractions like `0.5` allowed (a container's CPU limit). More work running at once than
    /// cores slows each piece down (processor sharing); `concurrency` still caps how many run. Each
    /// copy has its own cores. Absent = no sharing, every call takes its drawn time.
    #[serde(default)]
    pub cpu: Option<CpuConfig>,
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
    /// What a slot of this node covers (default `own`): `own` holds a slot only while the node
    /// does its own work; `call` holds it until everything the node calls has answered, as a
    /// thread-per-request server does, so a slow dependency fills this node's slots too.
    #[serde(default)]
    pub hold: Option<Hold>,
    /// Extra processing time per KB of payload (request and response each, once per body), a
    /// duration like `proc`. Absent = none.
    pub per_kb: Option<DistConfig>,
    /// This node's network card, in bytes per second (at least 1): one number for both
    /// directions or `{ out, in }`. Each copy has its own send and receive pipe; payloads share
    /// the rate fairly, so calls to different services share the uplink. Absent = no limit.
    #[serde(default)]
    pub bandwidth: Option<NodeBandwidth>,
    /// CPUs this copy's own work (`proc` and database ops) shares: `{ cores: N }`, N above 0 and at most
    /// 4096, fractions like `0.5` allowed (a container's CPU limit). More work running at once than
    /// cores slows each piece down (processor sharing); `concurrency` still caps how many run. Each
    /// copy has its own cores. Absent = no sharing, every call takes its drawn time.
    #[serde(default)]
    pub cpu: Option<CpuConfig>,
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
    /// This node's network card, in bytes per second (at least 1): one number for both
    /// directions or `{ out, in }`. Each copy has its own send and receive pipe; payloads share
    /// the rate fairly, so calls to different services share the uplink. Absent = no limit.
    #[serde(default)]
    pub bandwidth: Option<NodeBandwidth>,
    /// CPUs this copy's own work (`proc` and database ops) shares: `{ cores: N }`, N above 0 and at most
    /// 4096, fractions like `0.5` allowed (a container's CPU limit). More work running at once than
    /// cores slows each piece down (processor sharing); `concurrency` still caps how many run. Each
    /// copy has its own cores. Absent = no sharing, every call takes its drawn time.
    #[serde(default)]
    pub cpu: Option<CpuConfig>,
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
    /// This node's network card, in bytes per second (at least 1): one number for both
    /// directions or `{ out, in }`. Each copy has its own send and receive pipe; payloads share
    /// the rate fairly, so calls to different services share the uplink. Absent = no limit.
    #[serde(default)]
    pub bandwidth: Option<NodeBandwidth>,
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
    /// What a slot of this node covers (default `own`): `own` holds a slot only while the node
    /// does its own work; `call` holds it until everything the node calls has answered, as a
    /// thread-per-request server does, so a slow dependency fills this node's slots too.
    #[serde(default)]
    pub hold: Option<Hold>,
    /// Extra processing time per KB of payload (request and response each, once per body), a
    /// duration like `proc`. Absent = none.
    pub per_kb: Option<DistConfig>,
    /// This node's network card, in bytes per second (at least 1): one number for both
    /// directions or `{ out, in }`. Each copy has its own send and receive pipe; payloads share
    /// the rate fairly, so calls to different services share the uplink. Absent = no limit.
    #[serde(default)]
    pub bandwidth: Option<NodeBandwidth>,
    /// CPUs this copy's own work (`proc` and database ops) shares: `{ cores: N }`, N above 0 and at most
    /// 4096, fractions like `0.5` allowed (a container's CPU limit). More work running at once than
    /// cores slows each piece down (processor sharing); `concurrency` still caps how many run. Each
    /// copy has its own cores. Absent = no sharing, every call takes its drawn time.
    #[serde(default)]
    pub cpu: Option<CpuConfig>,
    /// Locale pack for the data this node's routes generate, e.g. `en-US`; beats the top-level
    /// `locale`.
    #[serde(default)]
    #[schemars(extend("pattern" = LOCALE_PATTERN))]
    pub locale: Option<String>,
}
