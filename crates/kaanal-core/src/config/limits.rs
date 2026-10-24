//! Capacity settings: concurrency, pools and rate limits.

use super::*;

/// How many own hops a node runs at once, and what happens to the rest.
#[derive(Debug, Clone, PartialEq)]
pub enum ConcurrencyConfig {
    /// `concurrency: 4`: four slots; requests beyond them wait without limit.
    Slots(NonZeroU32),
    /// `concurrency: { limit, queue, overflow_status }`: the same slots with a bounded queue.
    Bounded(BoundedConcurrency),
}

impl ConcurrencyConfig {
    /// The slot count of either form.
    pub fn slots(&self) -> NonZeroU32 {
        match self {
            ConcurrencyConfig::Slots(n) => *n,
            ConcurrencyConfig::Bounded(b) => b.limit,
        }
    }
}

/// The map form of `concurrency`: the same queue and overflow defaults as `pool`.
#[derive(Debug, Clone, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BoundedConcurrency {
    /// Own hops that run at once, at least 1.
    pub limit: NonZeroU32,
    /// Requests that may wait for a slot (default 0).
    #[serde(default)]
    pub queue: u32,
    /// Status returned when the queue is full, 500–504 (default 503).
    #[serde(default = "default_overflow", deserialize_with = "status_5xx")]
    #[schemars(extend("minimum" = 500, "maximum" = 504))]
    pub overflow_status: u16,
}

impl<'de> Deserialize<'de> for ConcurrencyConfig {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        struct V;
        impl<'de> Visitor<'de> for V {
            type Value = ConcurrencyConfig;
            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a positive number or a map { limit, queue, overflow_status }")
            }
            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Self::Value, E> {
                u32::try_from(v)
                    .ok()
                    .and_then(NonZeroU32::new)
                    .map(ConcurrencyConfig::Slots)
                    .ok_or_else(|| {
                        E::custom(format!(
                            "concurrency must be at least 1 and fit 32 bits (got {v})"
                        ))
                    })
            }
            fn visit_i64<E: de::Error>(self, v: i64) -> Result<Self::Value, E> {
                u64::try_from(v)
                    .map_err(|_| E::custom(format!("concurrency must be at least 1 (got {v})")))
                    .and_then(|u| self.visit_u64(u))
            }
            fn visit_map<A: MapAccess<'de>>(self, m: A) -> Result<Self::Value, A::Error> {
                BoundedConcurrency::deserialize(MapAccessDeserializer::new(m))
                    .map(ConcurrencyConfig::Bounded)
            }
        }
        d.deserialize_any(V)
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
    /// Requests per second, above 0 and at most 1000000000.
    #[serde(deserialize_with = "positive_rps")]
    #[schemars(extend("exclusiveMinimum" = 0, "maximum" = 1_000_000_000))]
    pub rps: f64,
    /// Largest burst, at least 1.
    pub burst: NonZeroU32,
}

/// **Private.** serde default for [`PoolConfig::overflow_status`].
pub(super) fn default_overflow() -> u16 {
    503
}
