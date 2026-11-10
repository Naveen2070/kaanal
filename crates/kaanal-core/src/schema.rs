//! The JSON Schema of `kaanal.yaml`, derived from the config types (rule R1.15).
//!
//! Editor-only: `kaanal schema` prints it, and the error path uses it to place
//! messages (the diagnostics module); the happy path never validates against it.

use std::borrow::Cow;

use schemars::generate::SchemaSettings;
use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde_json::{Value, json};

use crate::balance::Balance;
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

/// **Private.** Loose shape of a byte rate (`250KB`, `1.5 MB/s`, `512B`). `units::parse` has the
/// final say; this only helps editors.
const RATE_PATTERN: &str = r"^\s*[0-9]+(\.[0-9]+)?\s*([kKmMgG][iI]?)?[bB]\s*(/[sS])?\s*$";
/// **Private.** Loose shape of a byte size (`2KB`, `1.5 MB`, `512B`): a rate without `/s`.
const SIZE_PATTERN: &str = r"^\s*[0-9]+(\.[0-9]+)?\s*([kKmMgG][iI]?)?[bB]\s*$";

/// **Private.** A whole number of at least `min`, or text with a byte unit; `null` when `optional`.
fn bytes_schema(pattern: &str, min: u32, what: &str, optional: bool) -> Schema {
    let mut any = vec![
        json!({ "type": "integer", "minimum": min }),
        json!({ "type": "string", "pattern": pattern }),
    ];
    if optional {
        any.push(json!({ "type": "null" }));
    }
    json_schema!({ "description": what, "anyOf": any })
}

/// **Private.** Schema of an optional byte rate field (`node` and edge `bandwidth`, `out`, `in`,
/// `bandwidth_cap`): `#[schemars(schema_with = "crate::schema::rate")]`.
pub(crate) fn rate(_: &mut SchemaGenerator) -> Schema {
    bytes_schema(
        RATE_PATTERN,
        1,
        "Bytes per second as a whole number (at least 1), or text with a unit: `250KB`, `1.5MB/s`, `512B` (units B, KB, MB, GB; KB = 1024 bytes; `/s` optional).",
        true,
    )
}

/// **Private.** Schema of a required byte rate (the one-number form of a node's `bandwidth`).
pub(crate) fn rate_value(_: &mut SchemaGenerator) -> Schema {
    bytes_schema(
        RATE_PATTERN,
        1,
        "Bytes per second as a whole number (at least 1), or text with a unit: `250KB`, `1.5MB/s`, `512B` (units B, KB, MB, GB; KB = 1024 bytes; `/s` optional).",
        false,
    )
}

/// **Private.** Schema of an optional payload size (`req_bytes`, `resp_bytes`).
pub(crate) fn size(_: &mut SchemaGenerator) -> Schema {
    bytes_schema(
        SIZE_PATTERN,
        0,
        "A whole number of bytes, or text with a unit: `2KB`, `1.5MB`, `512B` (units B, KB, MB, GB; KB = 1024 bytes; at most 1 GiB).",
        true,
    )
}

/// **Private.** Schema of a required payload size (the one-number form of `header_bytes`).
pub(crate) fn size_value(_: &mut SchemaGenerator) -> Schema {
    bytes_schema(
        SIZE_PATTERN,
        0,
        "A whole number of bytes, or text with a unit: `2KB`, `1.5MB`, `512B` (units B, KB, MB, GB; KB = 1024 bytes; at most 1 GiB).",
        false,
    )
}

/// **Private.** Schema of a memory size (`memory:` `limit`, `per_request`, `base`): bytes or text
/// with a unit, no 1 GiB cap.
pub(crate) fn memory_size(_: &mut SchemaGenerator) -> Schema {
    bytes_schema(
        SIZE_PATTERN,
        1,
        "A whole number of bytes (at least 1), or text with a unit: `64MB`, `512MiB`, `8GB` (units B, KB, MB, GB; 1 KB = 1024 bytes).",
        true,
    )
}

/// **Private.** Schema of `stream`: a boolean (both bodies, or neither) or one body.
pub(crate) fn stream_mode(_: &mut SchemaGenerator) -> Schema {
    json_schema!({
        "description": "Which bodies stream while the callee works: `true` (both) or `false` (neither), or `request`, `response`, `both`, `none`. Absent = a storage read streams its response and a storage write its request.",
        "anyOf": [
            { "type": "boolean" },
            { "type": "string", "enum": ["request", "response", "both", "none"] },
        ]
    })
}

/// **Private.** Schema of an edge's `tls`: `false`, `true`, or the version `1.3` or `1.2`.
pub(crate) fn tls(_: &mut SchemaGenerator) -> Schema {
    json_schema!({
        "description": "TLS a new connection negotiates after TCP: `false` (default), `true` or `1.3` (one extra round trip), `1.2` (two).",
        "anyOf": [
            { "type": "boolean" },
            { "type": "number", "enum": [1.2, 1.3] },
        ]
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

impl JsonSchema for Balance {
    fn schema_name() -> Cow<'static, str> {
        "Balance".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "How traffic picks a replica: `round-robin` (default), `random`, `least-conn` (the replica with the fewest requests in it), `p2c` (the less loaded of two random replicas), or `{ weighted: [..] }` with one weight per replica (at least one above 0).",
            "anyOf": [
                { "type": "string", "enum": ["round-robin", "random", "least-conn", "p2c"] },
                {
                    "type": "object",
                    "properties": {
                        "weighted": {
                            "type": "array",
                            "items": { "type": "integer", "minimum": 0 },
                            "minItems": 1,
                            "maxItems": 64
                        }
                    },
                    "required": ["weighted"],
                    "additionalProperties": false
                }
            ]
        })
    }
}
