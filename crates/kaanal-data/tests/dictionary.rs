use std::sync::Arc;

use kaanal_data::{ApiModel, Dictionary, Endpoint, Options, dictionary_warnings, generate};
use serde_json::{Value, json};

/// Options with `src` as the project dictionary over the built-ins.
fn with_project(src: &str) -> Options {
    Options {
        dictionary_project: Some(Arc::new(Dictionary::parse(src).unwrap())),
        ..Options::default()
    }
}

fn gen_field(s: &Value, field: &str, seed: u64, opts: &Options) -> Value {
    let obj = json!({ "type": "object", "properties": { field: s } });
    generate(&obj, None, seed, opts)[field].clone()
}

#[test]
fn normalisation_makes_camel_snake_kebab_share_a_key() {
    let opts = with_project("version: 1\nfields:\n  user_email: { gen: static, value: X }\n");
    let s = json!({ "type": "string" });
    for f in [
        "userEmail",
        "user_email",
        "UserEmail",
        "user-email",
        "USER_EMAIL",
    ] {
        assert_eq!(gen_field(&s, f, 1, &opts), "X", "{f}");
    }
    let d = Dictionary::parse("fields:\n  First-Name: { gen: first_name }\n").unwrap();
    assert!(d.contains("firstName") && d.contains("first_name") && !d.contains("first"));
}

#[test]
fn exact_key_beats_suffix_and_longer_suffix_beats_shorter() {
    let opts = with_project(
        "fields:\n\
         \x20 \"*id\": { gen: static, value: short }\n\
         \x20 \"*order_id\": { gen: static, value: long }\n\
         \x20 parentOrderId: { gen: static, value: exact }\n",
    );
    let s = json!({ "type": "string" });
    assert_eq!(gen_field(&s, "userId", 1, &opts), "short");
    assert_eq!(gen_field(&s, "orderId", 1, &opts), "long");
    assert_eq!(gen_field(&s, "lastOrderId", 1, &opts), "long");
    assert_eq!(gen_field(&s, "parentOrderId", 1, &opts), "exact");
    // a suffix matches whole words only: `paid` and `valid` are not ids
    assert_ne!(gen_field(&s, "paid", 1, &opts), "short");
    assert_ne!(gen_field(&s, "valid", 1, &opts), "short");
}

#[test]
fn unknown_generator_error_lists_valid_ones_with_line() {
    let e = Dictionary::parse("version: 1\nfields:\n  sku: { gen: barcode }\n").unwrap_err();
    assert_eq!(e.line, Some(3), "{e}");
    for g in ["first_name", "money", "pick", "template", "static"] {
        assert!(e.message.contains(g), "{e}");
    }
    let e = Dictionary::parse("fields:\n  price: { gen: money, mn: 1 }\n").unwrap_err();
    assert_eq!(e.line, Some(2), "{e}");
    assert!(
        e.message.contains("min") && e.message.contains("max"),
        "{e}"
    );
    let e = Dictionary::parse("fields:\n  a: { gen: email, extra: 1 }\n").unwrap_err();
    assert!(e.message.contains("extra"), "{e}");
    for bad in [
        "fields:\n  a: { gen: pick, from: [] }\n",
        "fields:\n  a: { gen: template, value: \"X-{digits:0}\" }\n",
        "fields:\n  a: { gen: template, value: \"X-{oops}\" }\n",
        "fields:\n  a: { gen: integer, min: 5, max: 1 }\n",
        "fields:\n  \"*\": { gen: email }\n",
        "version: 2\nfields: {}\n",
    ] {
        assert!(Dictionary::parse(bad).is_err(), "{bad}");
    }
}

#[test]
fn project_entry_beats_format_but_not_enum_or_example() {
    let opts = with_project("fields:\n  contact: { gen: static, value: proj }\n");
    let fmt = json!({ "type": "string", "format": "email" });
    assert_eq!(gen_field(&fmt, "contact", 1, &opts), "proj");
    let en = json!({ "type": "string", "enum": ["e"] });
    assert_eq!(gen_field(&en, "contact", 1, &opts), "e");
    let ex = json!({ "type": "string", "example": "x" });
    assert_eq!(gen_field(&ex, "contact", 1, &opts), "x");
}

