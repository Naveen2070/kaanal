//! Floats, integers, counts, bounds, lat/lng and length limits.

use kaanal_data::{Options, generate};
use serde_json::json;

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
    use rand::{RngExt, SeedableRng};
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
