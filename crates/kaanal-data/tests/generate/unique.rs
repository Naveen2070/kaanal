//! `uniqueItems` for arrays of scalars (D2 spec section 5.5).

use std::collections::HashSet;

use kaanal_data::{ApiModel, Options, generate};
use serde_json::{Value, json};

fn one(s: &Value, seed: u64) -> Value {
    generate(s, None, seed, &Options::default())
}

fn warnings_of(schema: &str) -> Vec<String> {
    let spec = format!(
        r#"
openapi: 3.0.3
info: {{ title: t, version: "1" }}
paths:
  /x:
    get:
      responses:
        "200":
          description: ok
          content:
            application/json:
              schema: {{ type: object, properties: {{ tags: {schema} }} }}
"#
    );
    ApiModel::from_openapi(&spec).unwrap().warnings()
}

fn all_unique(v: &Value) -> bool {
    let a = v.as_array().unwrap();
    a.iter().map(Value::to_string).collect::<HashSet<_>>().len() == a.len()
}

#[test]
fn unique_scalar_enum_array_has_no_duplicates() {
    let s = json!({ "type": "array", "uniqueItems": true, "minItems": 3, "maxItems": 5,
        "items": { "type": "string", "enum": ["a", "b", "c", "d", "e", "f"] } });
    for seed in 0..500 {
        let v = one(&s, seed);
        assert!(all_unique(&v), "{v}");
        let n = v.as_array().unwrap().len();
        assert!((3..=5).contains(&n), "{v}");
    }
}

#[test]
fn unique_strings_and_integers_have_no_duplicates() {
    for items in [
        json!({ "type": "string" }),
        json!({ "type": "integer", "minimum": 1, "maximum": 8 }),
    ] {
        let s = json!({ "type": "array", "uniqueItems": true, "minItems": 6, "maxItems": 6, "items": items });
        for seed in 0..300 {
            let v = one(&s, seed);
            assert!(all_unique(&v), "{v}");
            assert_eq!(
                v.as_array().unwrap().len(),
                6,
                "enough distinct values exist: {v}"
            );
        }
    }
}

#[test]
fn too_few_distinct_values_emit_what_is_possible_and_warn() {
    let schema =
        "{ type: array, uniqueItems: true, minItems: 5, items: { type: string, enum: [a, b] } }";
    let w = warnings_of(schema);
    assert_eq!(w.len(), 1, "{w:?}");
    assert!(
        w[0].contains("only 2 distinct values but minItems is 5"),
        "{w:?}"
    );
    let s = json!({ "type": "array", "uniqueItems": true, "minItems": 5,
        "items": { "type": "string", "enum": ["a", "b"] } });
    for seed in 0..100 {
        let v = one(&s, seed);
        assert!(all_unique(&v) && v.as_array().unwrap().len() <= 2, "{v}");
    }
}

#[test]
fn unique_items_on_objects_still_warns() {
    let w = warnings_of(
        "{ type: array, uniqueItems: true, items: { type: object, properties: { k: { type: string } } } }",
    );
    assert_eq!(w.len(), 1, "{w:?}");
    assert!(
        w[0].contains(
            "uniqueItems is only supported for arrays of strings, numbers, integers and booleans"
        ),
        "{w:?}"
    );
}

#[test]
fn unique_scalars_have_no_warning_when_enough_values_exist() {
    let w = warnings_of(
        "{ type: array, uniqueItems: true, minItems: 2, items: { type: string, enum: [a, b, c] } }",
    );
    assert!(w.is_empty(), "{w:?}");
}

#[test]
fn an_array_without_unique_items_is_unchanged() {
    let plain = json!({ "type": "array", "minItems": 6, "maxItems": 6,
        "items": { "type": "integer", "minimum": 1, "maximum": 3 } });
    let unique = json!({ "type": "array", "uniqueItems": false, "minItems": 6, "maxItems": 6,
        "items": { "type": "integer", "minimum": 1, "maximum": 3 } });
    for seed in 0..50 {
        assert_eq!(one(&plain, seed), one(&unique, seed));
    }
}

#[test]
fn adding_a_property_elsewhere_does_not_change_the_unique_array() {
    let a = json!({ "type": "object", "properties": { "tags": { "type": "array", "uniqueItems": true,
        "minItems": 4, "maxItems": 4, "items": { "type": "string", "enum": ["a","b","c","d","e","f"] } } } });
    let mut b = a.clone();
    b["properties"]["other"] = json!({ "type": "string" });
    for seed in 0..100 {
        assert_eq!(one(&a, seed)["tags"], one(&b, seed)["tags"]);
    }
}

#[test]
fn a_redraw_under_a_one_of_never_becomes_null() {
    let s = json!({ "type": "array", "uniqueItems": true, "minItems": 3, "maxItems": 3,
        "items": { "oneOf": [
            { "type": "string", "enum": ["a", "b", "c", "d"] },
            { "type": "string", "enum": ["e", "f", "g", "h"] } ] } });
    for seed in 0..300 {
        let v = one(&s, seed);
        assert!(v.as_array().unwrap().iter().all(Value::is_string), "{v}");
        assert!(all_unique(&v), "{v}");
    }
}

#[test]
fn unique_items_without_an_items_schema_still_dedupes() {
    let s = json!({ "type": "array", "uniqueItems": true, "minItems": 4, "maxItems": 4 });
    for seed in 0..100 {
        assert!(all_unique(&one(&s, seed)));
    }
    let w = warnings_of("{ type: array, uniqueItems: true }");
    assert!(w.is_empty(), "{w:?}");
}
