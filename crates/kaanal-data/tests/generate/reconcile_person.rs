//! The reconcile pass: age and birth date (D2 spec section 4.2) and locale facts (4.4).

use kaanal_data::{Options, Pack, generate};
use serde_json::{Value, json};

/// Whole years from `YYYY-MM-DD...` to the base date 2026-01-01.
fn years_to_base(dob: &str) -> i64 {
    let y: i64 = dob[0..4].parse().unwrap();
    let (m, d): (u32, u32) = (dob[5..7].parse().unwrap(), dob[8..10].parse().unwrap());
    2026 - y - i64::from((m, d) > (1, 1))
}

#[expect(
    clippy::needless_pass_by_value,
    reason = "a test helper called with json! literals"
)]
fn person(dob: Value, age: Value) -> Value {
    json!({ "type": "object", "properties": { "dateOfBirth": dob, "age": age } })
}

#[test]
fn age_matches_dob_over_1000_seeds() {
    let s = person(
        json!({ "type": "string", "format": "date" }),
        json!({ "type": "integer" }),
    );
    for seed in 0..1000 {
        let v = generate(&s, None, seed, &Options::default());
        let dob = v["dateOfBirth"].as_str().unwrap();
        assert_eq!(years_to_base(dob), v["age"].as_i64().unwrap(), "{v}");
    }
}

#[test]
fn age_stays_the_value_it_would_have_alone() {
    let s = person(
        json!({ "type": "string", "format": "date" }),
        json!({ "type": "integer" }),
    );
    let alone = json!({ "type": "object", "properties": { "age": { "type": "integer" } } });
    for seed in 0..100 {
        assert_eq!(
            generate(&s, None, seed, &Options::default())["age"],
            generate(&alone, None, seed, &Options::default())["age"]
        );
    }
}

#[test]
fn a_pinned_age_derives_the_date() {
    let s = person(
        json!({ "type": "string", "format": "date" }),
        json!({ "type": "integer", "example": 33 }),
    );
    for seed in 0..300 {
        let v = generate(&s, None, seed, &Options::default());
        assert_eq!(v["age"], json!(33));
        assert_eq!(years_to_base(v["dateOfBirth"].as_str().unwrap()), 33, "{v}");
    }
}

#[test]
fn a_pinned_date_derives_the_age() {
    let s = person(
        json!({ "type": "string", "format": "date", "example": "1990-05-17" }),
        json!({ "type": "integer", "minimum": 0, "maximum": 120 }),
    );
    let v = generate(&s, None, 4, &Options::default());
    assert_eq!(v["dateOfBirth"], json!("1990-05-17"));
    assert_eq!(v["age"], json!(35));
}

#[test]
fn the_age_rule_is_skipped_when_its_bounds_would_break() {
    let s = person(
        json!({ "type": "string", "format": "date", "example": "2020-01-01" }),
        json!({ "type": "integer", "minimum": 18, "maximum": 80 }),
    );
    for seed in 0..50 {
        let age = generate(&s, None, seed, &Options::default())["age"]
            .as_i64()
            .unwrap();
        assert!((18..=80).contains(&age));
    }
}

#[test]
fn both_pinned_are_left_alone() {
    let s = person(
        json!({ "type": "string", "format": "date", "example": "2020-01-01" }),
        json!({ "type": "integer", "example": 99 }),
    );
    let v = generate(&s, None, 1, &Options::default());
    assert_eq!(
        (v["dateOfBirth"].clone(), v["age"].clone()),
        (json!("2020-01-01"), json!(99))
    );
}

#[test]
fn a_date_time_birth_date_keeps_its_shape() {
    let s = json!({ "type": "object", "properties": {
        "dob": { "type": "string", "format": "date-time" }, "age": { "type": "integer" } } });
    for seed in 0..300 {
        let v = generate(&s, None, seed, &Options::default());
        let dob = v["dob"].as_str().unwrap();
        assert_eq!(dob.len(), 20, "{v}");
        assert_eq!(years_to_base(dob), v["age"].as_i64().unwrap(), "{v}");
    }
}

