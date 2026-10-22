use kaanal_data::{Options, apply_params, generate};
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

#[test]
fn formats_look_right() {
    let v = generate(&rich_schema(), None, 3, &Options::default());
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
        generate(
            &json!({ "type": "string", "example": "hello" }),
            None,
            1,
            &Options::default()
        ),
        json!("hello")
    );
    let e = generate(
        &json!({ "type": "string", "enum": ["x"] }),
        None,
        1,
        &Options::default(),
    );
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
        let v = generate(&s, None, seed, &Options::default());
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
        "url": { "type": "string" }, "avatarUrl": { "type": "string" }, "urlUrl": { "type": "string" }
    }});
    for seed in 0..20 {
        let v = generate(&s, None, seed, &Options::default());
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
            v["urlUrl"]
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
    assert!(generate(&json!({}), None, 1, &Options::default()).is_string());
    let v = generate(
        &json!({ "type": "array", "minItems": 0, "maxItems": 0 }),
        None,
        1,
        &Options::default(),
    );
    assert_eq!(v, json!([]));
    let _ = generate(
        &json!({ "type": "integer", "minimum": 5, "maximum": 1 }),
        None,
        1,
        &Options::default(),
    );
}

#[test]
fn huge_max_items_stays_small() {
    let s = json!({ "type": "array", "items": { "type": "integer" }, "maxItems": 1_000_000 });
    for seed in 0..20 {
        assert!(
            generate(&s, None, seed, &Options::default())
                .as_array()
                .unwrap()
                .len()
                <= 20
        );
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
        let v = generate(&s, None, seed, &Options::default());
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
        .map(|s| generate(&int, None, s, &Options::default()).to_string())
        .collect();
    let nums: std::collections::HashSet<_> = (0..20)
        .map(|s| generate(&num, None, s, &Options::default()).to_string())
        .collect();
    assert!(ints.len() > 1 && nums.len() > 1);
    assert!((0..20).all(|s| {
        generate(&int, None, s, &Options::default())
            .as_i64()
            .unwrap()
            >= 5000
    }));
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
        let (a, b) = (
            generate(&before, None, seed, &Options::default()),
            generate(&after, None, seed, &Options::default()),
        );
        for key in ["firstName", "city", "score"] {
            assert_eq!(a[key], b[key], "seed {seed} key {key}");
        }
    }
}

#[test]
fn exclusive_bounds_are_strict() {
    let int = json!({ "type": "integer", "minimum": 1, "maximum": 3,
                      "exclusiveMinimum": true, "exclusiveMaximum": true });
    assert!((0..50).all(|s| generate(&int, None, s, &Options::default()) == json!(2)));
    let num = json!({ "type": "number", "minimum": 0.0, "maximum": 1.0,
                      "exclusiveMinimum": true, "exclusiveMaximum": true });
    for seed in 0..200 {
        let v = generate(&num, None, seed, &Options::default())
            .as_f64()
            .unwrap();
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
        assert!(
            check.is_valid(&generate(&whole, None, seed, &Options::default())),
            "seed {seed}"
        );
        let f = generate(&frac, None, seed, &Options::default())
            .as_i64()
            .unwrap();
        assert!((2..=3).contains(&f), "{f}");
        assert_eq!(generate(&excl, None, seed, &Options::default()), json!(3));
        assert!(huge_ok.is_valid(&generate(&huge, None, seed, &Options::default())));
    }
    // A lone maximum below the default range moves the range instead of pinning to it.
    let lone = json!({ "type": "integer", "maximum": -5 });
    let seen: std::collections::BTreeSet<i64> = (0..200)
        .map(|seed| {
            generate(&lone, None, seed, &Options::default())
                .as_i64()
                .unwrap()
        })
        .collect();
    assert!(seen.len() > 1 && seen.iter().all(|v| *v <= -5), "{seen:?}");
    // A minimum past i64::MAX saturates instead of being ignored or panicking.
    let past = json!({ "type": "integer", "minimum": u64::MAX });
    assert_eq!(
        generate(&past, None, 1, &Options::default()),
        json!(i64::MAX)
    );
}