#[test]
fn format_beats_the_built_in_name_entry() {
    let s = json!({ "type": "string", "format": "uuid" });
    for seed in 0..20 {
        let v = gen_field(&s, "email", seed, &Options::default());
        let v = v.as_str().unwrap();
        assert!(v.len() == 36 && !v.contains('@'), "{v}");
    }
}

#[test]
fn misfit_entry_is_skipped_with_a_warning_naming_the_field() {
    let opts = with_project("fields:\n  level: { gen: pick, from: [low, high] }\n");
    let s = json!({ "type": "object", "properties": {
        "level": { "type": "integer", "minimum": 1, "maximum": 3 },
        "tier": { "type": "string" }
    }});
    for seed in 0..20 {
        let v = generate(&s, None, seed, &opts);
        assert!((1..=3).contains(&v["level"].as_i64().unwrap()), "{v}");
    }
    let model = ApiModel {
        endpoints: vec![Endpoint {
            method: "GET".into(),
            path: "/x".into(),
            status: 200,
            schema: Some(s),
        }],
    };
    let w = dictionary_warnings(&model, &opts);
    assert_eq!(w.len(), 1, "{w:?}");
    assert!(
        w[0].contains("GET /x") && w[0].contains("level") && w[0].contains("integer"),
        "{w:?}"
    );
    // the built-ins and fitting entries are quiet
    assert_eq!(
        dictionary_warnings(&model, &Options::default()),
        Vec::<String>::new()
    );
}

#[test]
fn misfit_warnings_follow_what_the_generator_visits() {
    let opts = with_project(
        "fields:
  level: { gen: pick, from: [low, high] }
",
    );
    let int = json!({ "type": "integer" });
    let warned = |level: Value| {
        let s = json!({ "type": "object", "properties": { "level": level } });
        let model = ApiModel {
            endpoints: vec![Endpoint {
                method: "GET".into(),
                path: "/x".into(),
                status: 200,
                schema: Some(s),
            }],
        };
        !dictionary_warnings(&model, &opts).is_empty()
    };
    // `examples` wins in the generator, so the entry is never tried; an empty one does not
    assert!(!warned(json!({ "type": "integer", "examples": [2] })));
    assert!(!warned(
        json!({ "type": "integer", "examples": { "a": { "value": 2 } } })
    ));
    assert!(warned(json!({ "type": "integer", "examples": [] })));
    assert!(warned(json!({ "type": "integer", "enum": [] })));
    // only the first non-empty of `oneOf`/`anyOf` is a branch source
    assert!(!warned(
        json!({ "oneOf": [{ "type": "string" }], "anyOf": [int] })
    ));
    assert!(warned(json!({ "oneOf": [], "anyOf": [int] })));
    assert!(warned(json!({ "oneOf": [{ "type": "string" }, int] })));
}

#[test]
fn money_clamps_to_schema_bounds() {
    let opts = with_project("fields:\n  fee: { gen: money, min: 1000, max: 5000 }\n");
    let s = json!({ "type": "number", "minimum": 10, "maximum": 20 });
    let i = json!({ "type": "integer", "minimum": 10, "maximum": 20 });
    for seed in 0..50 {
        let v = gen_field(&s, "fee", seed, &opts).as_f64().unwrap();
        assert!((10.0..=20.0).contains(&v), "{v}");
        let v = gen_field(&i, "fee", seed, &opts).as_i64().unwrap();
        assert!((10..=20).contains(&v), "{v}");
        // without bounds the entry's range is used
        let v = gen_field(&json!({ "type": "number" }), "fee", seed, &opts)
            .as_f64()
            .unwrap();
        assert!((1000.0..=5000.0).contains(&v), "{v}");
    }
}

#[test]
fn text_trims_to_max_length_in_characters() {
    let opts = with_project("fields:\n  note: { gen: text, kind: paragraph }\n");
    for seed in 0..20 {
        let v = gen_field(
            &json!({ "type": "string", "maxLength": 10 }),
            "note",
            seed,
            &opts,
        );
        assert!(v.as_str().unwrap().chars().count() <= 10, "{v}");
        let v = gen_field(&json!({ "type": "string" }), "note", seed, &opts);
        assert!(v.as_str().unwrap().chars().count() > 40, "paragraph: {v}");
    }
}

/// One-endpoint model around an object schema, for `dictionary_warnings`.
fn model_of(s: Value) -> ApiModel {
    ApiModel {
        endpoints: vec![Endpoint {
            method: "GET".into(),
            path: "/u".into(),
            status: 200,
            schema: Some(s),
        }],
    }
}

