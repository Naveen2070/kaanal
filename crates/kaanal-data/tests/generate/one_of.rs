//! `oneOf`, `anyOf` and discriminators: branches, mappings, warnings and depth.

use crate::common::*;
use serde_json::{Value, json};

/// A pet API whose response is `oneOf: [Dog, Cat]` by `$ref`, with `disc` (indented YAML
/// lines under the schema, or empty) next to the `oneOf`.
pub(crate) fn pets(disc: &str) -> kaanal_data::ApiModel {
    let src = format!(
        r##"openapi: 3.0.3
info: {{ title: t, version: "1" }}
paths:
  /pet:
    get:
      responses:
        "200":
          description: ok
          content:
            application/json:
              schema:
                oneOf:
                  - $ref: "#/components/schemas/Dog"
                  - $ref: "#/components/schemas/Cat"
{disc}components:
  schemas:
    Dog:
      type: object
      required: [petType, bark]
      properties:
        petType: {{ type: string }}
        bark: {{ type: boolean }}
    Cat:
      type: object
      required: [petType, lives]
      properties:
        petType: {{ type: string }}
        lives: {{ type: integer, minimum: 1, maximum: 9 }}
"##
    );
    kaanal_data::ApiModel::from_openapi(&src).unwrap()
}

pub(crate) const MAPPING: &str = r##"                discriminator:
                  propertyName: petType
                  mapping:
                    dog: "#/components/schemas/Dog"
                    cat: Cat
"##;

#[test]
fn one_of_picks_different_branches_across_seeds_and_is_stable_per_seed() {
    for k in ["oneOf", "anyOf"] {
        let s = json!({ k: [{ "type": "integer" }, { "type": "boolean" }, { "type": "string", "format": "uuid" }] });
        let kinds: std::collections::BTreeSet<&str> = (0..60)
            .map(|seed| match g1(&s, seed) {
                Value::Number(_) => "int",
                Value::Bool(_) => "bool",
                Value::String(_) => "str",
                v => panic!("{v}"),
            })
            .collect();
        assert_eq!(kinds.len(), 3, "{k}: {kinds:?}");
        for seed in 0..20 {
            assert_eq!(g1(&s, seed), g1(&s, seed));
        }
    }
    // empty arrays fall through to the type; nothing panics
    assert!(g1(&json!({ "oneOf": [] }), 1).is_string());
    assert!(g1(&json!({ "oneOf": [], "anyOf": [{ "type": "integer" }] }), 1).is_i64());
    assert!(g1(&json!({ "type": "integer", "anyOf": [] }), 1).is_i64());
}

#[test]
fn adding_a_sibling_property_next_to_one_of_changes_nothing_else() {
    let pet = json!({
        "oneOf": [
            { "type": "object", "properties": { "kind": { "enum": ["a"] }, "name": { "type": "string" }, "n": { "type": "integer" } } },
            { "type": "object", "properties": { "kind": { "enum": ["b"] }, "email": { "type": "string" }, "x": { "type": "number" } } },
            { "type": "string", "format": "uuid" }
        ],
        "discriminator": { "propertyName": "kind" }
    });
    let twin = json!({ "oneOf": [{ "type": "integer" }, { "type": "boolean" }] });
    let before = json!({ "type": "object", "properties": {
        "pet": pet, "id": { "type": "integer" }, "city": { "type": "string" },
        "twinA": twin, "twinB": twin,
        "tags": { "type": "array", "items": { "anyOf": [{ "type": "integer" }, { "type": "boolean" }] } }
    }});
    let mut after = before.clone();
    for (k, v) in [
        ("extra", json!({ "type": "boolean" })),
        (
            "other",
            json!({ "oneOf": [{ "type": "integer" }, { "type": "string" }] }),
        ),
        (
            "aaa",
            json!({ "type": "object", "properties": { "pet": pet } }),
        ),
        (
            "samples",
            json!({ "type": "string", "examples": ["s1", "s2", "s3"] }),
        ),
        (
            "labels",
            json!({ "type": "object", "additionalProperties": { "type": "string" } }),
        ),
    ] {
        after["properties"][k] = v;
    }
    let mut picks = std::collections::BTreeSet::new();
    let mut twins_differ = false;
    for seed in 0..100 {
        let (a, b) = (g1(&before, seed), g1(&after, seed));
        for key in ["pet", "id", "city", "tags", "twinA", "twinB"] {
            assert_eq!(a[key], b[key], "seed {seed} key {key}");
        }
        picks.insert(a["pet"]["kind"].as_str().unwrap_or("uuid").to_string());
        twins_differ |= a["twinA"].is_boolean() != a["twinB"].is_boolean();
    }
    assert_eq!(picks.len(), 3, "all branches reached: {picks:?}");
    // the pick follows each field's own seed, not the shared object (group) seed
    assert!(
        twins_differ,
        "identical sibling oneOfs always picked the same branch"
    );
}

