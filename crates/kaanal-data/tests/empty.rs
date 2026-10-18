use kaanal_data::{empty, empty_of_value};
use serde_json::json;

#[test]
fn array_schema_is_empty_list() {
    let s = json!({"type": "array", "items": {"type": "string"}});
    assert_eq!(empty(&s, 1), Some(json!([])));
}

#[test]
fn envelope_with_data_and_meta() {
    let s = json!({"type": "object", "properties": {
        "data": {"type": "array", "items": {"type": "string"}},
        "meta": {"type": "object", "properties": {"total": {"type": "integer"}}}
    }});
    assert_eq!(
        empty(&s, 1),
        Some(json!({"data": [], "meta": {"total": 0}}))
    );
}

#[test]
fn pagination_names_are_case_insensitive() {
    let s = json!({"type": "object", "properties": {
        "TotalCount": {"type": "integer"},
        "has_more": {"type": "boolean"},
        "NextCursor": {"type": "string"}
    }});
    assert_eq!(
        empty(&s, 1),
        Some(json!({"TotalCount": 0, "has_more": false, "NextCursor": null}))
    );
}

#[test]
fn other_scalars_are_generated_and_stable_per_seed() {
    let s = json!({"type": "object", "properties": {
        "page": {"type": "integer"}, "limit": {"type": "integer"}, "name": {"type": "string"}
    }});
    let a = empty(&s, 7).unwrap();
    assert_eq!(Some(a.clone()), empty(&s, 7));
    assert!(a["page"].is_i64() && a["limit"].is_i64() && a["name"].is_string());
}

#[test]
fn oneof_and_bare_scalar_have_no_empty_form() {
    assert_eq!(empty(&json!({"oneOf": [{"type": "array"}]}), 1), None);
    assert_eq!(empty(&json!({"anyOf": [{"type": "array"}]}), 1), None);
    assert_eq!(empty(&json!({"allOf": [{"type": "array"}]}), 1), None);
    assert_eq!(empty(&json!({"type": "string"}), 1), None);
}

#[test]
fn object_without_properties_is_empty_object() {
    assert_eq!(empty(&json!({"type": "object"}), 1), Some(json!({})));
}

#[test]
fn value_form_array_object_scalar() {
    assert_eq!(empty_of_value(&json!([1, 2])), Some(json!([])));
    assert_eq!(
        empty_of_value(&json!({"items": [1], "total": 5, "name": "x"})),
        Some(json!({"items": [], "total": 0, "name": "x"}))
    );
    assert_eq!(empty_of_value(&json!(7)), None);
}

#[test]
fn name_rules_respect_property_type() {
    let s = json!({"type": "object", "properties": {
        "size": {"type": "string"}, "has_more": {"type": "string"}, "next": {"type": "boolean"}
    }});
    let e = empty(&s, 1).unwrap();
    assert!(e["size"].is_string() && e["has_more"].is_string());
    assert_eq!(e["next"], json!(null));
    let v = empty_of_value(&json!({"size": "L", "has_more": "yes", "count": 3})).unwrap();
    assert_eq!(v, json!({"size": "L", "has_more": "yes", "count": 0}));
}

#[test]
fn other_table_names_and_nested_value_arrays() {
    let s = json!({"type": "object", "properties": {
        "totalItems": {"type": "integer"}, "numFound": {"type": "number"},
        "hasNext": {"type": "boolean"}, "nextPage": {"type": "integer"}
    }});
    assert_eq!(
        empty(&s, 1),
        Some(json!({"totalItems": 0, "numFound": 0, "hasNext": false, "nextPage": null}))
    );
    assert_eq!(
        empty_of_value(&json!({"page": {"rows": [1], "hasNext": true}})),
        Some(json!({"page": {"rows": [], "hasNext": false}}))
    );
}

#[test]
fn union_types_act_like_their_non_null_member() {
    let s = json!({"type": "object", "properties": {
        "total": {"type": ["integer", "null"]},
        "hasNext": {"type": ["boolean", "null"]},
        "rows": {"type": ["array", "null"]},
        "meta": {"type": ["object", "null"], "properties": {"count": {"type": ["number", "null"]}}},
        "size": {}
    }});
    let e = empty(&s, 1).unwrap();
    assert_eq!(e["total"], json!(0));
    assert_eq!(e["hasNext"], json!(false));
    assert_eq!(e["rows"], json!([]));
    assert_eq!(e["meta"], json!({"count": 0}));
    assert_ne!(
        e["size"],
        json!(0),
        "untyped is not matched by number rules"
    );
}

#[test]
fn top_level_union_types() {
    assert_eq!(
        empty(&json!({"type": ["array", "null"]}), 1),
        Some(json!([]))
    );
    let o = json!({"type": ["object", "null"], "properties": {"total": {"type": "integer"}}});
    assert_eq!(empty(&o, 1), Some(json!({"total": 0})));
    assert_eq!(empty(&json!({"type": ["string", "null"]}), 1), None);
}
