//! Formats, examples, field-name heuristics, urls, params, odd schemas and bounds.

use crate::common::*;
use kaanal_data::{Options, apply_params, generate};
use serde_json::json;

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
fn array_items_use_the_array_field_name_and_a_map_is_never_empty() {
    // petstore `photoUrls`: plural names reach the url generator through the array.
    let s = json!({ "type": "array", "items": { "type": "string" }, "minItems": 2 });
    let v = generate(&s, Some("photoUrls"), 5, &Options::default());
    assert!(
        v.as_array()
            .unwrap()
            .iter()
            .all(|u| u.as_str().unwrap().starts_with("http")),
        "{v}"
    );
    // petstore `/store/inventory`: `additionalProperties` with a schema gives 1 to 3 keys.
    let m = json!({ "type": "object", "additionalProperties": { "type": "integer" } });
    for seed in 0..200 {
        let n = g1(&m, seed).as_object().unwrap().len();
        assert!((1..=3).contains(&n), "seed {seed}: {n} keys");
    }
}