#[test]
fn discriminator_with_mapping_sets_the_tag() {
    let api = pets(MAPPING);
    let s = api.endpoints[0].schema.as_ref().unwrap();
    let mut tags = std::collections::BTreeSet::new();
    for seed in 0..60 {
        let v = g1(s, seed);
        let tag = v["petType"].as_str().unwrap().to_string();
        match tag.as_str() {
            "dog" => assert!(v["bark"].is_boolean(), "{v}"),
            "cat" => assert!(v["lives"].is_i64(), "{v}"),
            _ => panic!("tag not from the mapping: {v}"),
        }
        tags.insert(tag);
    }
    assert_eq!(tags.len(), 2, "{tags:?}");
    assert!(api.warnings().is_empty(), "{:?}", api.warnings());
}

#[test]
fn discriminator_without_mapping_uses_a_single_value_enum() {
    let s = json!({
        "oneOf": [
            { "type": "object", "properties": { "kind": { "type": "string", "enum": ["circle"] }, "r": { "type": "number" } } },
            { "type": "object", "properties": { "kind": { "type": "string", "const": "square" }, "side": { "type": "number" } } }
        ],
        "discriminator": { "propertyName": "kind" }
    });
    let mut tags = std::collections::BTreeSet::new();
    for seed in 0..60 {
        let v = g1(&s, seed);
        let tag = v["kind"].as_str().unwrap().to_string();
        assert!(
            (tag == "circle" && v["r"].is_number()) || (tag == "square" && v["side"].is_number()),
            "{v}"
        );
        tags.insert(tag);
    }
    assert_eq!(tags.len(), 2, "{tags:?}");
    let api = kaanal_data::ApiModel {
        endpoints: vec![kaanal_data::Endpoint {
            method: "GET".into(),
            path: "/s".into(),
            status: 200,
            schema: Some(s),
        }],
    };
    assert!(api.warnings().is_empty(), "{:?}", api.warnings());
}

#[test]
fn discriminator_with_neither_warns_at_startup_and_still_generates() {
    let api = pets("                discriminator: { propertyName: petType }\n");
    let w = api.warnings();
    assert_eq!(w.len(), 2, "one per untagged branch: {w:?}");
    assert!(
        w.iter().all(|w| w.starts_with("GET /pet")
            && w.contains("discriminator 'petType'")
            && !w.contains("not supported")),
        "{w:?}"
    );
    let s = api.endpoints[0].schema.as_ref().unwrap();
    for seed in 0..20 {
        let v = g1(s, seed);
        assert!(
            v["petType"].is_string() && (v["bark"].is_boolean() || v["lives"].is_i64()),
            "{v}"
        );
    }
}

