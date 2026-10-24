//! Cities, addresses, emails, names, unknown formats and `fit_length`.

use kaanal_data::{Options, generate};
use serde_json::json;

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
        "pincode": { "type": "string" }, "address": { "type": "string" },
        "street": { "type": "string" }
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
        // `street` and the address's `{street}` are one choice of the object
        assert!(
            v["address"]
                .as_str()
                .unwrap()
                .contains(v["street"].as_str().unwrap()),
            "{v}"
        );
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
