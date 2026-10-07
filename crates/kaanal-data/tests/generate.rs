use kaanal_data::{apply_params, generate};
use serde_json::{Value, json};

fn rich_schema() -> Value {
    json!({
        "type": "object",
        "required": ["id", "email", "tags", "kind", "score", "nested"],
        "properties": {
            "id":     { "type": "integer", "minimum": 10, "maximum": 20 },
            "email":  { "type": "string", "format": "email" },
            "code":   { "type": "string", "minLength": 4, "maxLength": 6 },
            "kind":   { "type": "string", "enum": ["a", "b"] },
            "score":  { "type": "number", "minimum": 0.5, "maximum": 0.9 },
            "tags":   { "type": "array", "items": { "type": "string" }, "minItems": 2, "maxItems": 4 },
            "flag":   { "type": "boolean" },
            "when":   { "type": "string", "format": "date-time" },
            "uid":    { "type": "string", "format": "uuid" },
            "nested": { "allOf": [
                { "type": "object", "properties": { "x": { "type": "integer" } } },
                { "type": "object", "properties": { "y": { "type": "string" } } }
            ] }
        }
    })
}

#[test]
fn same_seed_same_value_different_seed_differs() {
    let s = rich_schema();
    assert_eq!(generate(&s, None, 7), generate(&s, None, 7));
    assert_ne!(generate(&s, None, 7), generate(&s, None, 8));
}

#[test]
fn output_validates_against_schema() {
    let s = rich_schema();
    let validator = jsonschema::validator_for(&s).unwrap();
    for seed in 0..200 {
        let v = generate(&s, None, seed);
        assert!(validator.is_valid(&v), "seed {seed}: {v}");
    }
}

#[test]
fn allof_merges_properties() {
    let v = generate(&rich_schema(), None, 1);
    assert!(v["nested"]["x"].is_i64());
    assert!(v["nested"]["y"].is_string());
}

#[test]
fn formats_look_right() {
    let v = generate(&rich_schema(), None, 3);
    let when = v["when"].as_str().unwrap();
    assert_eq!(when.len(), 20, "{when}");
    assert!(when.ends_with('Z') && &when[10..11] == "T", "{when}");
    let uid = v["uid"].as_str().unwrap();
    assert_eq!(uid.len(), 36);
    assert_eq!(&uid[14..15], "4", "uuid v4: {uid}");
}

#[test]
fn example_and_enum_win() {
    assert_eq!(
        generate(&json!({ "type": "string", "example": "hello" }), None, 1),
        json!("hello")
    );
    let e = generate(&json!({ "type": "string", "enum": ["x"] }), None, 1);
    assert_eq!(e, json!("x"));
}

#[test]
fn field_name_heuristics() {
    let s = json!({ "type": "object", "properties": {
        "email": { "type": "string" },
        "user_email": { "type": "string" },
        "firstName": { "type": "string" },
        "pincode": { "type": "string" },
        "createdAt": { "type": "string" },
        "price": { "type": "number" },
        "phone": { "type": "string" },
        "city": { "type": "string" },
        "currency": { "type": "string" }
    }});
    for seed in 0..50 {
        let v = generate(&s, None, seed);
        for key in ["email", "user_email"] {
            let e = v[key].as_str().unwrap();
            assert!(e.ends_with("@example.com") && e.contains('.'), "{key}: {e}");
        }
        let first = v["firstName"].as_str().unwrap();
        assert!(
            !first.is_empty() && first.chars().all(char::is_alphabetic),
            "{first}"
        );
        let pin = v["pincode"].as_str().unwrap();
        assert!(
            pin.len() == 6 && pin.chars().all(|c| c.is_ascii_digit()),
            "{pin}"
        );
        let created = v["createdAt"].as_str().unwrap();
        assert!(
            created.starts_with("2025-") || created.starts_with("2026-"),
            "{created}"
        );
        let price = v["price"].as_f64().unwrap();
        assert!((99.0..=4999.0).contains(&price), "{price}");
        assert!(v["phone"].as_str().unwrap().starts_with("+91 "));
        assert!(!v["city"].as_str().unwrap().is_empty());
        assert_eq!(v["currency"], "INR");
    }
}

#[test]
fn apply_params_sets_matching_fields() {
    let mut v = json!({ "id": 7, "slug": "x", "other": 1 });
    apply_params(
        &mut v,
        &[("id".into(), "42".into()), ("slug".into(), "abc".into())],
    );
    assert_eq!(v, json!({ "id": 42, "slug": "abc", "other": 1 }));
}

#[test]
fn apply_params_keeps_integer_when_param_is_not_numeric() {
    let mut v = json!({ "id": 7 });
    apply_params(&mut v, &[("id".into(), "abc".into())]);
    assert_eq!(v, json!({ "id": 7 }));
}

