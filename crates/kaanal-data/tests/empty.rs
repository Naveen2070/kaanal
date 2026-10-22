use std::sync::Arc;

use kaanal_data::{Dictionary, Options, Pack, empty, empty_of_value, generate};
use serde_json::json;

fn o() -> Options {
    Options::default()
}

#[test]
fn array_schema_is_empty_list() {
    let s = json!({"type": "array", "items": {"type": "string"}});
    assert_eq!(empty(&s, 1, &o()), Some(json!([])));
}

#[test]
fn envelope_with_data_and_meta() {
    let s = json!({"type": "object", "properties": {
        "data": {"type": "array", "items": {"type": "string"}},
        "meta": {"type": "object", "properties": {"total": {"type": "integer"}}}
    }});
    assert_eq!(
        empty(&s, 1, &o()),
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
        empty(&s, 1, &o()),
        Some(json!({"TotalCount": 0, "has_more": false, "NextCursor": null}))
    );
}

#[test]
fn other_scalars_are_generated_and_stable_per_seed() {
    let s = json!({"type": "object", "properties": {
        "page": {"type": "integer"}, "limit": {"type": "integer"}, "name": {"type": "string"}
    }});
    let a = empty(&s, 7, &o()).unwrap();
    assert_eq!(Some(a.clone()), empty(&s, 7, &o()));
    assert!(a["page"].is_i64() && a["limit"].is_i64() && a["name"].is_string());
}

#[test]
fn oneof_and_bare_scalar_have_no_empty_form() {
    assert_eq!(empty(&json!({"oneOf": [{"type": "array"}]}), 1, &o()), None);
    assert_eq!(empty(&json!({"anyOf": [{"type": "array"}]}), 1, &o()), None);
    assert_eq!(empty(&json!({"allOf": [{"type": "array"}]}), 1, &o()), None);
    assert_eq!(empty(&json!({"type": "string"}), 1, &o()), None);
}

#[test]
fn object_without_properties_is_empty_object() {
    assert_eq!(empty(&json!({"type": "object"}), 1, &o()), Some(json!({})));
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
    let e = empty(&s, 1, &o()).unwrap();
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
        empty(&s, 1, &o()),
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
    let e = empty(&s, 1, &o()).unwrap();
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
        empty(&json!({"type": ["array", "null"]}), 1, &o()),
        Some(json!([]))
    );
    let obj = json!({"type": ["object", "null"], "properties": {"total": {"type": "integer"}}});
    assert_eq!(empty(&obj, 1, &o()), Some(json!({"total": 0})));
    assert_eq!(empty(&json!({"type": ["string", "null"]}), 1, &o()), None);
}

#[test]
fn the_options_pack_and_project_dictionary_reach_the_empty_form() {
    let s = json!({"type": "object", "properties": {
        "currency": {"type": "string"}, "price": {"type": "integer"}, "items": {"type": "array"}
    }});
    let us = Options {
        pack: Pack::builtin("en-US").unwrap(),
        dictionary_project: Some(Arc::new(
            Dictionary::parse(
                "fields:
  price: { gen: static, value: 7 }
",
            )
            .unwrap(),
        )),
        ..Options::default()
    };
    assert_eq!(
        empty(&s, 3, &us),
        Some(json!({"currency": "USD", "price": 7, "items": []}))
    );
    assert_eq!(empty(&s, 3, &o()).unwrap()["currency"], json!("INR"));
}

#[test]
fn empty_form_scalars_match_generate_and_keep_sibling_seeds() {
    // R4.4: each property has its own child seed, as in `generate`, so two `oneOf`
    // properties do not pick together and adding a property changes no other value.
    let branch = json!({"oneOf": [{"const": "a"}, {"const": "b"}, {"const": "c"}]});
    let s = json!({"type": "object", "properties": {
        "p": branch, "q": branch, "name": {"type": "string"}, "firstName": {"type": "string"},
        "meta": {"type": "object", "properties": {"r": branch}}, "rows": {"type": "array"}
    }});
    let mut wider = s.clone();
    wider["properties"]["added"] = json!({"type": "integer"});
    let mut differ = false;
    for seed in 0..40 {
        let e = empty(&s, seed, &o()).unwrap();
        let g = generate(&s, None, seed, &o());
        for k in ["p", "q", "name", "firstName"] {
            assert_eq!(e[k], g[k], "{k} at seed {seed}");
        }
        assert_eq!(e["meta"]["r"], g["meta"]["r"]);
        differ |= e["p"] != e["q"];
        let w = empty(&wider, seed, &o()).unwrap();
        for k in ["p", "q", "name", "firstName", "meta"] {
            assert_eq!(e[k], w[k], "{k} moved when a sibling was added");
        }
    }
    assert!(
        differ,
        "oneOf properties of one empty body pick independently"
    );
}
