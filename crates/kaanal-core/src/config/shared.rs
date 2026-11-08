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
    /// Link capacity: bytes per second (at least 1) as a whole number, or text with a unit like
    /// `250KB` or `1MB/s` (KB = 1024 bytes). With a payload size, a transfer takes
    /// `bytes / bandwidth` when alone on the link, and more while other transfers share its rate
    /// fairly. Absent = no limit,
    /// and no link hop.
    #[serde(default, deserialize_with = "crate::units::bandwidth")]
    #[schemars(schema_with = "crate::schema::rate")]
    pub bandwidth: Option<u64>,
    /// Size of the request body a call over this edge carries: bytes as a whole number, or text
    /// with a unit like `200KB` (KB = 1024 bytes; at most 1 GiB). A `call:` step may override it.
    /// Absent = the size of the request that reached the caller (it is passed on).
    #[serde(default, deserialize_with = "crate::units::sends")]
    #[schemars(schema_with = "crate::schema::size")]
    pub sends: Option<u64>,
    /// Size of the response body a call over this edge brings back, written like `sends`. Absent
    /// = the size of the response the caller itself has to give.
    #[serde(default, deserialize_with = "crate::units::returns")]
    #[schemars(schema_with = "crate::schema::size")]
    pub returns: Option<u64>,
    /// Which bodies of a call over this edge stream: a response crosses the wire while the callee is
    /// still producing it, and an upload is consumed by the callee as it arrives, so the call takes
    /// `first byte + size / slowest stage` instead of the stages added. `true` is both, `false`
    /// neither, or `request` / `response`. Absent = a read of a `storage` node streams its response, a
    /// write its request, and every other call is buffered; a `call:` step's own `stream` wins.
    #[serde(default)]
    #[schemars(schema_with = "crate::schema::stream_mode")]
    pub stream: Option<super::StreamMode>,
    /// Probability that a packet on this wire is lost, 0 to 1 (0.01 is 1%); default 0, off. A lossy
    /// transfer pays fast retransmits and retransmission timeouts (long flows run at the TCP rate of
    /// RFC 5348); a lossy handshake waits 1 s per lost SYN. The cost is in round trips, so an edge
    /// with no `net` pays nothing.
    #[serde(default, deserialize_with = "super::conn::loss")]
    #[schemars(extend("minimum" = 0, "maximum" = 1))]
    pub loss: Option<f64>,
    /// The share of its size a response keeps on the wire when it is compressed (gzip, brotli): above
    /// 0 to 1, 0.3 sends 30% of the bytes; default none. The wire, slow start and loss see the smaller
    /// body; the node still handles the whole one (compressing costs the node's `per_kb`). An
    /// estimate you supply: there is no universal ratio.
    #[serde(default, deserialize_with = "super::conn::compress")]
    #[schemars(extend("exclusiveMinimum" = 0, "maximum" = 1))]
    pub compress: Option<f64>,
    /// TLS a new connection over this edge negotiates after TCP: `false` (default), `true` or
    /// `1.3` (one extra round trip) or `1.2` (two). Costs nothing on a reused connection, and
    /// nothing when the edge has no `net` (the round trip is its latency).
    #[serde(default)]
    #[schemars(schema_with = "crate::schema::tls")]
    pub tls: super::Tls,
    /// Calls a connection serves before it closes (keep-alive), 1 to 1000000, default 1000 (nginx's
    /// `keepalive_requests`). One call in `keepalive` opens a new connection and pays the TCP (and
    /// TLS) handshake and slow start; `1` is a new connection every call.
    #[serde(
        default = "super::conn::default_keepalive",
        deserialize_with = "super::conn::keepalive"
    )]
    #[schemars(extend("minimum" = 1, "maximum" = 1_000_000))]
    pub keepalive: u32,
    /// Retry a failed call over this edge (RULES 82-84): tries, backoff and an optional budget. Absent =
    /// no retry. A `call:` step's own `retry` wins.
    #[serde(default)]
    pub retry: Option<super::RetryConfig>,
    /// What happens to a call's work when its caller times out and gives up: `cancel` (default) drops what
    /// has not started, `continue` lets the callee finish it, loading it for nothing (RULES 88).
    #[serde(default)]
    pub abandon: super::Abandon,
    /// A circuit breaker on this edge (step 7c): after enough failed calls the caller stops calling for a
    /// while. Absent = none; `{}` takes resilience4j's defaults.
    #[serde(default)]
    pub breaker: Option<super::BreakerConfig>,
    /// How long the caller keeps sending to a copy of the callee that has gone down before it notices
    /// (health-check interval times unhealthy threshold, step 7d). Absent = the calling balancer's own
    /// `detect`, else 0 (at once). A copy down from the start of the run is already noticed.
    #[serde(default, deserialize_with = "detect_window")]
    #[schemars(schema_with = "crate::schema::duration_text")]
    pub detect: Option<Duration>,
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