#[test]
fn generated_discriminated_object_validates_against_its_schema() {
    // Dog and Cat differ ONLY by the tag: without the tag enum a body matches both and
    // fails `oneOf`, so this pins the mapping rewrite, not required-property exclusivity.
    let src = r##"openapi: 3.0.3
info: { title: t, version: "1" }
paths:
  /pet:
    get:
      responses:
        "200":
          description: ok
          content:
            application/json:
              schema:
                oneOf:
                  - $ref: "#/components/schemas/Dog"
                  - $ref: "#/components/schemas/Cat"
                discriminator:
                  propertyName: petType
                  mapping: { dog: Dog, cat: "#/components/schemas/Cat" }
components:
  schemas:
    Dog: { type: object, required: [petType], properties: { petType: { type: string }, name: { type: string } } }
    Cat: { type: object, required: [petType], properties: { petType: { type: string }, name: { type: string } } }
"##;
    let api = kaanal_data::ApiModel::from_openapi(src).unwrap();
    let s = api.endpoints[0].schema.as_ref().unwrap();
    // the no-mapping path: tags from an `enum` and a `const`, branches otherwise identical
    let plain = json!({
        "oneOf": [
            { "type": "object", "required": ["kind"], "properties": { "kind": { "type": "string", "enum": ["a"] }, "name": { "type": "string" } } },
            { "type": "object", "required": ["kind"], "properties": { "kind": { "type": "string", "const": "b" }, "name": { "type": "string" } } }
        ],
        "discriminator": { "propertyName": "kind" }
    });
    for (s, tag) in [(s, "petType"), (&plain, "kind")] {
        let v = jsonschema::validator_for(s).unwrap();
        let mut tags = std::collections::BTreeSet::new();
        for seed in 0..60 {
            let out = g1(s, seed);
            assert!(v.is_valid(&out), "seed {seed}: {out}");
            tags.insert(out[tag].to_string());
        }
        assert_eq!(tags.len(), 2, "{tags:?}");
    }
}

#[test]
fn discriminator_mapping_to_no_branch_warns() {
    let api = pets(&MAPPING.replace("cat: Cat", "cat: Kat"));
    let w = api.warnings();
    assert!(
        w.iter()
            .any(|w| w.contains("mapping 'cat' names no oneOf/anyOf branch")),
        "{w:?}"
    );
    assert!(
        w.iter()
            .any(|w| w.contains("no mapping, enum or const for oneOf branch 1")),
        "{w:?}"
    );
    assert_eq!(w.len(), 2, "{w:?}");
}

#[test]
fn a_reused_discriminated_schema_warns_once_per_branch() {
    let d = json!({ "oneOf": [{ "type": "object" }, { "type": "object" }],
                    "discriminator": { "propertyName": "t" } });
    let s = json!({ "type": "object", "properties": {
        "a": d, "b": d, "c": { "type": "array", "items": d } } });
    let api = kaanal_data::ApiModel {
        endpoints: vec![kaanal_data::Endpoint {
            method: "GET".into(),
            path: "/r".into(),
            status: 200,
            schema: Some(s),
        }],
    };
    let w = api.warnings();
    assert_eq!(w.len(), 2, "{w:?}");
}

#[test]
fn nested_and_deep_one_of_stays_within_the_depth_budget() {
    // oneOf nested past MAX_DEPTH (16): no panic, and the innermost values give up as null
    let mut s = json!({ "type": "integer" });
    for _ in 0..40 {
        s = json!({ "oneOf": [{ "type": "integer" }, { "type": "object", "properties": { "a": s } }] });
    }
    for seed in 0..20 {
        let _ = g1(&s, seed);
    }
    let mut deep = json!({ "type": "integer" });
    for _ in 0..20 {
        deep = json!({ "oneOf": [deep] });
    }
    assert!(g1(&deep, 1).is_null());
    let api = kaanal_data::ApiModel {
        endpoints: vec![kaanal_data::Endpoint {
            method: "GET".into(),
            path: "/d".into(),
            status: 200,
            schema: Some(deep),
        }],
    };
    assert!(api.warnings().iter().any(|w| w.contains("deeper than 16")));
}
