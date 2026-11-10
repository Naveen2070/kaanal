//! Parsing of `kaanal.yaml` into typed config.
//!
//! `nodes:` is a map of `type`-tagged nodes (rule R1.15). Each kind is its own struct
//! with `deny_unknown_fields`, so a field on the wrong kind fails at load. This module
//! parses and checks single values (ranges, `p50 ≤ p99`, URL shape); graph rules
//! (optional gateway, edge table, DAG, ops that presets must complete) live in
//! [`crate::topology`], route/spec wiring in the server crate. `endpoints:` (per-endpoint
//! chains, overrides and error maps) is read here and compiled by [`crate::chain`]. The editor JSON Schema is
//! derived from these types ([`crate::schema`]).

use std::fmt;
use std::num::NonZeroU32;
use std::time::Duration;

use indexmap::IndexMap;
use schemars::JsonSchema;
use serde::de::value::MapAccessDeserializer;
use serde::de::{self, Error as _, MapAccess, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};

use crate::diagnostics::{Located, PathSeg, keys};
use crate::exec::Op;
use crate::pin::{PinEntryConfig, StateOverrideConfig};
mod aliases;
mod balancer;
mod breaker;
mod checks;
mod conn;
mod endpoints;
mod kinds;
mod limits;
mod nodes;
mod retry;
mod scenario;
mod shared;
mod storage;
mod traffic;
mod values;

use crate::balance::{Balance, MAX_REPLICAS};
pub use balancer::LoadBalancerConfig;
pub use breaker::BreakerConfig;
pub(crate) use checks::prob_ok;
pub use checks::valid_locale_tag;
use checks::{
    LOCALE_PATTERN, check_auth, check_capacity, check_edges, check_locales, check_replicas,
};
pub use conn::{Conn, DEFAULT_KEEPALIVE, MAX_KEEPALIVE, StreamMode, Tls, compress_permille};
use endpoints::default_calls;
pub use endpoints::{
    Branch, CallStep, EndpointConfig, EndpointsConfig, ErrorMapping, StepConfig, parse_endpoints,
};
pub use kinds::{
    AuthConfig, CacheConfig, DatabaseConfig, ExternalApiConfig, GatewayConfig, MonolithConfig,
    ServiceConfig,
};
pub use limits::{BoundedConcurrency, ConcurrencyConfig, PoolConfig, RateLimitConfig};
pub use nodes::{CacheFlavor, DbFlavor, Hold, NodeConfig, NodeKind, OpsConfig};
pub use retry::{Abandon, Budget, DEFAULT_MAX_ATTEMPTS, MAX_ATTEMPTS, RetryConfig};
pub use scenario::{ChaosEntry, EdgeRef, ScenarioConfig, parse_scenario};
use shared::MAX_TOKEN_TTL;
pub use shared::{
    BodyEncoding, CpuConfig, DEFAULT_HEADER_BYTES, DistConfig, EdgeConfig, HeaderBytes,
    HeaderBytesSplit, Memory, MemoryConfig, NodeBandwidth, NodeBandwidthSplit, Override,
    UserConfig,
};
pub use storage::{StorageConfig, StorageFlavor};
pub use traffic::{JourneyConfig, MAX_DURATION, TrafficConfig, check_profile};
use values::{
    MAX_BURST_WINDOW_S, NOT_TEXT, call_timeout, detect_window, http_url, password_text,
    positive_rps, ratio, status_5xx, token_ttl, unique_nodes, unique_users,
};

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
    /// Header bytes an HTTP call carries on top of its body: a size for both sides, or
    /// `{ request, response }`. Default 600 B and 400 B (estimates); `0` is off. Charged on every
    /// wire transfer and on the per-KB cost of an HTTP node, so only a topology that has a
    /// `bandwidth`, a `per_kb` or a storage `throughput` is affected.
    #[serde(default)]
    pub header_bytes: Option<HeaderBytes>,
    /// `controlled` (default): timings and failures follow `seed`, so a run can be replayed.
    /// `realistic`: a seed is picked at startup (and printed), so runs differ like a real system.
    #[serde(default)]
    pub determinism: Determinism,
    /// Per-endpoint chains, overrides and error maps: a path to a YAML file (relative to this
    /// file, e.g. `kaanal.endpoints.yaml`) or an inline map keyed by `"METHOD /path"`.
    #[serde(default)]
    pub endpoints: Option<EndpointsConfig>,
    /// Named chaos scenarios (a list of chaos entries each), switched on and off at runtime.
    /// More can live in `scenarios/NAME.yaml` next to this file.
    #[serde(default)]
    pub scenarios: IndexMap<String, ScenarioConfig>,
    /// Locale pack for generated data (names, places, phone, currency), e.g. `en-US`. A node's
    /// own `locale` wins. Default `en-IN`. A built-in tag, or a pack you add as
    /// `locales/<tag>.yaml` next to this file; `kaanal validate` lists the available tags.
    #[serde(default)]
    #[schemars(extend("pattern" = LOCALE_PATTERN))]
    pub locale: Option<String>,
    /// Field dictionary for generated data, relative to this file. Default: `data.yaml` next
    /// to this file, when it exists.
    #[serde(default)]
    pub data: Option<String>,
    /// Static files served next to the mocks, a folder relative to this file.
    #[serde(default)]
    pub site: Option<String>,
    /// Named load profiles for the simulator: a total rate and a weighted mix of endpoints.
    #[serde(default)]
    pub traffic: IndexMap<String, TrafficConfig>,
    /// **Private.** Phase One's `routes:` key, kept only to explain that it was replaced.
    #[serde(default)]
    #[schemars(skip)]
    routes: Option<serde::de::IgnoredAny>,
}

