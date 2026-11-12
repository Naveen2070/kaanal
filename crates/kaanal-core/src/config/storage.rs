//! The `storage` node's config struct and flavor label, in a file of their own (R3.8: `nodes.rs`
//! is near its size limit). One cohesive type; the kind itself and its accessors stay in `nodes.rs`.

use std::time::Duration;

use super::{
    ConcurrencyConfig, Deserialize, DistConfig, JsonSchema, LOCALE_PATTERN, NodeBandwidth,
    RateLimitConfig, call_timeout, ratio,
};
use crate::balance::Balance;

/// The first byte of a storage node that does not write one: 25 ms, the same estimate Studio starts a
/// new node with. An estimate, not a published figure (RULES 61).
fn default_first_byte() -> DistConfig {
    DistConfig::Fixed(Duration::from_millis(25))
}

/// An object store (S3, GCS, Azure Blob): a leaf that calls nothing. One call costs `first_byte`
/// plus (request bytes + response bytes) / `throughput`; every operation (`read`, `write`,
/// `scan`) costs the same. No provider publishes a first-byte latency, so there are no presets:
/// `first_byte` is written by the user (RULES 61).
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StorageConfig {
    /// Label only (`s3`, `gcs`, `blob`); it sets no numbers. Absent = no label (`s3` shown).
    pub flavor: Option<StorageFlavor>,
    /// Time to the first byte of a call: the node's service time. A duration like `proc`
    /// (`25ms`, `{ p50: 20ms, p99: 80ms }` or `{ mean: 25ms }`). Absent = 25 ms, a labelled estimate
    /// (no provider publishes one, RULES 61): write your own measurement.
    #[serde(default = "default_first_byte")]
    pub first_byte: DistConfig,
    /// Bytes per second the store moves for one call, as a whole number or text with a unit like
    /// `80MB/s` (KB = 1024 bytes); at least 1. Each call adds (request bytes + response bytes) /
    /// `throughput`. Absent = no size term.
    #[serde(default, deserialize_with = "crate::units::throughput")]
    #[schemars(schema_with = "crate::schema::rate")]
    pub throughput: Option<u64>,
    /// Most calls this node serves at once, as a number (the rest wait without limit) or a map
    /// `{ limit, queue, overflow_status }`. Absent = unlimited.
    #[serde(default)]
    #[schemars(schema_with = "crate::schema::concurrency")]
    pub concurrency: Option<ConcurrencyConfig>,
    /// Token-bucket limit on how fast calls may reach this node (the store's request rate):
    /// `rps` refilled per second, at most `burst` at once; beyond it the call fails with 503
    /// (S3's "Slow Down"), not 429 like the other kinds. Absent = no rate limit.
    pub rate_limit: Option<RateLimitConfig>,
    /// Deadline for a call to this node: queue wait plus its own work. `0 < timeout <= 5 min`;
    /// absent = no deadline.
    #[serde(default, deserialize_with = "call_timeout")]
    #[schemars(schema_with = "crate::schema::duration_text")]
    pub timeout: Option<Duration>,
    /// Copies of this node, 1 to 64 (default 1). Each has its own slots, queue and rate limit;
    /// `balance` picks one per request.
    #[serde(default)]
    #[schemars(extend("minimum" = 1, "maximum" = 64))]
    pub replicas: Option<u16>,
    /// How traffic entering this node picks a replica (default `round-robin`); a calling edge
    /// may override it.
    #[serde(default)]
    pub balance: Option<Balance>,
    /// This node's network card, in bytes per second (at least 1): one number for both
    /// directions or `{ out, in }`. Absent = no limit.
    #[serde(default)]
    pub bandwidth: Option<NodeBandwidth>,
    /// Data kept in the store, as a size (`500GB`, `2TB`... units B, KB, MB, GB), for the **cost
    /// estimate only**: it prices the storage per GB-month. Absent = nothing stored, so only
    /// requests are priced. It changes no timing.
    #[serde(default, deserialize_with = "crate::units::custom_memory")]
    #[schemars(schema_with = "crate::schema::memory_size")]
    pub stored: Option<u64>,
    /// Share of the calls that are writes (PUT, COPY, POST, LIST), 0 to 1, for the **cost estimate
    /// only**: a write costs more than a read in every object store. Absent = 0, every call priced
    /// as a read (GET), the cheaper side. It changes no timing.
    #[serde(default, deserialize_with = "ratio")]
    #[schemars(extend("minimum" = 0, "maximum" = 1))]
    pub write_share: Option<f64>,
    /// Locale pack; a storage node generates no data, so this only passes down.
    #[serde(default)]
    #[schemars(extend("pattern" = LOCALE_PATTERN))]
    pub locale: Option<String>,
}

impl StorageConfig {
    /// The size term as the engine's `per_kb` cost: `1024 / throughput` seconds per KB, a fixed
    /// duration (nanosecond resolution, so a size term is exact to about 1 part in 10^4 at
    /// 100 MB/s). `None` without a `throughput`.
    pub fn per_kb(&self) -> Option<DistConfig> {
        // f64 seconds are exact enough for a rate (a 1 B/s store is 1024 s per KB)
        #[allow(clippy::cast_precision_loss)]
        self.throughput
            .map(|t| DistConfig::Fixed(Duration::from_secs_f64(1024.0 / t.max(1) as f64)))
    }
}

/// Object-store flavors: a label for the card and docs, no numbers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum StorageFlavor {
    /// Amazon S3.
    S3,
    /// Google Cloud Storage.
    Gcs,
    /// Azure Blob Storage.
    Blob,
}

impl StorageFlavor {
    /// The YAML spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            StorageFlavor::S3 => "s3",
            StorageFlavor::Gcs => "gcs",
            StorageFlavor::Blob => "blob",
        }
    }
}