#[test]
fn user_strings_out_of_length_bounds_are_emitted_whole_and_warned() {
    let opts = with_project(
        "fields:\n\
         \x20 motto: { gen: static, value: \"日本語のテキスト\" }\n\
         \x20 tier: { gen: pick, from: [gold, platinum] }\n\
         \x20 code: { gen: template, value: \"SKU-{digits:6}\" }\n\
         \x20 pad: { gen: static, value: ab }\n\
         \x20 fine: { gen: template, value: \"{letters:3}\" }\n",
    );
    let s = json!({ "type": "object", "properties": {
        "motto": { "type": "string", "maxLength": 3 },
        "tier": { "type": "string", "maxLength": 4 },
        "code": { "type": "string", "maxLength": 4 },
        "pad": { "type": "string", "minLength": 5 },
        "fine": { "type": "string", "minLength": 3, "maxLength": 3 }
    }});
    let mut tiers = std::collections::BTreeSet::new();
    for seed in 0..40 {
        let v = generate(&s, None, seed, &opts);
        assert_eq!(v["motto"], "日本語のテキスト");
        assert_eq!(v["pad"], "ab");
        assert_eq!(v["code"].as_str().unwrap().len(), 10, "{v}");
        tiers.insert(v["tier"].as_str().unwrap().to_string());
    }
    assert_eq!(tiers.len(), 2, "{tiers:?}");
    let w = dictionary_warnings(&model_of(s), &opts);
    assert_eq!(w.len(), 4, "{w:?}");
    for f in ["motto", "tier", "code", "pad"] {
        assert!(
            w.iter()
                .any(|m| m.contains("GET /u") && m.contains(&format!("`{f}`"))),
            "{f}: {w:?}"
        );
    }
}

#[test]
fn generated_and_built_in_strings_still_fit_their_lengths() {
    let s = json!({ "type": "object", "properties": {
        "firstName": { "type": "string", "maxLength": 2 },
        "sku": { "type": "string", "maxLength": 5 },
        "role": { "type": "string", "minLength": 9 },
        "phone": { "type": "string", "maxLength": 3 },
        "email": { "type": "string", "maxLength": 3 }
    }});
    for seed in 0..40 {
        let v = generate(&s, None, seed, &Options::default());
        assert!(v["firstName"].as_str().unwrap().chars().count() <= 2, "{v}");
        // a built-in entry that cannot meet the bounds is skipped, never cut or padded
        let sku = v["sku"].as_str().unwrap();
        assert!(sku.chars().count() <= 5 && !sku.starts_with("SKU-"), "{v}");
        let role = v["role"].as_str().unwrap();
        assert!(role.chars().count() >= 9, "{v}");
        assert!(
            !role.starts_with("admin") && !role.starts_with("editor"),
            "{v}"
        );
        // well-formed values ignore lengths (R4.5)
        assert!(v["phone"].as_str().unwrap().len() > 3, "{v}");
        assert!(
            v["email"].as_str().unwrap().ends_with("@example.com"),
            "{v}"
        );
    }
    // built-ins never warn
    assert_eq!(
        dictionary_warnings(&model_of(s), &Options::default()),
        Vec::<String>::new()
    );
}

#[test]
fn bare_total_is_a_count_and_star_total_is_money() {
    let o = Options::default();
    let mut seen_money = false;
    for seed in 0..60 {
        let t = gen_field(&json!({ "type": "integer" }), "total", seed, &o);
        assert!((0..=1000).contains(&t.as_i64().unwrap()), "{t}");
        let t = gen_field(&json!({ "type": "number" }), "total", seed, &o);
        assert!(
            t.is_i64() && (0..=1000).contains(&t.as_i64().unwrap()),
            "{t}"
        );
        for f in ["orderTotal", "grandTotal", "subtotal", "cartSubtotal"] {
            let v = gen_field(&json!({ "type": "number" }), f, seed, &o)
                .as_f64()
                .unwrap();
            assert!((99.0..=4999.0).contains(&v), "{f}: {v}");
            seen_money |= v.fract() != 0.0;
        }
    }
    assert!(seen_money, "money has cents");
}