/// A node's CPUs (`cpu:`, step 8): how many the copy's own work shares. Fractions are allowed (a
/// container limit of `0.5`); the engine keeps integer millicores.
#[derive(Debug, Clone, Copy, PartialEq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CpuConfig {
    /// CPUs, above 0 (at least 0.001) and at most 4096: a number like `4` or `0.5`.
    #[schemars(extend("minimum" = 0.001, "maximum" = 4096))]
    pub cores: f64,
}

/// **Private.** Most cores a node may set (a 4096-core machine is far past any real instance).
pub(super) const MAX_CORES: f64 = 4096.0;

impl CpuConfig {
    /// Whether `cores` is a usable number: finite, at least one millicore, at most [`MAX_CORES`].
    pub(super) fn is_valid(self) -> bool {
        self.cores.is_finite() && (self.cores * 1000.0).round() >= 1.0 && self.cores <= MAX_CORES
    }

    /// The cores as whole millicores (`0.5` is 500). Call after validation: the range makes the
    /// cast exact.
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "validated to 0.001 ..= 4096, so the product is 1 ..= 4_096_000"
    )]
    pub fn millicores(self) -> u32 {
        (self.cores * 1000.0).round() as u32
    }
}

/// A node's network card (`bandwidth:`): a rate (bytes per second as a whole number, or text
/// with a unit like `250KB/s`), one for both directions or `{ out, in }`. Each direction is at
/// least 1; a missing side of the pair is unlimited.
#[derive(Debug, Clone, Copy, PartialEq, Eq, JsonSchema)]
#[serde(untagged)]
pub enum NodeBandwidth {
    /// The same rate sending and receiving.
    Both(#[schemars(schema_with = "crate::schema::rate_value")] u64),
    /// Separate rates (full duplex).
    Split(NodeBandwidthSplit),
}

impl<'de> Deserialize<'de> for NodeBandwidth {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        /// **Private.** A number or text is both directions; a map is `{ out, in }`.
        struct V;
        impl<'de> de::Visitor<'de> for V {
            type Value = NodeBandwidth;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(
                    "a rate like `250KB/s` (or a whole number of bytes a second), or `{ out, in }`",
                )
            }

            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Self::Value, E> {
                Ok(NodeBandwidth::Both(v))
            }

            fn visit_i64<E: de::Error>(self, v: i64) -> Result<Self::Value, E> {
                u64::try_from(v).map(NodeBandwidth::Both).map_err(|_| {
                    E::custom(format!(
                        "bandwidth: {v} is negative; use a number of 0 or more"
                    ))
                })
            }

            fn visit_f64<E: de::Error>(self, v: f64) -> Result<Self::Value, E> {
                Err(E::custom(format!(
                    "bandwidth: {v} is not a whole number of bytes; write it in text with a unit, like `1.5KB/s`"
                )))
            }

            fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
                crate::units::node_rate(v).map(NodeBandwidth::Both)
            }

            fn visit_map<A: de::MapAccess<'de>>(self, m: A) -> Result<Self::Value, A::Error> {
                NodeBandwidthSplit::deserialize(de::value::MapAccessDeserializer::new(m))
                    .map(NodeBandwidth::Split)
            }
        }
        d.deserialize_any(V)
    }
}

