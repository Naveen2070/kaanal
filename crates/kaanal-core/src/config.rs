//! Parsing of `kaanal.yaml` into typed config.
//!
//! This module only parses; semantic validation (one gateway, DAG, edge shapes)
//! happens in [`crate::topology`], route/spec wiring in the server crate.
//! Unknown fields are rejected everywhere, so a typo fails loudly at load.

use std::fmt;
use std::time::Duration;

use indexmap::IndexMap;
use serde::de::Error as _;
use serde::{Deserialize, Deserializer};

/// The parsed `kaanal.yaml` file.
#[derive(Debug, Clone, Deserialize)]
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
    pub nodes: IndexMap<String, NodeConfig>,
    /// Dependency edges between nodes.
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

/// The kind of a topology node (`type:` in YAML, lowercase).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NodeKind {
    /// Entry point of every request path; exactly one per topology.
    Gateway,
    /// A backend service; may carry an `OpenAPI` `spec`.
    Service,
    /// A data store with `ops` costs (read/write/scan).
    Database,
}

/// One node in `nodes:`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeConfig {
    /// Node kind (YAML key `type`).
    #[serde(rename = "type")]
    pub kind: NodeKind,
    /// Cosmetic label (`postgres`, `redis`, …) shown in status output.
    #[serde(default)]
    pub flavor: Option<String>,
    /// Processing-time distribution for gateway/service nodes.
    #[serde(default)]
    pub proc: Option<DistConfig>,
    /// `OpenAPI` spec file (relative to the config's directory); service nodes only.
    #[serde(default)]
    pub spec: Option<String>,
    /// Operation costs; required on database nodes, invalid elsewhere.
    #[serde(default)]
    pub ops: Option<OpsConfig>,
}

/// Per-operation latency costs of a database node. All three are required.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpsConfig {
    /// Cost of reading one row.
    pub read: DistConfig,
    /// Cost of a write.
    pub write: DistConfig,
    /// Cost of a list/search.
    pub scan: DistConfig,
}

/// One entry in `edges:`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EdgeConfig {
    /// Name of the upstream node.
    pub from: String,
    /// Name of the downstream node.
    pub to: String,
    /// Network latency on this edge; absent means zero.
    #[serde(default)]
    pub net: Option<DistConfig>,
}

/// A duration in YAML: `5ms` (fixed) or `{ p50: 4ms, p99: 20ms }` (lognormal fitted to both).
#[derive(Debug, Clone)]
pub enum DistConfig {
    /// A single fixed duration, parsed with `humantime`.
    Fixed(Duration),
    /// A percentile pair; fitted to a lognormal by [`crate::Dist::from_p50_p99`].
    Pct {
        /// 50th percentile (median).
        p50: Duration,
        /// 99th percentile.
        p99: Duration,
    },
}

impl<'de> Deserialize<'de> for DistConfig {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        // A plain untagged enum reports "data did not match any variant"; say what is expected instead.
        /// **Private.** Parsing shim for the two YAML shapes of a duration.
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Raw {
            Fixed(#[serde(with = "humantime_serde")] Duration),
            Pct {
                #[serde(with = "humantime_serde")]
                p50: Duration,
                #[serde(with = "humantime_serde")]
                p99: Duration,
            },
        }
        Raw::deserialize(d)
            .map(|r| match r {
                Raw::Fixed(x) => DistConfig::Fixed(x),
                Raw::Pct { p50, p99 } => DistConfig::Pct { p50, p99 },
            })
            .map_err(|_| {
                D::Error::custom("expected a duration like `5ms`, or `{ p50: 1ms, p99: 3ms }`")
            })
    }
}

/// A static response override for a `"METHOD /path"` key in `routes:`.
#[derive(Debug, Clone, Deserialize)]
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
    pub fn from_yaml(src: &str) -> Result<Config, ConfigError> {
        // Windows editors may save a UTF-8 BOM; YAML parsers reject it.
        let src = src.strip_prefix('\u{feff}').unwrap_or(src);
        let cfg: Config = serde_yaml_ng::from_str(src).map_err(|e| {
            let loc = e.location();
            ConfigError {
                line: loc.as_ref().map(serde_yaml_ng::Location::line),
                col: loc.as_ref().map(serde_yaml_ng::Location::column),
                msg: e.to_string(),
            }
        })?;
        if cfg.version != 1 {
            return Err(ConfigError {
                line: None,
                col: None,
                msg: format!("unsupported config version {} (expected 1)", cfg.version),
            });
        }
        Ok(cfg)
    }
}
