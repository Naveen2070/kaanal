//! Pieces shared by several nodes: users, edges, latency distributions and endpoint overrides.

use super::*;

/// One account on an `auth` node (synthetic accounts only). `Debug` never shows the password.
#[derive(Clone, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UserConfig {
    /// Login name and the token's `sub`: unique per node, 1 to 256 bytes.
    pub username: String,
    /// Plain-text password, at most 256 bytes. Never logged or printed. Must be text: put a
    /// number or `true` in quotes.
    #[serde(deserialize_with = "password_text")]
    pub password: String,
    /// Roles put into the token's `roles` claim (at most 32).
    #[serde(default)]
    pub roles: Vec<String>,
    /// OAuth scopes, joined with spaces into the token's `scope` claim (at most 32; each
    /// printable ASCII without space, `"` or `\`).
    #[serde(default)]
    pub scopes: Vec<String>,
    /// Extra claims in this user's token (string, number or boolean; at most 32); they win
    /// over the node's `claims`. Same reserved names.
    #[serde(default)]
    #[schemars(schema_with = "crate::schema::claims")]
    pub claims: IndexMap<String, serde_json::Value>,
}

impl fmt::Debug for UserConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Passwords never reach logs, panic messages or test output (spec section 5).
        f.debug_struct("UserConfig")
            .field("username", &self.username)
            .field("password", &"<redacted>")
            .field("roles", &self.roles)
            .field("scopes", &self.scopes)
            .field("claims", &self.claims)
            .finish()
    }
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
    /// Share of requests that take this edge, above 0 and at most 1 (default 1). The default for
    /// every `call:` step over this edge; a step's own `prob` wins. Not for caches (they keep
    /// their hit ratio) or edges out of a gateway.
    #[serde(default)]
    #[schemars(extend("exclusiveMinimum" = 0, "maximum" = 1))]
    pub prob: Option<f64>,
    /// How calls over this edge pick a replica of the callee; overrides the callee's own
    /// `balance`.
    #[serde(default)]
    pub balance: Option<crate::balance::Balance>,
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
pub(super) const MAX_DURATION: Duration = Duration::from_secs(3600);

/// **Private.** Longest `token_ttl`.
pub(super) const MAX_TOKEN_TTL: Duration = Duration::from_hours(24);

/// A fixed response for an endpoint (`override:`), served instead of generated data.
#[derive(Debug, Clone, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Override {
    /// HTTP status to return. Defaults to 200.
    #[serde(default = "default_status")]
    pub status: u16,
    /// Response body, returned verbatim.
    #[serde(default)]
    pub body: serde_json::Value,
}

/// **Private.** serde default for [`Override::status`].
pub(super) fn default_status() -> u16 {
    200
}
