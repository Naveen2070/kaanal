//! The JSON Schema of `kaanal.yaml`, derived from the config types (rule R1.15).
//!
//! Editor-only: `kaanal schema` prints it, and the error path uses it to place
//! messages (the diagnostics module); the happy path never validates against it.

use std::borrow::Cow;

use schemars::generate::SchemaSettings;
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde_json::Value;

use crate::config::{
    BoundedConcurrency, Config, DistConfig, EndpointConfig, EndpointsConfig, ErrorMapping,
    StepConfig,
};

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

/// **Private.** Schema of a `claims` map: names to strings, numbers or booleans (at most 32),
/// for `#[schemars(schema_with = "crate::schema::claims")]`. The load rules are in `config`.
pub(crate) fn claims(_: &mut SchemaGenerator) -> Schema {
    json_schema!({
        "type": "object",
        "maxProperties": 32,
        "additionalProperties": { "type": ["string", "number", "boolean"] }
    })
}

/// **Private.** Schema of a scenario's `pins`: a list, or null (read as empty)
/// (`#[schemars(schema_with = "crate::schema::pins")]`).
pub(crate) fn pins(generator: &mut SchemaGenerator) -> Schema {
    let list = generator.subschema_for::<Vec<crate::pin::PinEntryConfig>>();
    json_schema!({
        "description": "UI-state pins applied together with the chaos: each forces one endpoint to
    `loading`, `empty`, `error`, `slow` or `success`.",
        "default": [],
        "anyOf": [list, { "type": "null" }]
    })
}

/// **Private.** Schema of `concurrency`: a positive number, or the map form
/// (`#[schemars(schema_with = "crate::schema::concurrency")]`).
pub(crate) fn concurrency(generator: &mut SchemaGenerator) -> Schema {
    let map = generator.subschema_for::<BoundedConcurrency>();
    json_schema!({
        "description": "Most requests served at once: a number (the rest wait without limit) or `{ limit, queue, overflow_status }` (the rest wait up to `queue`, then get `overflow_status`).",
        "anyOf": [
            { "type": "integer", "minimum": 1 },
            map
        ]
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

impl JsonSchema for StepConfig {
    fn schema_name() -> Cow<'static, str> {
        "Step".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        // A `$ref` to this definition: schemars resolves the recursion by name.
        let step = generator.subschema_for::<StepConfig>();
        json_schema!({
            "description": "One step: a node name (call its default chain), `call`, `cache` or `parallel`.",
            "oneOf": [
                { "type": "string", "description": "Shorthand for `call: <node>`." },
                {
                    "type": "object",
                    "properties": {
                        "call": { "type": "string", "description": "Node to call; an edge from the running node to it must exist." },
                        "op": { "type": "string", "enum": ["read", "write", "scan"], "description": "Op for this step (database or monolith with inline ops)." },
                        "repeat": { "type": "integer", "minimum": 1, "maximum": 1000, "description": "Sequential calls; default is the edge's `calls`." },
                        "prob": { "type": "number", "exclusiveMinimum": 0, "maximum": 1, "description": "Share of requests that take this call; default is the edge's `prob`. One draw covers every `repeat`." },
                        "chain": { "type": "array", "items": step, "description": "Inline body for a service or monolith callee." },
                        "with": { "type": "string", "description": "A named chain of the callee (its `chains:`)." },
                        "via": { "type": "string", "description": "One of the callee's endpoints, `\"METHOD /path\"`; uses that endpoint's chain and op." }
                    },
                    "required": ["call"],
                    "additionalProperties": false
                },
                {
                    "type": "object",
                    "properties": { "cache": { "type": "string", "description": "Cache node to look up; a hit ends the enclosing sequence." } },
                    "required": ["cache"],
                    "additionalProperties": false
                },
                {
                    "type": "object",
                    "properties": {
                        "parallel": {
                            "type": "array",
                            "minItems": 2,
                            "maxItems": 32,
                            "items": { "oneOf": [step, { "type": "array", "items": step }] },
                            "description": "Branches (a step or a list of steps) that start together; the first failure fails the block."
                        }
                    },
                    "required": ["parallel"],
                    "additionalProperties": false
                }
            ]
        })
    }
}

impl JsonSchema for ErrorMapping {
    fn schema_name() -> Cow<'static, str> {
        "ErrorMapping".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "The response for a failure: a status, or a status and a body.",
            "oneOf": [
                { "type": "integer", "minimum": 200, "maximum": 599 },
                {
                    "type": "object",
                    "properties": {
                        "status": { "type": "integer", "minimum": 200, "maximum": 599, "description": "HTTP status to answer with." },
                        "body": { "description": "Response body, returned verbatim." }
                    },
                    "required": ["status"],
                    "additionalProperties": false
                }
            ]
        })
    }
}

impl JsonSchema for EndpointsConfig {
    fn schema_name() -> Cow<'static, str> {
        "Endpoints".into()
    }

    fn json_schema(generator: &mut SchemaGenerator) -> Schema {
        let entry = generator.subschema_for::<EndpointConfig>();
        json_schema!({
            "description": "A path to an endpoints file, or the endpoints inline.",
            "oneOf": [
                { "type": "string", "description": "Path relative to this file, e.g. `kaanal.endpoints.yaml`." },
                { "type": "object", "description": "Endpoints keyed by `\"METHOD /path\"`.", "additionalProperties": entry }
            ]
        })
    }
}
