//! Self-validation of generated responses (D2 spec section 7).

use kaanal_data::selfcheck::{Finding, SAMPLES, check_endpoint, check_model};
use kaanal_data::{ApiModel, Endpoint, Options};
use serde_json::{Value, json};

fn ep(schema: Value) -> Endpoint {
    Endpoint {
        method: "GET".into(),
        path: "/x".into(),
        status: 200,
        schema: Some(schema),
        query_params: Vec::new(),
        file_in: false,
        file_out: false,
    }
}

fn findings(schema: Value) -> Vec<Finding> {
    check_endpoint(&ep(schema), &Options::default(), SAMPLES, 7)
}

#[test]
fn valid_schema_has_no_findings() {
    let f = findings(json!({
        "type": "object",
        "required": ["id", "tags"],
        "properties": {
            "id": { "type": "integer", "minimum": 1, "maximum": 9 },
            "name": { "type": "string" },
            "tags": { "type": "array", "items": { "type": "string" }, "minItems": 1, "maxItems": 3 }
        }
    }));
    assert!(f.is_empty(), "{f:?}");
}

#[test]
fn reports_violation_with_path_and_keyword() {
    let f = findings(json!({
        "type": "object",
        "properties": { "total": { "type": "integer", "maximum": 5, "example": 99 } }
    }));
    assert_eq!(f.len(), SAMPLES, "{f:?}");
    assert!(
        f.iter()
            .all(|x| x.path == "/total" && x.keyword == "maximum" && !x.budget)
    );
    assert_eq!(f[0].endpoint, "GET /x");
}

#[test]
fn draft4_boolean_exclusive_minimum_is_accepted() {
    let f = findings(json!({
        "type": "object",
        "properties": { "n": { "type": "integer", "minimum": 0, "exclusiveMinimum": true } }
    }));
    assert!(f.is_empty(), "{f:?}");
}

#[test]
fn required_empty_array_does_not_fail_build() {
    let f = findings(json!({
        "type": "object", "required": [],
        "properties": { "a": { "type": "string" } }
    }));
    assert!(f.is_empty(), "{f:?}");
}

#[test]
fn format_violation_is_reported() {
    let f = findings(json!({
        "type": "object",
        "properties": { "mail": { "type": "string", "format": "email", "example": "not-an-email" } }
    }));
    assert!(
        f.iter().any(|x| x.path == "/mail" && x.keyword == "format"),
        "{f:?}"
    );
}

#[test]
fn write_only_property_is_not_required() {
    let f = findings(json!({
        "type": "object", "required": ["pw", "a"],
        "properties": {
            "pw": { "type": "string", "writeOnly": true },
            "a": { "type": "string" }
        }
    }));
    assert!(f.is_empty(), "{f:?}");
}

#[test]
fn nullable_accepts_null_type() {
    let f = findings(json!({
        "type": "object",
        "properties": { "n": { "type": "string", "nullable": true, "example": null } }
    }));
    assert!(f.is_empty(), "{f:?}");
}

#[test]
fn budget_null_is_classified_not_violation() {
    let f = findings(json!({
        "type": "array", "minItems": 5000,
        "items": { "type": "array", "minItems": 5000, "items": { "type": "string" } }
    }));
    assert!(!f.is_empty(), "expected budget findings");
    assert!(f.iter().all(|x| x.budget), "{:?}", &f[..f.len().min(3)]);
}

#[test]
fn endpoint_without_schema_has_no_findings() {
    let mut e = ep(json!({}));
    e.schema = None;
    let f = check_endpoint(&e, &Options::default(), SAMPLES, 1);
    assert!(f.is_empty(), "{f:?}");
}

