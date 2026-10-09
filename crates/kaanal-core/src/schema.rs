//! The JSON Schema of `kaanal.yaml`, derived from the config types (rule R1.15).
//!
//! Editor-only: `kaanal schema` prints it, and the error path uses it to place
//! messages (the diagnostics module); the happy path never validates against it.

use std::borrow::Cow;

use schemars::generate::SchemaSettings;
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde_json::Value;

use crate::config::{Config, DistConfig};

/// **Private.** Loose shape of a `humantime` duration: numbers with unit words (`5ms`,
/// `1.5s`, `1m 30s`). serde has the final say; this only helps editors.
const DURATION_PATTERN: &str = r"^\s*([0-9]+(\.[0-9]+)?\s*[a-zA-Zµ]+\s*)+$";

/// The JSON Schema (draft 2020-12) for `kaanal.yaml`.
pub fn config_schema() -> Value {
    SchemaSettings::draft2020_12()
        .into_generator()
        .into_root_schema_for::<Config>()
        .to_value()
}

/// **Private.** Schema of a `humantime` duration string, for `Duration` fields
/// (`#[schemars(schema_with = "crate::schema::duration_text")]`).
pub(crate) fn duration_text(_: &mut SchemaGenerator) -> Schema {
    json_schema!({
        "type": "string",
        "pattern": DURATION_PATTERN,
        "description": "A duration with a unit: `5ms`, `1.5s`, `2h`."
    })
}

impl JsonSchema for DistConfig {
    fn schema_name() -> Cow<'static, str> {
        "Distribution".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        let text = duration_text(generator);
        json_schema!({
            "description": "A latency: `5ms` (fixed), `{ p50, p99 }` (lognormal fitted to both, p50 at most p99) or `{ mean }` (exponential). At most 1h.",
            "oneOf": [
                text,
                {
                    "type": "object",
                    "properties": { "p50": text, "p99": text },
                    "required": ["p50", "p99"],
                    "additionalProperties": false
                },
                {
                    "type": "object",
                    "properties": { "mean": text },
                    "required": ["mean"],
                    "additionalProperties": false
                }
            ]
        })
    }
}
