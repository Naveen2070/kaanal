//! Scenario files: the `chaos:` entries, `pins:` and the scenario parser.

use super::{
    ConfigError, Deserialize, Deserializer, JsonSchema, PinEntryConfig, Serialize, serde_error,
};

/// One named scenario: chaos entries and UI-state pins applied together when it is active.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ScenarioConfig {
    /// Free text for humans.
    #[serde(default)]
    pub description: Option<String>,
    /// The chaos entries. Each names a `node`, an `endpoint` or both, and sets at least one
    /// of the patch fields. A scenario needs `chaos`, `pins` or both.
    #[serde(default)]
    pub chaos: Vec<ChaosEntry>,
    /// UI-state pins applied together with the chaos: each forces one endpoint to
    /// `loading`, `empty`, `error`, `slow` or `success`.
    #[serde(default, deserialize_with = "null_as_empty")]
    #[schemars(schema_with = "crate::schema::pins")]
    pub pins: Vec<PinEntryConfig>,
    /// **Private.** Reserved: runtime limit overrides; rejected as "not supported yet".
    /// An explicit `null` reads as absent.
    #[serde(default)]
    #[schemars(skip)]
    pub(crate) limits: Option<serde::de::IgnoredAny>,
}

/// **Private.** A list where an explicit `null` reads as empty.
pub(super) fn null_as_empty<'de, D: Deserializer<'de>, T: Deserialize<'de>>(
    d: D,
) -> Result<Vec<T>, D::Error> {
    Ok(Option::<Vec<T>>::deserialize(d)?.unwrap_or_default())
}

impl ScenarioConfig {
    /// Rejects the reserved key that parses but does nothing yet (`limits`). One source for
    /// the message: the server and `kaanal scenario load` both call it. A `null` (a key with
    /// no value) counts as absent; any other value, even `{}`, is rejected.
    ///
    /// # Errors
    /// `"limits: not supported yet (...)"`.
    pub fn check_supported(&self) -> Result<(), String> {
        if self.limits.is_some() {
            return Err("limits: not supported yet (runtime limit overrides)".into());
        }
        Ok(())
    }
}

/// One chaos entry of a scenario: where it applies plus the fields it sets.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ChaosEntry {
    /// A node name. With `endpoint`: only when visited through that endpoint; with `replica`:
    /// one copy of the node.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node: Option<String>,
    /// `METHOD /path`: the front door of that endpoint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    /// A link: the edge from one node to another (it must exist). Takes no node, endpoint or
    /// replica; sets `latency_ms`, `jitter_ms`, `error_rate` with `error_kind` `reset` or `hang`,
    /// `down` (a partition) and `bandwidth_cap`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edge: Option<EdgeRef>,
    /// With `node`: one replica of it (0-based), whatever the endpoint. Needs the node to have
    /// that many `replicas`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("minimum" = 0, "maximum" = 63))]
    pub replica: Option<u16>,
    /// Extra latency in milliseconds (at most 300000, which is `MAX_LATENCY_ADD`; a doc
    /// attribute cannot compute it, so keep the two in step).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<u64>,
    /// Failure probability, 0 to 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_rate: Option<f64>,
    /// How a failure shows up: `status` (default), `reset`, `hang` or `malformed`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_kind: Option<String>,
    /// Status of a failed visit, 500 to 504 (default 503).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_status: Option<u16>,
    /// Edge only: random extra latency of up to this many milliseconds either way.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jitter_ms: Option<u64>,
    /// Edge only: caps the edge's `bandwidth` (bytes per second, at least 1, at most the
    /// edge's own; a whole number or text with a unit like `500KB/s`).
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        deserialize_with = "crate::units::bandwidth_cap"
    )]
    #[schemars(schema_with = "crate::schema::rate")]
    pub bandwidth_cap: Option<u64>,
    /// Hard outage.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub down: Option<bool>,
}

/// Parses one `scenarios/NAME.yaml` file (a BOM is stripped).
///
/// # Errors
/// Malformed YAML, an unknown key or a wrong type, with serde's line and column.
pub fn parse_scenario(src: &str) -> Result<ScenarioConfig, ConfigError> {
    let src = super::preprocess(src)?;
    serde_yaml_ng::from_str(src).map_err(|e| serde_error(&e))
}

/// The two ends of an edge in a chaos entry: `edge: { from: app, to: db }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct EdgeRef {
    /// The calling node.
    pub from: String,
    /// The node it calls.
    pub to: String,
}