#[test]
fn lines_are_deduplicated_and_worded_per_spec() {
    let spec = r#"
openapi: 3.0.3
info: { title: t, version: "1" }
paths:
  /x:
    get:
      responses:
        "200":
          description: ok
          content:
            application/json:
              schema:
                type: object
                properties:
                  items:
                    type: array
                    minItems: 3
                    maxItems: 3
                    items: { type: object, properties: { total: { type: integer, maximum: 5, example: 99 } } }
"#;
    let model = ApiModel::from_openapi(spec).unwrap();
    let lines = check_model(&model, &|_| Options::default(), SAMPLES);
    assert_eq!(
        lines,
        vec![
            "GET /x: 5 of 5 generated samples violate the response schema: /items/*/total: maximum"
        ]
    );
}

#[test]
fn paged_route_is_checked_as_the_client_sees_it() {
    let spec = r#"
openapi: 3.0.3
info: { title: t, version: "1" }
paths:
  /things:
    get:
      parameters:
        - { name: page, in: query, schema: { type: integer } }
      responses:
        "200":
          description: ok
          content:
            application/json:
              schema:
                type: object
                required: [items, total]
                properties:
                  total: { type: integer }
                  items: { type: array, items: { type: object, properties: { id: { type: integer } } } }
"#;
    let model = ApiModel::from_openapi(spec).unwrap();
    let lines = check_model(&model, &|_| Options::default(), SAMPLES);
    assert!(lines.is_empty(), "{lines:?}");
}

#[test]
fn unsupported_pattern_is_not_reported_again() {
    // lookahead is refused by the sampler (own startup warning), but the validator checks it
    let f = findings(json!({
        "type": "object",
        "properties": { "code": { "type": "string", "pattern": "^(?=Z)[A-Y]+$", "example": "ABC" } }
    }));
    assert!(f.iter().all(|x| x.keyword != "pattern"), "{f:?}");
}

#[test]
fn ignored_keywords_are_not_reported_again() {
    let f = findings(json!({
        "type": "object",
        "properties": { "n": { "type": "integer", "example": 7, "not": { "enum": [7] } } }
    }));
    assert!(f.is_empty(), "{f:?}");
}

#[test]
fn a_pinned_value_that_breaks_multiple_of_is_reported() {
    let f = findings(json!({
        "type": "object",
        "properties": { "n": { "type": "integer", "multipleOf": 5, "example": 7 } }
    }));
    assert!(f.iter().any(|x| x.keyword == "multipleOf"), "{f:?}");
}

#[test]
fn a_multiple_of_that_cannot_fit_is_left_to_its_own_warning() {
    let f = findings(json!({
        "type": "object",
        "properties": { "n": { "type": "integer", "multipleOf": 10, "minimum": 1, "maximum": 9 } }
    }));
    assert!(f.iter().all(|x| x.keyword != "multipleOf"), "{f:?}");
}

#[test]
fn unique_items_on_objects_is_left_to_its_own_warning() {
    let f = findings(json!({
        "type": "array", "uniqueItems": true, "minItems": 3,
        "items": { "type": "object", "properties": { "k": { "type": "string", "enum": ["a"] } } }
    }));
    assert!(f.iter().all(|x| x.keyword != "uniqueItems"), "{f:?}");
}

#[test]
fn a_paged_route_is_not_blamed_for_min_items_beyond_one_page() {
    let spec = r#"
openapi: 3.0.3
info: { title: t, version: "1" }
paths:
  /things:
    get:
      parameters:
        - { name: page, in: query, schema: { type: integer } }
      responses:
        "200":
          description: ok
          content:
            application/json:
              schema:
                type: object
                properties:
                  total: { type: integer }
                  items:
                    type: array
                    minItems: 45
                    items: { type: object, properties: { id: { type: integer } } }
"#;
    let model = ApiModel::from_openapi(spec).unwrap();
    let lines = check_model(&model, &|_| Options::default(), SAMPLES);
    assert!(lines.is_empty(), "{lines:?}");
}

#[test]
fn a_pattern_that_cannot_meet_its_length_is_left_to_its_own_warning() {
    let f = findings(json!({
        "type": "object",
        "properties": { "code": { "type": "string", "pattern": "^[A-Z]{8}$", "maxLength": 4 } }
    }));
    assert!(f.iter().all(|x| x.keyword != "pattern"), "{f:?}");
}
