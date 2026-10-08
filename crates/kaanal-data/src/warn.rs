//! Warnings for schema keywords the generator does not honour.
//!
//! Generated values may violate ignored keywords, so each one becomes a one-line
//! warning that `kaanal validate` and `kaanal up` print (rule R4.6). Keep
//! [`IGNORED`] in sync with the generator: if it learns a keyword, remove it here.

use serde_json::Value;

use crate::ApiModel;

/// **Private.** Keywords the generator does not honour; generated values may violate them.
const IGNORED: [&str; 6] = [
    "not",
    "pattern",
    "patternProperties",
    "uniqueItems",
    "multipleOf",
    "discriminator",
];

/// One line per ignored keyword in `schema` (inlined, as in `Endpoint.schema`).
/// `at` names the endpoint, e.g. `GET /users`. Property *names* are never mistaken for keywords.
pub fn unsupported_keywords(schema: &Value, at: &str) -> Vec<String> {
    let mut out = Vec::new();
    walk(schema, at, &mut out);
    out
}

/// **Private.** Recursive schema walk: flags ignored keywords and `maxLength` too
/// small for a `format`, then descends into properties, items and combinators.
fn walk(s: &Value, at: &str, out: &mut Vec<String>) {
    let Value::Object(m) = s else { return };
    for k in IGNORED {
        if m.contains_key(k) {
            out.push(format!("{at}: '{k}' is not supported and is ignored"));
        }
    }
    if let (Some(format), Some(max)) = (
        m.get("format").and_then(Value::as_str),
        m.get("maxLength").and_then(Value::as_u64),
    ) {
        let need = match format {
            "uuid" => 36,
            "date-time" => 20,
            "date" => 10,
            _ => 0,
        };
        if max < need {
            out.push(format!(
                "{at}: maxLength {max} is smaller than a '{format}' value ({need}); the format wins"
            ));
        }
    }
    if let Some(Value::Object(props)) = m.get("properties") {
        for (name, p) in props {
            walk(p, &format!("{at} .{name}"), out);
        }
    }
    for k in ["items", "additionalProperties"] {
        if let Some(v) = m.get(k) {
            walk(v, at, out);
        }
    }
    for k in ["allOf", "oneOf", "anyOf"] {
        if let Some(Value::Array(parts)) = m.get(k) {
            for p in parts {
                walk(p, at, out);
            }
        }
    }
}

impl ApiModel {
    /// Ignored-keyword warnings for every endpoint's response schema.
    pub fn warnings(&self) -> Vec<String> {
        self.endpoints
            .iter()
            .filter_map(|e| {
                let at = format!("{} {}", e.method, e.path);
                e.schema.as_ref().map(|s| unsupported_keywords(s, &at))
            })
            .flatten()
            .collect()
    }
}
