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
fn allof_merges_non_object_parts() {
    // R6.10a: the schemas merge, so a constraint in another part applies to the value.
    let s = json!({ "allOf": [{ "type": "string" }, { "maxLength": 5 }] });
    let v = jsonschema::validator_for(&s).unwrap();
    for seed in 0..50 {
        let b = generate(&s, Some("note"), seed);
        assert!(v.is_valid(&b) && b.is_string(), "{b}");
    }
    // A `$ref` to an enum plus a description part (refs are inlined before generation).
    let s = json!({ "allOf": [
        { "type": "string", "enum": ["red", "green"] }, { "description": "colour" }
    ] });
    for seed in 0..20 {
        let b = generate(&s, None, seed);
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
        let b = generate(&s, None, seed);
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
        let b = generate(&s, None, seed);
        assert!(v.is_valid(&b), "{b}");
        assert!(b.get("password").is_none() && b["note"].is_string(), "{b}");
    }
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
        assert_ne!(v["city"].as_str().unwrap(), "");
        assert_eq!(v["currency"], "INR");
    }
}

#[test]
fn url_fields_have_no_empty_path_segment() {
    let s = json!({ "type": "object", "properties": {
        "url": { "type": "string" }, "avatarUrl": { "type": "string" }, "urlurl": { "type": "string" }
    }});
    for seed in 0..20 {
        let v = generate(&s, None, seed);
        let bare = v["url"].as_str().unwrap();
        let rest = bare.strip_prefix("https://example.com/").unwrap();
        assert!(
            !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit()),
            "{bare}"
        );
        assert!(
            v["avatarUrl"]
                .as_str()
                .unwrap()
                .starts_with("https://example.com/avatar/")
        );
        // only one `url` suffix is stripped
        assert!(
            v["urlurl"]
                .as_str()
                .unwrap()
                .starts_with("https://example.com/url/")
        );
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

// Reordering is not tested: `properties` is a BTreeMap (no `preserve_order`), so key order
// is unobservable. It would become observable if `preserve_order` were enabled (R4.4).
#[test]
fn adding_properties_keeps_other_values() {
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
fn whole_number_float_and_fractional_integer_bounds_are_honoured() {
    // R6.10c: `0.0`/`100.0` count as integers; fractional bounds round inward.
    let whole = json!({ "type": "integer", "minimum": 0.0, "maximum": 100.0 });
    let frac = json!({ "type": "integer", "minimum": 1.5, "maximum": 3.5 });
    let excl = json!({ "type": "integer", "minimum": 2.0, "maximum": 4.0,
                       "exclusiveMinimum": true, "exclusiveMaximum": true });
    let huge = json!({ "type": "integer", "maximum": u64::MAX });
    let check =
        jsonschema::validator_for(&json!({ "type": "integer", "minimum": 0, "maximum": 100 }))
            .unwrap();
    let huge_ok = jsonschema::validator_for(&huge).unwrap();
    for seed in 0..200 {
        assert!(check.is_valid(&generate(&whole, None, seed)), "seed {seed}");
        let f = generate(&frac, None, seed).as_i64().unwrap();
        assert!((2..=3).contains(&f), "{f}");
        assert_eq!(generate(&excl, None, seed), json!(3));
        assert!(huge_ok.is_valid(&generate(&huge, None, seed)));
    }
    // A lone maximum below the default range moves the range instead of pinning to it.
    let lone = json!({ "type": "integer", "maximum": -5 });
    let seen: std::collections::BTreeSet<i64> = (0..200)
        .map(|seed| generate(&lone, None, seed).as_i64().unwrap())
        .collect();
    assert!(seen.len() > 1 && seen.iter().all(|v| *v <= -5), "{seen:?}");
    // A minimum past i64::MAX saturates instead of being ignored or panicking.
    let past = json!({ "type": "integer", "minimum": u64::MAX });
    assert_eq!(generate(&past, None, 1), json!(i64::MAX));
}

#[test]
fn counts_accept_whole_number_floats_and_max_items_zero_is_empty() {
    assert_eq!(
        generate(&json!({ "type": "array", "maxItems": 0 }), None, 1),
        json!([])
    );
    assert_eq!(
        generate(&json!({ "type": "array", "maxItems": 0.0 }), None, 1),
        json!([])
    );
    let arr = json!({ "type": "array", "items": { "type": "integer" }, "minItems": 2.0, "maxItems": 3.0 });
    let s = json!({ "type": "string", "format": "password", "minLength": 12.0, "maxLength": 14.0 });
    for seed in 0..50 {
        let n = generate(&arr, None, seed).as_array().unwrap().len();
        assert!((2..=3).contains(&n), "{n}");
        let n = generate(&s, None, seed).as_str().unwrap().chars().count();
        assert!((12..=14).contains(&n), "{n}");
    }
}

#[test]
fn number_bounds_are_honoured_at_the_edges() {
    // A lone maximum below the default money range moves the range instead of pinning to it.
    let price =
        json!({ "type": "object", "properties": { "price": { "type": "number", "maximum": 50 } } });
    let v = jsonschema::validator_for(&price).unwrap();
    let mut seen = std::collections::HashSet::new();
    for seed in 0..50 {
        let b = generate(&price, None, seed);
        assert!(v.is_valid(&b), "{b}");
        seen.insert(b["price"].to_string());
    }
    assert!(seen.len() > 1, "price should vary: {seen:?}");
    // An infinite span must not panic; the value stays finite and in range.
    let wide = json!({ "type": "number", "minimum": -1e308, "maximum": 1e308 });
    let v = jsonschema::validator_for(&wide).unwrap();
    assert!(v.is_valid(&generate(&wide, None, 1)));
    // An exclusive range narrower than a cent gives the exact midpoint.
    let narrow = json!({ "type": "number", "minimum": 0.0, "maximum": 0.015,
                         "exclusiveMinimum": true, "exclusiveMaximum": true });
    assert_eq!(generate(&narrow, None, 1), json!(0.0075));
}

#[test]
fn lat_lng_get_bounds_and_unbounded_output_is_unchanged() {
    use rand::{Rng, SeedableRng};
    let round6 = |v: f64| (v * 1e6).round() / 1e6;
    for seed in 0..50 {
        // Bit-identical to the old fixed-range code: same draw, same rounding.
        let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(seed);
        let lat = generate(&json!({ "type": "number" }), Some("lat"), seed);
        assert_eq!(lat, json!(round6(rng.random_range(8.0..=35.0))));
        let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(seed);
        let lng = generate(&json!({ "type": "number" }), Some("longitude"), seed);
        assert_eq!(lng, json!(round6(rng.random_range(68.0..=97.0))));
    }
    let bounded = json!({ "type": "object", "properties": {
        "lat": { "type": "number", "minimum": 10, "maximum": 12 },
        "lng": { "type": "number", "minimum": -180, "maximum": -170 } } });
    let v = jsonschema::validator_for(&bounded).unwrap();
    for seed in 0..50 {
        let b = generate(&bounded, None, seed);
        assert!(v.is_valid(&b), "{b}");
        let lat = b["lat"].as_f64().unwrap();
        assert_eq!(lat, round6(lat), "six places kept: {lat}");
    }
}

#[test]
fn min_length_pads_in_one_pass_and_is_capped() {
    let s = json!({ "type": "string", "minLength": 40, "maxLength": 50 });
    let v = jsonschema::validator_for(&s).unwrap();
    assert!((0..50).all(|seed| v.is_valid(&generate(&s, None, seed))));
    // A huge minLength is capped at 10 000 characters (R6.10d) and returns at once.
    let huge = json!({ "type": "string", "minLength": 1_000_000_000_u64 });
    let t = std::time::Instant::now();
    assert_eq!(
        generate(&huge, None, 1).as_str().unwrap().chars().count(),
        10_000
    );
    assert!(t.elapsed() < std::time::Duration::from_secs(1));
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

#[test]
fn names_and_email_describe_the_same_person() {
    let s = json!({ "type": "object", "properties": {
        "firstName": { "type": "string" }, "lastName": { "type": "string" },
        "name": { "type": "string" }, "email": { "type": "string", "format": "email" }
    }});
    let mut people = std::collections::HashSet::new();
    for seed in 0..300 {
        let v = generate(&s, None, seed);
        let (first, last) = (
            v["firstName"].as_str().unwrap(),
            v["lastName"].as_str().unwrap(),
        );
        assert_eq!(v["name"], format!("{first} {last}"), "{v}");
        let prefix = format!("{}.{}", first.to_lowercase(), last.to_lowercase());
        assert!(v["email"].as_str().unwrap().starts_with(&prefix), "{v}");
        people.insert(format!("{first} {last}"));
    }
    assert!(people.len() > 3, "people should still vary: {people:?}");
}

#[test]
fn unknown_formats_still_get_length_limits() {
    let s = json!({ "type": "string", "format": "password", "minLength": 12, "maxLength": 16 });
    for seed in 0..50 {
        let v = generate(&s, None, seed);
        let n = v.as_str().unwrap().chars().count();
        assert!((12..=16).contains(&n), "{v}");
    }
}

#[test]
fn huge_or_deeply_nested_arrays_are_bounded() {
    // One array asking for a billion items.
    let s =
        json!({ "type": "array", "minItems": 1_000_000_000_u64, "items": { "type": "integer" } });
    let v = generate(&s, None, 1);
    assert!(v.as_array().unwrap().len() <= 10_000);

    // Five nested levels of 20 items would be 3.2M values.
    let mut s = json!({ "type": "integer" });
    for _ in 0..5 {
        s = json!({ "type": "array", "minItems": 20, "maxItems": 20, "items": s });
    }
    let text = generate(&s, None, 1).to_string();
    assert!(text.len() < 1_000_000, "body is {} bytes", text.len());
}