/// **Private.** serde default for [`Config::port`].
fn default_port() -> u16 {
    8080
}

/// How chain draws are seeded (rule R1.34).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Determinism {
    /// Draws follow `seed:`: the same requests see the same timings and failures.
    #[default]
    Controlled,
    /// Draws follow a seed picked at startup and printed; replay a run with `controlled`
    /// and that seed.
    Realistic,
}

/// Where the seed of the chain draws comes from. New sources (e.g. per-node streams in
/// arrival order) are added as variants without a file-format change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeedSource {
    /// This exact seed (`determinism: controlled`, and the simulator always).
    Fixed(u64),
    /// A seed the driver draws from OS entropy once at startup (`determinism: realistic`).
    Entropy,
}

impl Config {
    /// The seed source `determinism` selects.
    pub fn seed_source(&self) -> SeedSource {
        match self.determinism {
            Determinism::Controlled => SeedSource::Fixed(self.seed),
            Determinism::Realistic => SeedSource::Entropy,
        }
    }
}

/// **Private.** A `serde_yaml_ng` error as a [`ConfigError`] with serde's own position.
fn serde_error(e: &serde_yaml_ng::Error) -> ConfigError {
    let loc = e.location();
    let mut msg = e.to_string();
    // serde appends " at line L column C"; the error already carries (and prints) that position.
    if let Some(l) = &loc {
        let at = format!(" at line {} column {}", l.line(), l.column());
        if let Some(m) = msg.strip_suffix(&at) {
            msg = m.to_string();
        }
    }
    ConfigError {
        line: loc.as_ref().map(serde_yaml_ng::Location::line),
        col: loc.as_ref().map(serde_yaml_ng::Location::column),
        msg,
    }
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

/// The one preprocessing step before `kaanal.yaml` text reaches the YAML parser: a UTF-8 BOM
/// (Windows editors add one; YAML rejects it) is dropped. Anything that must read the text the
/// way [`Config::from_yaml`] does (a security check) calls this too, so the two cannot differ.
#[must_use]
pub fn strip_bom(src: &str) -> &str {
    src.strip_prefix('\u{feff}').unwrap_or(src)
}

/// The whole preprocessing of `kaanal.yaml` text before it reaches a YAML parser: [`strip_bom`],
/// then a refusal of a file whose aliases expand past a bound (a "billion laughs" file would pin
/// a core for minutes; nothing real comes near it). [`Config::from_yaml`] and every security check
/// that reads the text itself (`paths::confine`) call this, so they see the same text.
///
/// # Errors
/// [`ConfigError`] ("too much alias expansion ...") for such a file; nothing has been expanded.
pub fn preprocess(src: &str) -> Result<&str, ConfigError> {
    let src = strip_bom(src);
    aliases::check(src)?;
    Ok(src)
}

impl Config {
    /// Parses `kaanal.yaml` source text.
    ///
    /// A UTF-8 BOM is stripped first (Windows editors add one; YAML rejects it —
    /// rule R6.4), and `version` must be `1`.
    ///
    /// # Errors
    /// Returns [`ConfigError`] (with line/column when the parser knows them) for
    /// malformed YAML, unknown fields, bad values, or an unsupported `version`,
    /// and for the auth rules serde cannot place (claims, scopes, limits, empty usernames),
    /// each at its own key or value; also for `pool` and `concurrency` on one node, and a
    /// rate limit window over a year.
    /// When parsing fails inside `nodes:`, the error is re-placed at the real node and
    /// field (see [`crate::diagnostics`]).
    pub fn from_yaml(src: &str) -> Result<Config, ConfigError> {
        let src = preprocess(src)?;
        let cfg: Config = serde_yaml_ng::from_str(src).map_err(|e| {
            // R1.15: serde loses positions inside tagged nodes; recover them (error path only).
            let mut err = crate::diagnostics::explain(src, serde_error(&e));
            // The private `routes` field (kept to explain its removal) must not be offered as valid.
            err.msg = err.msg.replace(", `routes`", "");
            err
        })?;
        if cfg.version != 1 {
            return Err(Located::new(
                keys(&["version"]),
                format!("unsupported config version {} (expected 1)", cfg.version),
            )
            .in_source(src));
        }
        if cfg.routes.is_some() {
            return Err(Located::new(
                keys(&["routes"]),
                "routes: was replaced by endpoints: (see docs)",
            )
            .in_source(src));
        }
        check_auth(&cfg).map_err(|e| e.in_source(src))?;
        check_capacity(&cfg).map_err(|e| e.in_source(src))?;
        check_edges(&cfg).map_err(|e| e.in_source(src))?;
        check_replicas(&cfg).map_err(|e| e.in_source(src))?;
        check_locales(&cfg).map_err(|e| e.in_source(src))?;
        traffic::check_traffic(&cfg.traffic).map_err(|e| e.in_source(src))?;
        Ok(cfg)
    }
}