#[test]
fn a_format_less_birth_date_is_handled() {
    let s = json!({ "type": "object", "properties": {
        "birthday": { "type": "string" }, "age": { "type": "integer" } } });
    for seed in 0..300 {
        let v = generate(&s, None, seed, &Options::default());
        let dob = v["birthday"].as_str().unwrap();
        assert_eq!(years_to_base(dob), v["age"].as_i64().unwrap(), "{v}");
    }
}

#[test]
fn a_birth_date_alone_is_not_touched() {
    let alone = json!({ "type": "object", "properties": {
        "dateOfBirth": { "type": "string", "format": "date" } } });
    let with_other = json!({ "type": "object", "properties": {
        "dateOfBirth": { "type": "string", "format": "date" },
        "nickname": { "type": "string" } } });
    for seed in 0..50 {
        assert_eq!(
            generate(&alone, None, seed, &Options::default())["dateOfBirth"],
            generate(&with_other, None, seed, &Options::default())["dateOfBirth"]
        );
    }
}

#[test]
fn an_age_alone_is_not_touched() {
    let age = json!({ "type": "object", "properties": { "age": { "type": "integer" } } });
    let both = person(
        json!({ "type": "string", "format": "date" }),
        json!({ "type": "integer" }),
    );
    for seed in 0..50 {
        assert_eq!(
            generate(&age, None, seed, &Options::default())["age"],
            generate(&both, None, seed, &Options::default())["age"]
        );
    }
}

#[test]
fn a_date_time_on_the_edge_day_is_exact_to_the_second() {
    let s = json!({ "type": "object", "properties": {
        "dob": { "type": "string", "format": "date-time" }, "age": { "type": "integer" } } });
    let mut edge = 0;
    for seed in 0..20_000 {
        let v = generate(&s, None, seed, &Options::default());
        let (dob, age) = (v["dob"].as_str().unwrap(), v["age"].as_i64().unwrap());
        // the birthday that falls on the base date itself is only that age at midnight
        if dob.starts_with(&format!("{}-01-01", 2026 - age)) {
            edge += 1;
            assert!(dob.ends_with("T00:00:00Z"), "{dob} {age}");
        }
    }
    assert!(edge > 0, "the edge day was never drawn");
}

#[test]
fn a_birth_date_under_a_dictionary_entry_is_an_adult() {
    let s = json!({ "type": "object", "properties": {
        "birthday": { "type": "string" } } });
    for seed in 0..500 {
        let v = generate(&s, None, seed, &Options::default());
        let d = v["birthday"].as_str().unwrap();
        assert!(("1944-12-31"..="2008-01-01").contains(&d), "{d}");
    }
}

#[test]
fn an_object_never_mixes_countries_across_locale_facts() {
    let s = json!({ "type": "object", "properties": {
        "city": { "type": "string" }, "state": { "type": "string" },
        "postcode": { "type": "string" }, "country": { "type": "string" },
        "phone": { "type": "string" }, "currency": { "type": "string" },
        "locale": { "type": "string" }, "language": { "type": "string" } } });
    for tag in ["en-IN", "de-DE", "fr-FR", "pt-BR", "en-US"] {
        let opts = Options {
            pack: Pack::builtin(tag).unwrap(),
            ..Options::default()
        };
        for seed in 0..100 {
            let v = generate(&s, None, seed, &opts);
            assert_eq!(v["country"], json!(opts.pack.country), "{tag} {v}");
            assert_eq!(v["currency"], json!(opts.pack.currency.code), "{tag} {v}");
            assert_eq!(v["locale"], json!(tag), "{tag} {v}");
            assert_eq!(
                v["language"],
                json!(tag.split('-').next().unwrap()),
                "{tag} {v}"
            );
            let city = v["city"].as_str().unwrap();
            assert!(opts.pack.places.iter().any(|p| p.city == city), "{tag} {v}");
        }
    }
}
