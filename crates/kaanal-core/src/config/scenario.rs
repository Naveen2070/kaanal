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
    /// A node name. With `endpoint`: only when visited through that endpoint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node: Option<String>,
    /// `METHOD /path`: the front door of that endpoint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
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
    /// Hard outage.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub down: Option<bool>,
}

/// Parses one `scenarios/NAME.yaml` file (a BOM is stripped).
///
/// # Errors
/// Malformed YAML, an unknown key or a wrong type, with serde's line and column.
pub fn parse_scenario(src: &str) -> Result<ScenarioConfig, ConfigError> {
    let src = src.strip_prefix('\u{feff}').unwrap_or(src);
    serde_yaml_ng::from_str(src).map_err(|e| serde_error(&e))
}
