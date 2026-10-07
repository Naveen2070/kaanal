use std::fmt;
use std::time::Duration;

use indexmap::IndexMap;
use serde::de::Error as _;
use serde::{Deserialize, Deserializer};

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub version: u32,
    #[serde(default)]
    pub seed: u64,
    #[serde(default = "default_port")]
    pub port: u16,
    pub nodes: IndexMap<String, NodeConfig>,
    #[serde(default)]
    pub edges: Vec<EdgeConfig>,
    #[serde(default)]
    pub routes: IndexMap<String, RouteOverride>,
}

fn default_port() -> u16 {
    8080
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NodeKind {
    Gateway,
    Service,
    Database,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeConfig {
    #[serde(rename = "type")]
    pub kind: NodeKind,
    #[serde(default)]
    pub flavor: Option<String>,
    #[serde(default)]
    pub proc: Option<DistConfig>,
    #[serde(default)]
    pub spec: Option<String>,
    #[serde(default)]
    pub ops: Option<OpsConfig>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpsConfig {
    pub read: DistConfig,
    pub write: DistConfig,
    pub scan: DistConfig,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EdgeConfig {
    pub from: String,
    pub to: String,
    #[serde(default)]
    pub net: Option<DistConfig>,
}

/// `5ms` (fixed) or `{ p50: 4ms, p99: 20ms }` (lognormal fitted to both).
#[derive(Debug, Clone)]
pub enum DistConfig {
    Fixed(Duration),
    Pct { p50: Duration, p99: Duration },
}

impl<'de> Deserialize<'de> for DistConfig {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        // A plain untagged enum reports "data did not match any variant"; say what is expected instead.
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

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RouteOverride {
    #[serde(default = "default_status")]
    pub status: u16,
    #[serde(default)]
    pub body: serde_json::Value,
}

fn default_status() -> u16 {
    200
}

#[derive(Debug, Clone, PartialEq)]
pub struct ConfigError {
    pub line: Option<usize>,
    pub col: Option<usize>,
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
    pub fn from_yaml(src: &str) -> Result<Config, ConfigError> {
        // Windows editors may save a UTF-8 BOM; YAML parsers reject it.
        let src = src.strip_prefix('\u{feff}').unwrap_or(src);
        let cfg: Config = serde_yaml_ng::from_str(src).map_err(|e| {
            let loc = e.location();
            ConfigError {
                line: loc.as_ref().map(|l| l.line()),
                col: loc.as_ref().map(|l| l.column()),
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
