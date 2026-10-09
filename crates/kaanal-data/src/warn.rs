//! Warnings for schema keywords the generator does not honour.
//!
//! Generated values may violate ignored keywords, so each one becomes a one-line
//! warning that `kaanal validate` and `kaanal up` print (rule R4.6). Keep
//! [`IGNORED`] in sync with the generator: if it learns a keyword, remove it here.

use serde_json::Value;

use crate::ApiModel;
use crate::generate::{MAX_DEPTH, MAX_MIN_LENGTH, count, merge_all_of};

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
    walk(schema, at, &mut out, 0);
    out
}

/// **Private.** Recursive schema walk: flags ignored keywords and `maxLength` too
/// small for a `format`, then descends into properties, items and combinators.
/// Nesting past the generator's `MAX_DEPTH` warns once: those values are `null` (R6.2).
fn walk(s: &Value, at: &str, out: &mut Vec<String>, depth: usize) {
    let Value::Object(m) = s else { return };
    // Named by endpoint only (`at` before the first ` .field`), so a branching schema warns once.
    let endpoint = at.split(" .").next().unwrap_or(at);
    let mut once = |msg: String| {
        if !out.contains(&msg) {
            out.push(msg);
        }
    };
    if m.contains_key("allOf") {
        // Walk what the generator generates: the merged schema, at the same depth.
        let (merged, conflicts) = merge_all_of(s);
        if !conflicts.is_empty() {
            once(format!("{endpoint}: allOf types conflict; first wins"));
        }
        return walk(&merged, at, out, depth);
    }
    if depth > MAX_DEPTH {
        once(format!(
            "{endpoint}: schema nests deeper than {MAX_DEPTH} levels; deeper values are null"
        ));
        return;
    }
    for k in IGNORED {
        if m.contains_key(k) {
            out.push(format!("{at}: '{k}' is not supported and is ignored"));
        }
    }
    let (min, max) = (count(s, "minLength", true), count(s, "maxLength", false));
    if let Some(min) = min.filter(|m| *m > MAX_MIN_LENGTH) {
        out.push(format!(
            "{at}: minLength {min} is capped at {MAX_MIN_LENGTH}; values are shorter"
        ));
    }
    if let Some(format) = m.get("format").and_then(Value::as_str) {
        let need = match format {
            "uuid" => 36,
            "date-time" => 20,
            "date" => 10,
            _ => 0,
        };
        if let Some(max) = max.filter(|m| *m < need) {
            out.push(format!(
                "{at}: maxLength {max} is smaller than a '{format}' value ({need}); the format wins"
            ));
        }
        if let Some(min) = min.filter(|m| need > 0 && *m > need) {
            out.push(format!(
                "{at}: minLength {min} is larger than a '{format}' value ({need}); the format wins"
            ));
        }
    }
    if let Some(Value::Object(props)) = m.get("properties") {
        for (name, p) in props {
            walk(p, &format!("{at} .{name}"), out, depth + 1);
        }
    }
    for k in ["items", "additionalProperties"] {
        if let Some(v) = m.get(k) {
            walk(v, at, out, depth + 1);
        }
    }
    for k in ["oneOf", "anyOf"] {
        if let Some(Value::Array(parts)) = m.get(k) {
            for p in parts {
                walk(p, at, out, depth + 1);
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
    fn flags_min_length_over_the_cap_or_the_format() {
        let s = json!({ "type": "object", "properties": {
            "bio": { "type": "string", "minLength": 20_000 },
            "id": { "type": "string", "format": "uuid", "minLength": 50 },
            "ok": { "type": "string", "minLength": 10_000 },
            "mail": { "type": "string", "format": "email", "minLength": 50 }
        }});
        let w = unsupported_keywords(&s, "GET /m");
        assert_eq!(w.len(), 2, "{w:?}");
        assert!(
            w[0].contains(".bio") && w[0].contains("capped at 10000"),
            "{w:?}"
        );
        assert!(
            w[1].contains(".id") && w[1].contains("minLength 50") && w[1].contains("uuid"),
            "{w:?}"
        );
    }

    #[test]
    fn allof_type_conflict_warns_once_per_endpoint() {
        let clash = json!({ "allOf": [ { "type": "string" }, { "type": "integer" } ] });
        let s = json!({ "type": "object", "properties": { "a": clash, "b": clash } });
        let w = unsupported_keywords(&s, "GET /c");
        assert_eq!(w, vec!["GET /c: allOf types conflict; first wins"]);
        let same =
            json!({ "allOf": [ { "type": "string" }, { "type": "string", "maxLength": 3 } ] });
        assert_eq!(unsupported_keywords(&same, "GET /c"), Vec::<String>::new());
    }

    #[test]
    fn clean_schema_has_no_warnings() {
        let s =
            json!({ "type": "object", "properties": { "a": { "type": "integer", "minimum": 1 } } });
        assert_eq!(unsupported_keywords(&s, "GET /ok"), Vec::<String>::new());
    }

    #[test]
    fn too_deep_schema_warns_once_and_matches_the_generator() {
        use super::{MAX_DEPTH, Value};
        let nest = |levels: usize| {
            let mut s = json!({ "type": "integer" });
            for _ in 0..levels {
                s = json!({ "type": "object", "properties": { "a": s } });
            }
            s
        };
        let leaf = |v: &Value, levels: usize| (0..levels).fold(v.clone(), |v, _| v["a"].clone());
        // Deepest allowed: no warning, and the generator still fills the leaf.
        assert_eq!(
            unsupported_keywords(&nest(MAX_DEPTH), "GET /ok"),
            Vec::<String>::new()
        );
        assert!(leaf(&crate::generate(&nest(MAX_DEPTH), None, 1), MAX_DEPTH).is_i64());
        // One level more, on two branches: one warning, and the generator gives up there too.
        let deep = json!({ "type": "object", "properties": { "x": nest(MAX_DEPTH), "y": nest(MAX_DEPTH) } });
        let w = unsupported_keywords(&deep, "GET /deep");
        assert_eq!(w.len(), 1, "once per endpoint, not per branch: {w:?}");
        assert!(
            w[0].starts_with("GET /deep: ") && w[0].contains("deeper than 16"),
            "{w:?}"
        );
        let body = crate::generate(&deep, None, 1);
        assert!(leaf(&body["x"], MAX_DEPTH - 1)["a"].is_null(), "{body}");
    }
}
