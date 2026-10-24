//! Seeds, schema validation, allOf merging and write-only fields.

use crate::common::*;
use kaanal_data::{Options, generate};
use serde_json::json;

#[test]
fn same_seed_same_value_different_seed_differs() {
    let s = rich_schema();
    assert_eq!(
        generate(&s, None, 7, &Options::default()),
        generate(&s, None, 7, &Options::default())
    );
    assert_ne!(
        generate(&s, None, 7, &Options::default()),
        generate(&s, None, 8, &Options::default())
    );
}

#[test]
fn output_validates_against_schema() {
    let s = rich_schema();
    let validator = jsonschema::validator_for(&s).unwrap();
    for seed in 0..200 {
        let v = generate(&s, None, seed, &Options::default());
        assert!(validator.is_valid(&v), "seed {seed}: {v}");
    }
}

#[test]
fn allof_merges_properties() {
    let v = generate(&rich_schema(), None, 1, &Options::default());
    assert!(v["nested"]["x"].is_i64());
    assert!(v["nested"]["y"].is_string());
}

#[test]
fn allof_merges_non_object_parts() {
    // R6.10a: the schemas merge, so a constraint in another part applies to the value.
    let s = json!({ "allOf": [{ "type": "string" }, { "maxLength": 5 }] });
    let v = jsonschema::validator_for(&s).unwrap();
    for seed in 0..50 {
        let b = generate(&s, Some("note"), seed, &Options::default());
        assert!(v.is_valid(&b) && b.is_string(), "{b}");
    }
    // A `$ref` to an enum plus a description part (refs are inlined before generation).
    let s = json!({ "allOf": [
        { "type": "string", "enum": ["red", "green"] }, { "description": "colour" }
    ] });
    for seed in 0..20 {
        let b = generate(&s, None, seed, &Options::default());
        assert!(b == "red" || b == "green", "{b}");
    }
}

#[test]
fn allof_parts_describe_one_person_and_keep_siblings() {
    let s = json!({
        "type": "object",
        "required": ["id"],
        "properties": { "id": { "type": "integer", "minimum": 1 } },
        "allOf": [
            { "type": "object", "required": ["firstName"],
              "properties": { "firstName": { "type": "string" } } },
            { "allOf": [{ "required": ["lastName", "email"], "properties": {
                "lastName": { "type": "string" }, "email": { "type": "string", "format": "email" } } }] }
        ]
    });
    let v = jsonschema::validator_for(&s).unwrap();
    for seed in 0..100 {
        let b = generate(&s, None, seed, &Options::default());
        assert!(v.is_valid(&b), "seed {seed}: {b}");
        assert!(b["id"].as_i64().unwrap() >= 1, "sibling kept: {b}");
        let (first, last) = (
            b["firstName"].as_str().unwrap(),
            b["lastName"].as_str().unwrap(),
        );
        let prefix = format!("{}.{}", first.to_lowercase(), last.to_lowercase());
        assert!(
            b["email"].as_str().unwrap().starts_with(&prefix),
            "one person: {b}"
        );
    }
}

#[test]
fn write_only_properties_are_left_out_of_responses() {
    // `password` is not in `required`: a response never carries it (R6.10b).
    let s = json!({ "type": "object", "required": ["id"], "properties": {
        "id": { "type": "integer" },
        "password": { "type": "string", "writeOnly": true },
        "note": { "type": "string", "writeOnly": false }
    }});
    let v = jsonschema::validator_for(&s).unwrap();
    for seed in 0..20 {
        let b = generate(&s, None, seed, &Options::default());
        assert!(v.is_valid(&b), "{b}");
        assert!(b.get("password").is_none() && b["note"].is_string(), "{b}");
    }
}