/// The `{ out, in }` form of [`NodeBandwidth`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct NodeBandwidthSplit {
    /// Rate this node can send (its calls out and its responses): bytes per second, or text
    /// like `1MB/s`.
    #[serde(default, deserialize_with = "crate::units::out")]
    #[schemars(schema_with = "crate::schema::rate")]
    pub out: Option<u64>,
    /// Rate this node can receive: bytes per second, or text like `1MB/s`.
    #[serde(default, rename = "in", deserialize_with = "crate::units::inbound")]
    #[schemars(schema_with = "crate::schema::rate")]
    pub r#in: Option<u64>,
}

impl NodeBandwidth {
    /// `(send, receive)` in bytes per second; `None` = unlimited in that direction.
    pub fn pair(self) -> (Option<u64>, Option<u64>) {
        match self {
            NodeBandwidth::Both(b) => (Some(b), Some(b)),
            NodeBandwidth::Split(s) => (s.out, s.r#in),
        }
    }
}

/// What an HTTP call carries besides its body (`header_bytes:` at the top of the file): one size
/// for a request and for a response, or `{ request, response }`. Default 600 B and 400 B, a labelled
/// estimate; `0` turns headers off.
#[derive(Debug, Clone, Copy, PartialEq, Eq, JsonSchema)]
#[serde(untagged)]
pub enum HeaderBytes {
    /// The same size for a request and a response.
    Both(#[schemars(schema_with = "crate::schema::size_value")] u64),
    /// Separate sizes; a missing side keeps its default.
    Split(HeaderBytesSplit),
}

/// The `{ request, response }` form of [`HeaderBytes`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HeaderBytesSplit {
    /// Header bytes of a request: bytes as a whole number, or text with a unit like `1KB`.
    #[serde(default, deserialize_with = "crate::units::header_request")]
    #[schemars(schema_with = "crate::schema::size")]
    pub request: Option<u64>,
    /// Header bytes of a response, written like `request`.
    #[serde(default, deserialize_with = "crate::units::header_response")]
    #[schemars(schema_with = "crate::schema::size")]
    pub response: Option<u64>,
}

/// Header bytes of a request and a response when `header_bytes` is not written: estimates of what a
/// browser or an HTTP client sends (cookies and an auth token included) and a server answers.
pub const DEFAULT_HEADER_BYTES: (u64, u64) = (600, 400);

impl HeaderBytes {
    /// `(request, response)` in bytes.
    pub fn pair(self) -> (u64, u64) {
        match self {
            HeaderBytes::Both(b) => (b, b),
            HeaderBytes::Split(s) => (
                s.request.unwrap_or(DEFAULT_HEADER_BYTES.0),
                s.response.unwrap_or(DEFAULT_HEADER_BYTES.1),
            ),
        }
    }
}

impl<'de> Deserialize<'de> for HeaderBytes {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        /// **Private.** A number or text is both sides; a map is `{ request, response }`.
        struct V;
        impl<'de> de::Visitor<'de> for V {
            type Value = HeaderBytes;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(
                    "a size like `800B` (or a whole number of bytes), or `{ request, response }`",
                )
            }

            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Self::Value, E> {
                Ok(HeaderBytes::Both(v))
            }

            fn visit_i64<E: de::Error>(self, v: i64) -> Result<Self::Value, E> {
                u64::try_from(v).map(HeaderBytes::Both).map_err(|_| {
                    E::custom(format!(
                        "header_bytes: {v} is negative; use a number of 0 or more"
                    ))
                })
            }

            fn visit_str<E: de::Error>(self, v: &str) -> Result<Self::Value, E> {
                crate::units::parse(v, false)
                    .map(HeaderBytes::Both)
                    .map_err(|m| E::custom(format!("header_bytes: {m}")))
            }

            fn visit_map<A: de::MapAccess<'de>>(self, m: A) -> Result<Self::Value, A::Error> {
                HeaderBytesSplit::deserialize(de::value::MapAccessDeserializer::new(m))
                    .map(HeaderBytes::Split)
            }
        }
        d.deserialize_any(V)
    }
}