#[test]
fn pick_template_static_values_pass_through() {
    let opts = with_project(
        "fields:\n\
         \x20 sku: { gen: template, value: \"SKU-{digits:6}\" }\n\
         \x20 state: { gen: pick, from: [paid, refunded, pending] }\n\
         \x20 owner: { gen: static, value: \"Ünïcode Name\" }\n\
         \x20 level: { gen: pick, from: [1, 2, 3] }\n",
    );
    let s = json!({ "type": "object", "properties": {
        "sku": { "type": "string" }, "state": { "type": "string" },
        "owner": { "type": "string" }, "level": { "type": "integer" }
    }});
    let mut states = std::collections::BTreeSet::new();
    for seed in 0..50 {
        let v = generate(&s, None, seed, &opts);
        let sku = v["sku"].as_str().unwrap();
        let digits = sku.strip_prefix("SKU-").unwrap();
        assert!(
            digits.len() == 6 && digits.bytes().all(|b| b.is_ascii_digit()),
            "{sku}"
        );
        states.insert(v["state"].as_str().unwrap().to_string());
        assert_eq!(v["owner"], "Ünïcode Name");
        assert!((1..=3).contains(&v["level"].as_i64().unwrap()), "{v}");
    }
    assert_eq!(states.len(), 3, "{states:?}");
}

#[test]
fn builtin_dictionary_has_at_least_150_keys_and_covers_todays_names() {
    let d = Dictionary::builtin();
    assert!(d.len() >= 150, "{} keys", d.len());
    // every name the old hardcoded heuristic, `int_for` and `number_for` knew
    for name in [
        "firstName",
        "givenName",
        "lastName",
        "surname",
        "familyName",
        "name",
        "fullName",
        "email",
        "workEmail",
        "phone",
        "homePhone",
        "mobile",
        "city",
        "state",
        "pincode",
        "zip",
        "zipcode",
        "postalCode",
        "country",
        "address",
        "street",
        "currency",
        "createdAt",
        "updatedAt",
        "deletedAt",
        "timestamp",
        "description",
        "bio",
        "summary",
        "status",
        "url",
        "avatarUrl",
        "id",
        "userId",
        "age",
        "quantity",
        "qty",
        "count",
        "lat",
        "latitude",
        "lng",
        "lon",
        "longitude",
        "price",
        "total",
        "amount",
        "cost",
        "subtotal",
        "unitPrice",
    ] {
        assert!(d.contains(name), "{name}");
    }
}

#[test]
fn adding_a_sibling_property_leaves_other_values_identical() {
    let before = json!({ "type": "object", "properties": {
        "email": { "type": "string" }, "price": { "type": "number" },
        "status": { "type": "string" }, "sku": { "type": "string" },
        "userId": { "type": "integer" }, "bio": { "type": "string", "maxLength": 20 }
    }});
    let mut after = before.clone();
    for (k, v) in [
        ("age", json!({ "type": "integer" })),
        ("orderId", json!({ "type": "string" })),
        ("city", json!({ "type": "string" })),
    ] {
        after["properties"][k] = v;
    }
    let plain = Options::default();
    // a new dictionary entry for a field not in `before` changes nothing either
    let extra = with_project("fields:\n  city: { gen: static, value: Nowhere }\n");
    for seed in 0..30 {
        let a = generate(&before, None, seed, &plain);
        for opts in [&plain, &extra] {
            let b = generate(&after, None, seed, opts);
            for key in ["email", "price", "status", "sku", "userId", "bio"] {
                assert_eq!(a[key], b[key], "seed {seed} key {key}");
            }
        }
    }
}

#[test]
fn keys_that_normalise_alike_resolve_in_file_order_and_are_reported() {
    let src = "fields:\n\
               \x20 userEmail: { gen: static, value: first }\n\
               \x20 b: { gen: email }\n\
               \x20 user_email: { gen: static, value: second }\n\
               \x20 c: { gen: email }\n\
               \x20 USER-EMAIL: { gen: static, value: third }\n";
    let s = json!({ "type": "string" });
    for _ in 0..20 {
        let d = Dictionary::parse(src).unwrap();
        assert_eq!(d.warnings().len(), 2, "{:?}", d.warnings());
        assert!(
            d.warnings()[1].contains("user_email") && d.warnings()[1].contains("USER-EMAIL"),
            "{:?}",
            d.warnings()
        );
        let opts = Options {
            dictionary_project: Some(Arc::new(d)),
            ..Options::default()
        };
        assert_eq!(gen_field(&s, "userEmail", 1, &opts), "third");
    }
    assert_eq!(Dictionary::builtin().warnings(), &[] as &[String]);
}

