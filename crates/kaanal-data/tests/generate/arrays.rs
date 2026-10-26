//! FR-DATA-07: item counts of non-paged arrays (`minItems`, `maxItems`, caps, nesting).

use kaanal_data::ApiModel;
use serde_json::json;

use crate::common::g1;

fn len(v: &serde_json::Value) -> usize {
    v.as_array().expect("an array").len()
}

#[test]
fn min_items_greater_than_max_items_is_clamped() {
    // max is clamped up to min: the array has exactly minItems items, for every seed
    let s = json!({"type": "array", "items": {"type": "integer"}, "minItems": 5, "maxItems": 2});
    for seed in 0..50 {
        assert_eq!(len(&g1(&s, seed)), 5, "seed {seed}");
    }
    // and the spec gets one startup warning naming the route
    let src = r#"
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
              schema: { type: array, minItems: 5, maxItems: 2, items: { type: integer } }
"#;
    let w = ApiModel::from_openapi(src).unwrap().warnings();
    let hits: Vec<&String> = w.iter().filter(|m| m.contains("minItems 5")).collect();
    assert_eq!(hits.len(), 1, "{w:?}");
    assert!(
        hits[0].starts_with("GET /things") && hits[0].contains("maxItems 2"),
        "{w:?}"
    );
}

#[test]
fn max_items_zero_is_empty() {
    let s = json!({"type": "array", "items": {"type": "string"}, "maxItems": 0});
    for seed in 0..50 {
        assert_eq!(g1(&s, seed), json!([]), "seed {seed}");
    }
}

#[test]
fn huge_max_items_is_capped() {
    let s = json!({"type": "array", "items": {"type": "integer"}, "maxItems": 1_000_000});
    for seed in 0..50 {
        assert!(len(&g1(&s, seed)) <= 20, "seed {seed}");
    }
}

#[test]
fn min_items_is_honoured_for_nested_arrays() {
    let inner = json!({"type": "array", "items": {"type": "integer"}, "minItems": 3});
    let s = json!({"type": "object", "properties": {
        "rows": {"type": "array", "items": inner, "minItems": 7}
    }});
    for seed in 0..20 {
        let v = g1(&s, seed);
        let rows = v["rows"].as_array().unwrap();
        assert!(rows.len() >= 7, "seed {seed}");
        assert!(rows.iter().all(|r| len(r) >= 3), "seed {seed}");
    }
}
