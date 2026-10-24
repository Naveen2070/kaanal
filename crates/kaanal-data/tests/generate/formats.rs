//! Examples by seed, the newer formats, ipv4, nullable, additionalProperties and password.

use crate::common::*;
use serde_json::json;

#[test]
fn examples_pick_one_entry_by_seed_and_are_stable() {
    let arr = json!({ "type": "string", "examples": ["a", "b", "c"] });
    let map =
        json!({ "type": "string", "examples": { "x": { "value": "a" }, "y": { "value": "b" } } });
    for s in [&arr, &map] {
        let all: std::collections::BTreeSet<String> = (0..60)
            .map(|seed| g1(s, seed).as_str().unwrap().to_string())
            .collect();
        assert!(all.len() >= 2 && all.iter().all(|v| ["a", "b", "c"].contains(&v.as_str())));
        assert_eq!(g1(s, 5), g1(s, 5));
    }
    // `example` still beats `examples`, which beats `enum`
    let both = json!({ "example": "e", "examples": ["a"], "enum": ["z"] });
    assert_eq!(g1(&both, 1), json!("e"));
    let ex_enum = json!({ "examples": ["a"], "enum": ["z"] });
    assert_eq!(g1(&ex_enum, 1), json!("a"));
}

#[test]
fn each_new_format_validates_against_the_schema() {
    for (ty, f) in [
        ("string", "hostname"),
        ("string", "ipv4"),
        ("string", "ipv6"),
        ("string", "byte"),
        ("string", "password"),
        ("string", "duration"),
    ] {
        let s = json!({ "type": ty, "format": f });
        let v = jsonschema::options()
            .should_validate_formats(true)
            .build(&s)
            .unwrap();
        for seed in 0..50 {
            let out = g1(&s, seed);
            assert!(v.is_valid(&out), "{f} seed {seed}: {out}");
        }
    }
    let b = g1(&json!({ "type": "string", "format": "byte" }), 2);
    let b = b.as_str().unwrap();
    assert!(
        b.len().is_multiple_of(4)
            && b.chars()
                .all(|c| c.is_ascii_alphanumeric() || "+/=".contains(c))
    );
    let d = g1(&json!({ "type": "string", "format": "duration" }), 2);
    assert!(d.as_str().unwrap().starts_with('P'), "{d}");
}

#[test]
fn ipv4_is_in_a_documentation_range() {
    for seed in 0..100 {
        let v = g1(&json!({ "type": "string", "format": "ipv4" }), seed);
        let v = v.as_str().unwrap();
        assert!(
            v.starts_with("198.51.100.") || v.starts_with("203.0.113."),
            "{v}"
        );
    }
}

#[test]
fn nullable_emits_a_real_value() {
    let a = json!({ "type": "string", "nullable": true });
    let b = json!({ "type": ["integer", "null"] });
    let c = json!({ "type": ["null", "boolean"] });
    for seed in 0..20 {
        assert!(g1(&a, seed).is_string());
        assert!(g1(&b, seed).is_i64());
        assert!(g1(&c, seed).is_boolean());
    }
}

#[test]
fn additional_properties_schema_adds_up_to_three_keys_and_none_when_absent() {
    let with = json!({ "type": "object", "properties": { "id": { "type": "integer" } },
        "additionalProperties": { "type": "integer" } });
    let mut seen = std::collections::BTreeSet::new();
    for seed in 0..100 {
        let v = g1(&with, seed);
        let extra = v.as_object().unwrap().len() - 1;
        assert!(extra <= 3, "{v}");
        assert!(
            v.as_object()
                .unwrap()
                .iter()
                .all(|(k, x)| k == "id" || x.is_i64())
        );
        seen.insert(extra);
    }
    assert!(seen.len() >= 2, "count should vary: {seen:?}");
    let without = json!({ "type": "object", "properties": { "id": { "type": "integer" } } });
    let off = json!({ "type": "object", "properties": { "id": { "type": "integer" } },
        "additionalProperties": false });
    for seed in 0..20 {
        assert_eq!(g1(&without, seed).as_object().unwrap().len(), 1);
        assert_eq!(g1(&off, seed).as_object().unwrap().len(), 1);
    }
    // adding the keyword never changes the declared fields
    for seed in 0..20 {
        assert_eq!(g1(&with, seed)["id"], g1(&without, seed)["id"]);
    }
}

#[test]
fn int32_bounds_are_honoured() {
    let s = json!({ "type": "integer", "format": "int32", "minimum": 2_147_483_000_i64 });
    let t = json!({ "type": "integer", "format": "int32", "maximum": -2_147_483_000_i64 });
    for seed in 0..100 {
        let v = g1(&s, seed).as_i64().unwrap();
        assert!((2_147_483_000..=i64::from(i32::MAX)).contains(&v), "{v}");
        let v = g1(&t, seed).as_i64().unwrap();
        assert!((i64::from(i32::MIN)..=-2_147_483_000).contains(&v), "{v}");
    }
}

#[test]
fn password_and_byte_respect_lengths() {
    let long = json!({ "type": "string", "format": "password", "minLength": 20 });
    let short = json!({ "type": "string", "format": "password", "maxLength": 5 });
    for seed in 0..20 {
        assert!(g1(&long, seed).as_str().unwrap().chars().count() >= 20);
        assert!(g1(&short, seed).as_str().unwrap().chars().count() <= 5);
    }
}

#[test]
fn typeless_additional_properties_is_an_object() {
    let s = json!({ "additionalProperties": { "type": "integer" } });
    let mut max = 0;
    for seed in 0..50 {
        let v = g1(&s, seed);
        max = max.max(v.as_object().expect("object").len());
    }
    assert!(max >= 1);
}

// ---- T6: seeded oneOf/anyOf and discriminator ----
