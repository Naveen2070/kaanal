use kaanal_data::{ApiModel, unsupported_keywords};
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
    let s = json!({ "type": "object", "properties": { "a": { "type": "integer", "minimum": 1 } } });
    assert_eq!(unsupported_keywords(&s, "GET /ok"), Vec::<String>::new());
}

#[test]
fn api_model_collects_warnings_per_endpoint() {
    let spec = r#"
openapi: 3.0.3
info: { title: t, version: "1" }
paths:
  /things:
    get:
      responses:
        "200":
          description: ok
          content:
            application/json:
              schema: { type: object, properties: { code: { type: string, pattern: "^A" } } }
"#;
    let w = ApiModel::from_openapi(spec).unwrap().warnings();
    assert_eq!(w.len(), 1, "{w:?}");
    assert!(w[0].starts_with("GET /things"), "{w:?}");
}
