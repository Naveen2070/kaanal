//! Shared helpers: the rich schema.

use kaanal_data::{Options, generate};

use serde_json::{Value, json};

pub(crate) fn rich_schema() -> Value {
    json!({
        "type": "object",
        "required": ["id", "email", "tags", "kind", "score", "nested"],
        "properties": {
            "id":     { "type": "integer", "minimum": 10, "maximum": 20 },
            "email":  { "type": "string", "format": "email" },
            "code":   { "type": "string", "minLength": 4, "maxLength": 6 },
            "kind":   { "type": "string", "enum": ["a", "b"] },
            "score":  { "type": "number", "minimum": 0.5, "maximum": 0.9 },
            "tags":   { "type": "array", "items": { "type": "string" }, "minItems": 2, "maxItems": 4 },
            "flag":   { "type": "boolean" },
            "when":   { "type": "string", "format": "date-time" },
            "uid":    { "type": "string", "format": "uuid" },
            "nested": { "allOf": [
                { "type": "object", "properties": { "x": { "type": "integer" } } },
                { "type": "object", "properties": { "y": { "type": "string" } } }
            ] }
        }
    })
}

pub(crate) fn g1(s: &Value, seed: u64) -> Value {
    generate(s, None, seed, &Options::default())
}
