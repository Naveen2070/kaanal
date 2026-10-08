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

/// **Private.** One line per ignored keyword in `schema` (inlined, as in `Endpoint.schema`).
/// `at` names the endpoint, e.g. `GET /users`. Property *names* are never mistaken for keywords.
fn unsupported_keywords(schema: &Value, at: &str) -> Vec<String> {
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

#[cfg(test)]
mod tests {
    use super::unsupported_keywords;
    use serde_json::json;
    #[test]
    fn flags_ignored_keywords_but_not_property_names() {
        let s = json!({ "type": "object", "properties": {
            "pattern": { "type": "string" },
            "code": { "type": "string", "pattern": "^[A-Z]+$" },
            "tags": { "type": "array", "uniqueItems": true, "items": { "type": "string" } }
        }});
        let w = unsupported_keywords(&s, "GET /x");
        assert_eq!(w.len(), 2, "{w:?}");
        assert!(w[0].contains("'pattern'") && w[0].contains("code"), "{w:?}");
        assert!(
            w[1].contains("'uniqueItems'") && w[1].contains("tags"),
            "{w:?}"
        );
    }

    #[test]
    fn walks_items_and_combinators() {
        let s = json!({ "type": "array", "items": { "allOf": [ { "not": { "type": "null" } } ] } });
        let w = unsupported_keywords(&s, "GET /y");
        assert_eq!(w.len(), 1, "{w:?}");
        assert!(w[0].contains("'not'"), "{w:?}");
    }

    #[test]
    fn flags_max_length_that_cannot_fit_the_format() {
        let s = json!({ "type": "object", "properties": {
            "id": { "type": "string", "format": "uuid", "maxLength": 10 },
            "ok": { "type": "string", "format": "uuid", "maxLength": 36 }
        }});
        let w = unsupported_keywords(&s, "GET /z");
        assert_eq!(w.len(), 1, "{w:?}");
        assert!(
            w[0].contains("maxLength 10") && w[0].contains("uuid"),
            "{w:?}"
        );
    }

    #[test]
    fn clean_schema_has_no_warnings() {
        let s =
            json!({ "type": "object", "properties": { "a": { "type": "integer", "minimum": 1 } } });
        assert_eq!(unsupported_keywords(&s, "GET /ok"), Vec::<String>::new());
    }
}