#[test]
fn counts_accept_whole_number_floats_and_max_items_zero_is_empty() {
    assert_eq!(
        generate(
            &json!({ "type": "array", "maxItems": 0 }),
            None,
            1,
            &Options::default()
        ),
        json!([])
    );
    assert_eq!(
        generate(
            &json!({ "type": "array", "maxItems": 0.0 }),
            None,
            1,
            &Options::default()
        ),
        json!([])
    );
    let arr = json!({ "type": "array", "items": { "type": "integer" }, "minItems": 2.0, "maxItems": 3.0 });
    let s = json!({ "type": "string", "format": "password", "minLength": 12.0, "maxLength": 14.0 });
    for seed in 0..50 {
        let n = generate(&arr, None, seed, &Options::default())
            .as_array()
            .unwrap()
            .len();
        assert!((2..=3).contains(&n), "{n}");
        let n = generate(&s, None, seed, &Options::default())
            .as_str()
            .unwrap()
            .chars()
            .count();
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
        let b = generate(&price, None, seed, &Options::default());
        assert!(v.is_valid(&b), "{b}");
        seen.insert(b["price"].to_string());
    }
    assert!(seen.len() > 1, "price should vary: {seen:?}");
    // An infinite span must not panic; the value stays finite and in range.
    let wide = json!({ "type": "number", "minimum": -1e308, "maximum": 1e308 });
    let v = jsonschema::validator_for(&wide).unwrap();
    assert!(v.is_valid(&generate(&wide, None, 1, &Options::default())));
    // An exclusive range narrower than a cent gives the exact midpoint.
    let narrow = json!({ "type": "number", "minimum": 0.0, "maximum": 0.015,
                         "exclusiveMinimum": true, "exclusiveMaximum": true });
    assert_eq!(
        generate(&narrow, None, 1, &Options::default()),
        json!(0.0075)
    );
}

#[test]
fn lat_lng_get_bounds_and_unbounded_output_is_unchanged() {
    use rand::{Rng, SeedableRng};
    let round6 = |v: f64| (v * 1e6).round() / 1e6;
    for seed in 0..50 {
        // Bit-identical to the old fixed-range code: same draw, same rounding.
        let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(seed);
        let lat = generate(
            &json!({ "type": "number" }),
            Some("lat"),
            seed,
            &Options::default(),
        );
        assert_eq!(lat, json!(round6(rng.random_range(8.0..=35.0))));
        let mut rng = rand_chacha::ChaCha8Rng::seed_from_u64(seed);
        let lng = generate(
            &json!({ "type": "number" }),
            Some("longitude"),
            seed,
            &Options::default(),
        );
        assert_eq!(lng, json!(round6(rng.random_range(68.0..=97.0))));
    }
    let bounded = json!({ "type": "object", "properties": {
        "lat": { "type": "number", "minimum": 10, "maximum": 12 },
        "lng": { "type": "number", "minimum": -180, "maximum": -170 } } });
    let v = jsonschema::validator_for(&bounded).unwrap();
    for seed in 0..50 {
        let b = generate(&bounded, None, seed, &Options::default());
        assert!(v.is_valid(&b), "{b}");
        let lat = b["lat"].as_f64().unwrap();
        assert_eq!(lat, round6(lat), "six places kept: {lat}");
    }
}

#[test]
fn min_length_pads_in_one_pass_and_is_capped() {
    let s = json!({ "type": "string", "minLength": 40, "maxLength": 50 });
    let v = jsonschema::validator_for(&s).unwrap();
    assert!((0..50).all(|seed| v.is_valid(&generate(&s, None, seed, &Options::default()))));
    // A huge minLength is capped at 10 000 characters (R6.10d) and returns at once.
    let huge = json!({ "type": "string", "minLength": 1_000_000_000_u64 });
    let t = std::time::Instant::now();
    assert_eq!(
        generate(&huge, None, 1, &Options::default())
            .as_str()
            .unwrap()
            .chars()
            .count(),
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
            generate(&email, None, seed, &Options::default())
                .as_str()
                .unwrap()
                .ends_with("@example.com")
        );
        assert_eq!(
            generate(&uuid, None, seed, &Options::default())
                .as_str()
                .unwrap()
                .len(),
            36
        );
        let v = generate(&by_name, None, seed, &Options::default());
        assert!(v["userEmail"].as_str().unwrap().ends_with("@example.com"));
    }
}

#[test]
fn city_state_pincode_and_address_describe_the_same_place() {
    // the original ten cities keep their explicit pin; every other city keeps a 3-digit prefix
    const FIXED: &[(&str, &str)] = &[
        ("Chennai", "600001"),
        ("Coimbatore", "641001"),
        ("Madurai", "625001"),
        ("Bengaluru", "560001"),
        ("Mumbai", "400001"),
        ("Pune", "411001"),
        ("Hyderabad", "500001"),
        ("Kochi", "682001"),
        ("Delhi", "110001"),
        ("Kolkata", "700001"),
    ];
    let s = json!({ "type": "object", "properties": {
        "city": { "type": "string" }, "state": { "type": "string" },
        "pincode": { "type": "string" }, "address": { "type": "string" }
    }});
    let mut by_city = std::collections::HashMap::new();
    for seed in 0..2000 {
        let v = generate(&s, None, seed, &Options::default());
        let city = v["city"].as_str().unwrap().to_string();
        let pin = v["pincode"].as_str().unwrap();
        assert!(
            pin.len() == 6 && pin.bytes().all(|b| b.is_ascii_digit()) && !pin.starts_with('0'),
            "{v}"
        );
        if let Some((_, fixed)) = FIXED.iter().find(|(c, _)| *c == city) {
            assert_eq!(pin, *fixed, "{v}");
        }
        let place = (v["state"].clone(), pin[..3].to_string());
        assert_eq!(
            by_city.entry(city.clone()).or_insert(place.clone()),
            &place,
            "{v}"
        );
        assert!(v["address"].as_str().unwrap().ends_with(&city), "{v}");
    }
    assert!(
        by_city.len() > 60,
        "cities should still vary: {}",
        by_city.len()
    );
}