#[test]
fn deep_or_odd_schemas_do_not_panic() {
    assert!(generate(&json!({}), None, 1).is_string());
    let v = generate(
        &json!({ "type": "array", "minItems": 0, "maxItems": 0 }),
        None,
        1,
    );
    assert_eq!(v, json!([]));
    let _ = generate(
        &json!({ "type": "integer", "minimum": 5, "maximum": 1 }),
        None,
        1,
    );
}

#[test]
fn huge_max_items_stays_small() {
    let s = json!({ "type": "array", "items": { "type": "integer" }, "maxItems": 1_000_000 });
    for seed in 0..20 {
        assert!(generate(&s, None, seed).as_array().unwrap().len() <= 20);
    }
}

#[test]
fn only_real_id_fields_get_ids() {
    let s = json!({ "type": "object", "properties": {
        "userId": { "type": "integer" },
        "valid": { "type": "integer" },
        "orderId": { "type": "string" },
        "paid": { "type": "string" }
    }});
    for seed in 0..200 {
        let v = generate(&s, None, seed);
        assert!(v["userId"].as_i64().unwrap() >= 1);
        assert_eq!(v["orderId"].as_str().unwrap().len(), 36);
        assert_ne!(v["paid"].as_str().unwrap().len(), 36);
        // `valid` uses the generic 0..=1000 range, not the id range 1..=99_999
        assert!(v["valid"].as_i64().unwrap() <= 1000);
    }
}

#[test]
fn minimum_above_default_range_still_varies() {
    let int = json!({ "type": "integer", "minimum": 5000 });
    let num = json!({ "type": "number", "minimum": 5000.0 });
    let ints: std::collections::HashSet<_> = (0..20)
        .map(|s| generate(&int, None, s).to_string())
        .collect();
    let nums: std::collections::HashSet<_> = (0..20)
        .map(|s| generate(&num, None, s).to_string())
        .collect();
    assert!(ints.len() > 1 && nums.len() > 1);
    assert!((0..20).all(|s| generate(&int, None, s).as_i64().unwrap() >= 5000));
}

#[test]
fn adding_or_reordering_properties_keeps_other_values() {
    let before = json!({ "type": "object", "properties": {
        "firstName": { "type": "string" }, "city": { "type": "string" }, "score": { "type": "integer" }
    }});
    let after = json!({ "type": "object", "properties": {
        "phone": { "type": "string" },
        "score": { "type": "integer" }, "firstName": { "type": "string" },
        "extra": { "type": "boolean" }, "city": { "type": "string" }
    }});
    for seed in 0..20 {
        let (a, b) = (generate(&before, None, seed), generate(&after, None, seed));
        for key in ["firstName", "city", "score"] {
            assert_eq!(a[key], b[key], "seed {seed} key {key}");
        }
    }
}

#[test]
fn exclusive_bounds_are_strict() {
    let int = json!({ "type": "integer", "minimum": 1, "maximum": 3,
                      "exclusiveMinimum": true, "exclusiveMaximum": true });
    assert!((0..50).all(|s| generate(&int, None, s) == json!(2)));
    let num = json!({ "type": "number", "minimum": 0.0, "maximum": 1.0,
                      "exclusiveMinimum": true, "exclusiveMaximum": true });
    for seed in 0..200 {
        let v = generate(&num, None, seed).as_f64().unwrap();
        assert!(v > 0.0 && v < 1.0, "{v}");
    }
}

#[test]
fn length_limits_never_break_formats() {
    let email = json!({ "type": "string", "format": "email", "maxLength": 8 });
    let uuid = json!({ "type": "string", "format": "uuid", "minLength": 50 });
    let by_name = json!({ "type": "object", "properties": {
        "userEmail": { "type": "string", "minLength": 60 } }});
    for seed in 0..20 {
        assert!(
            generate(&email, None, seed)
                .as_str()
                .unwrap()
                .ends_with("@example.com")
        );
        assert_eq!(generate(&uuid, None, seed).as_str().unwrap().len(), 36);
        let v = generate(&by_name, None, seed);
        assert!(v["userEmail"].as_str().unwrap().ends_with("@example.com"));
    }
}

#[test]
fn city_state_pincode_and_address_describe_the_same_place() {
    let s = json!({ "type": "object", "properties": {
        "city": { "type": "string" }, "state": { "type": "string" },
        "pincode": { "type": "string" }, "address": { "type": "string" }
    }});
    let mut by_city = std::collections::HashMap::new();
    for seed in 0..300 {
        let v = generate(&s, None, seed);
        let city = v["city"].as_str().unwrap().to_string();
        let place = (v["state"].clone(), v["pincode"].clone());
        // one city always means the same state and pincode
        assert_eq!(
            by_city.entry(city.clone()).or_insert(place.clone()),
            &place,
            "{v}"
        );
        assert!(v["address"].as_str().unwrap().ends_with(&city), "{v}");
    }
    assert!(by_city.len() > 3, "cities should still vary: {by_city:?}");
}