#[test]
fn check_errors_carry_a_position() {
    for (src, line, what) in [
        (
            "version: 1\nfields:\n  a: { gen: email }\n  sku: { gen: template, value: \"X-{oops}\" }\n",
            4,
            "template",
        ),
        (
            "fields:\n  a: { gen: email }\n  n: { gen: integer, min: 5, max: 1 }\n",
            3,
            "above max",
        ),
        (
            "fields:\n  m: [{ gen: uuid }, { gen: money, min: .inf }]\n",
            2,
            "finite",
        ),
        (
            "fields:\n  a: { gen: email }\n  \"*_\": { gen: email }\n",
            3,
            "empty",
        ),
        ("fields: {}\n# a comment\nversion: 2\n", 3, "version"),
    ] {
        let e = Dictionary::parse(src).unwrap_err();
        assert_eq!(e.line, Some(line), "{src}: {e}");
        assert!(e.col.is_some() && e.message.contains(what), "{src}: {e}");
    }
}

#[test]
fn integer_entry_fits_number_fields() {
    let opts = with_project("fields:\n  n: { gen: integer, min: 3, max: 7 }\n");
    for seed in 0..30 {
        let v = gen_field(&json!({ "type": "number" }), "n", seed, &opts);
        assert!((3..=7).contains(&v.as_i64().unwrap()), "{v}");
        // bounds no whole number fits: a float inside them
        let narrow = json!({ "type": "number", "minimum": 0.1, "maximum": 0.9 });
        let v = gen_field(&narrow, "n", seed, &opts).as_f64().unwrap();
        assert!((0.1..=0.9).contains(&v), "{v}");
    }
}

#[test]
fn write_only_fields_are_not_warned() {
    let opts = with_project("fields:\n  secret: { gen: pick, from: [a] }\n");
    let s = json!({ "type": "object", "properties": {
        "secret": { "type": "integer", "writeOnly": true }
    }});
    let model = ApiModel {
        endpoints: vec![Endpoint {
            method: "GET".into(),
            path: "/w".into(),
            status: 200,
            schema: Some(s),
        }],
    };
    assert_eq!(dictionary_warnings(&model, &opts), Vec::<String>::new());
}

#[test]
fn non_postal_addresses_and_states_are_not_places() {
    let s = json!({ "type": "object", "properties": {
        "city": { "type": "string" }, "ipAddress": { "type": "string" },
        "macAddress": { "type": "string" }, "walletAddress": { "type": "string" },
        "emailAddress": { "type": "string" }, "shippingAddress": { "type": "string" },
        "orderState": { "type": "string" }, "connectionState": { "type": "string" }
    }});
    let pack = Options::default().pack;
    let states: Vec<&str> = pack
        .places
        .iter()
        .filter_map(|p| p.state.as_deref())
        .collect();
    for seed in 0..50 {
        let v = generate(&s, None, seed, &Options::default());
        let city = v["city"].as_str().unwrap();
        let ip = v["ipAddress"].as_str().unwrap();
        assert!(ip.parse::<std::net::Ipv4Addr>().is_ok(), "{v}");
        let mac = v["macAddress"].as_str().unwrap();
        assert!(mac.len() == 17 && mac.starts_with("02:00:00:"), "{v}");
        assert!(!v["walletAddress"].as_str().unwrap().contains(city), "{v}");
        assert!(
            v["emailAddress"]
                .as_str()
                .unwrap()
                .ends_with("@example.com"),
            "{v}"
        );
        assert!(
            v["shippingAddress"].as_str().unwrap().ends_with(city),
            "{v}"
        );
        for k in ["orderState", "connectionState"] {
            assert!(!states.contains(&v[k].as_str().unwrap()), "{k}: {v}");
        }
    }
}

#[test]
fn integer_fields_never_get_floats_or_out_of_range_values() {
    let opts = with_project(
        "fields:\n  lvl: { gen: pick, from: [2.0, 3] }\n  big: { gen: static, value: 18446744073709551615 }\n",
    );
    let i = json!({ "type": "integer" });
    for seed in 0..30 {
        assert_eq!(gen_field(&i, "lvl", seed, &opts), json!(3));
        assert!(gen_field(&i, "big", seed, &opts).is_i64());
    }
}