#[test]
fn email_of_two_non_latin_names_is_one_user_n() {
    use std::sync::Arc;
    let src = "version: 1\ntag: xx-XX\nname_order: given-family\ncountry: X\n\
         currency: { code: XXX, symbol: x }\nphone: { prefix: \"+0\", formats: [\"{3}\"] }\n\
         address: \"{number} {street}\"\nfirst_names: [田中]\nlast_names: [鈴木]\n\
         streets: [S]\nplaces: [{ city: C, postcode: \"{3}\" }]\n";
    let opts = Options {
        pack: Arc::new(kaanal_data::Pack::parse(src).unwrap()),
        ..Options::default()
    };
    let s = json!({ "type": "string", "format": "email" });
    for seed in 0..50 {
        let v = generate(&s, None, seed, &opts);
        let local = v.as_str().unwrap().strip_suffix("@example.com").unwrap();
        assert!(
            local
                .strip_prefix("user")
                .is_some_and(|d| !d.is_empty() && d.bytes().all(|b| b.is_ascii_digit())),
            "{v}"
        );
    }
}

#[test]
fn names_and_email_describe_the_same_person() {
    let s = json!({ "type": "object", "properties": {
        "firstName": { "type": "string" }, "lastName": { "type": "string" },
        "name": { "type": "string" }, "email": { "type": "string", "format": "email" }
    }});
    let mut people = std::collections::HashSet::new();
    for seed in 0..300 {
        let v = generate(&s, None, seed, &Options::default());
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
        let v = generate(&s, None, seed, &Options::default());
        let n = v.as_str().unwrap().chars().count();
        assert!((12..=16).contains(&n), "{v}");
    }
}

#[test]
fn huge_or_deeply_nested_arrays_are_bounded() {
    // One array asking for a billion items.
    let s =
        json!({ "type": "array", "minItems": 1_000_000_000_u64, "items": { "type": "integer" } });
    let v = generate(&s, None, 1, &Options::default());
    assert!(v.as_array().unwrap().len() <= 10_000);

    // Five nested levels of 20 items would be 3.2M values.
    let mut s = json!({ "type": "integer" });
    for _ in 0..5 {
        s = json!({ "type": "array", "minItems": 20, "maxItems": 20, "items": s });
    }
    let text = generate(&s, None, 1, &Options::default()).to_string();
    assert!(text.len() < 1_000_000, "body is {} bytes", text.len());
}

#[test]
fn fit_length_counts_characters_not_bytes() {
    use std::sync::Arc;
    // `firstName` is the pack's name verbatim, so it exercises `fit_length` on non-ASCII text.
    for (name, max, want) in [
        ("田中太郎花子", 4, "田中太郎"),
        ("😀😀😀😀😀", 4, "😀😀😀😀"),
        ("e\u{301}e\u{301}e\u{301}", 4, "e\u{301}e\u{301}"),
        ("田中太郎", 4, "田中太郎"),
    ] {
        let src = format!(
            "version: 1\ntag: xx-XX\nname_order: given-family\ncountry: X\n\
             currency: {{ code: XXX, symbol: x }}\nphone: {{ prefix: \"+0\", formats: [\"{{3}}\"] }}\n\
             address: \"{{number}} {{street}}\"\nfirst_names: [\"{name}\"]\nlast_names: [Z]\n\
             streets: [S]\nplaces: [{{ city: C, postcode: \"{{3}}\" }}]\n"
        );
        let opts = Options {
            pack: Arc::new(kaanal_data::Pack::parse(&src).unwrap()),
            ..Options::default()
        };
        let s = json!({ "type": "string", "maxLength": max });
        assert_eq!(generate(&s, Some("firstName"), 1, &opts), want, "{name}");
        // padding counts characters too
        let s = json!({ "type": "string", "minLength": 6 });
        let v = generate(&s, Some("firstName"), 1, &opts);
        assert_eq!(v.as_str().unwrap().chars().count(), 6, "{v}");
    }
}

fn g1(s: &Value, seed: u64) -> Value {
    generate(s, None, seed, &Options::default())
}

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

/// A pet API whose response is `oneOf: [Dog, Cat]` by `$ref`, with `disc` (indented YAML
/// lines under the schema, or empty) next to the `oneOf`.
fn pets(disc: &str) -> kaanal_data::ApiModel {
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

const MAPPING: &str = r##"                discriminator:
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
