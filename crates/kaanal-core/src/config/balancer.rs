//! The `load-balancer` node's config struct, split out of `nodes.rs` (R3.8: that file passed 800
//! lines). It is one cohesive type; the kind itself and its accessors stay in `nodes.rs`.

use super::Hosting;
use super::{
    ConcurrencyConfig, CpuConfig, Deserialize, DistConfig, Duration, Hold, JsonSchema,
    LOCALE_PATTERN, MemoryConfig, NodeBandwidth, RateLimitConfig, call_timeout, detect_window,
};
use crate::balance::Balance;

/// A load balancer (nginx, `HAProxy`, an ALB): a real hop with its own cost and capacity that
/// sends each call to ONE of the components it has wires to, chosen by `spread`. It has no chain of
/// its own and is never an entry node; there may be several (one in front of each tier).
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LoadBalancerConfig {
    /// Cost of passing one call through (default none).
    pub proc: Option<DistConfig>,
    /// Most calls this node passes at once, as a number (the rest wait without limit) or a map
    /// `{ limit, queue, overflow_status }`. Absent = unlimited.
    #[serde(default)]
    #[schemars(schema_with = "crate::schema::concurrency")]
    pub concurrency: Option<ConcurrencyConfig>,
    /// Token-bucket limit on how fast calls may reach this node: 429 beyond it. Absent = no rate limit.
    pub rate_limit: Option<RateLimitConfig>,
    /// Deadline for a call to this node: queue wait, its own work and the chosen component's
    /// answer. `0 < timeout <= 5 min`; absent = no deadline.
    #[serde(default, deserialize_with = "call_timeout")]
    #[schemars(schema_with = "crate::schema::duration_text")]
    pub timeout: Option<Duration>,
    /// Copies of this node, 1 to 64 (default 1).
    #[serde(default)]
    #[schemars(extend("minimum" = 1, "maximum" = 64))]
    pub replicas: Option<u16>,
    /// How traffic entering this node picks one of its copies (default `round-robin`).
    #[serde(default)]
    pub balance: Option<Balance>,
    /// How this node picks the component a call goes to: `round-robin` (default), `random`, or
    /// `{ weighted: [3, 1] }` with one weight per wire out of it, in the order of `edges:`. A
    /// component with no live copy is skipped; with none left the call fails with 503.
    #[serde(default)]
    pub spread: Option<Balance>,
    /// How long this balancer keeps sending to a component copy that has gone down before it notices
    /// (health-check interval times unhealthy threshold); default 0 = at once. A copy that is down from
    /// the start of the run is already noticed. An edge's own `detect` wins for its calls.
    #[serde(default, deserialize_with = "detect_window")]
    #[schemars(schema_with = "crate::schema::duration_text")]
    pub detect: Option<Duration>,
    /// What a slot of this node covers (default `own`); with `call` it is held until the chosen
    /// component has answered, as a connection-limited proxy does.
    #[serde(default)]
    pub hold: Option<Hold>,
    /// Extra processing time per KB of payload (request and response each, once per body). Absent = no size-dependent cost.
    pub per_kb: Option<DistConfig>,
    /// This node's network card, in bytes per second (at least 1): one number for both
    /// directions or `{ out, in }`. Absent = no limit.
    #[serde(default)]
    pub bandwidth: Option<NodeBandwidth>,
    /// CPUs this copy's own work (`proc` and database ops) shares: `{ cores: N }`, N above 0 and at most
    /// 4096, fractions like `0.5` allowed (a container's CPU limit). More work running at once than
    /// cores slows each piece down (processor sharing); `concurrency` still caps how many run. Each
    /// copy has its own cores. Absent = no sharing, every call takes its drawn time.
    #[serde(default)]
    pub cpu: Option<CpuConfig>,
    /// A cloud instance type, `provider/type` like `aws/m5.large` (`gcp/...`, `azure/...`): fills
    /// `cpu.cores`, `bandwidth` and the `memory.limit` you leave out. A value you write wins. The
    /// figures are estimates, an upper bound on capacity, not on speed (see the instance docs). Absent = none: `cpu`, `bandwidth` and `memory.limit` are then only what you write.
    #[serde(default)]
    pub instance: Option<String>,
    /// Where this component runs (default `self`): `self` is a machine you run, so `instance:` is
    /// a VM type; `managed`, `serverless` and `external` are the provider's or someone else's
    /// service (see `hosting` in the node docs for what each kind allows). It decides which
    /// `instance:` is allowed and how the cost estimate prices the node.
    #[serde(default)]
    pub hosting: Option<Hosting>,
    /// Memory as a hard limit: `{ limit, per_request, base }`. The copy holds at most
    /// `(limit - base) / per_request` requests at once, lowering `concurrency` when that is fewer.
    /// No GC or swap curve is modelled. `limit` may come from `instance`. Absent = no memory limit.
    #[serde(default)]
    pub memory: Option<MemoryConfig>,
    /// Locale pack; a balancer generates no data, so this only passes down.
    #[serde(default)]
    #[schemars(extend("pattern" = LOCALE_PATTERN))]
    pub locale: Option<String>,
}
