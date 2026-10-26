//! Realistic distributions and weighted choices (D2 spec section 5).

use std::collections::HashMap;
use std::sync::Arc;

use kaanal_data::{Dictionary, Options, generate};
use serde_json::{Value, json};

fn with_dictionary(src: &str) -> Options {
    Options {
        dictionary_project: Some(Arc::new(Dictionary::parse(src).unwrap())),
        ..Options::default()
    }
}

/// `field` of `schema` over `n` seeds.
fn draws(field: &str, schema: &Value, n: u64, opts: &Options) -> Vec<Value> {
    let object = json!({ "type": "object", "properties": { field: schema } });
    (0..n)
        .map(|seed| generate(&object, None, seed, opts)[field].clone())
        .collect()
}

#[expect(clippy::cast_precision_loss, reason = "test counts are tiny")]
fn share(values: &[Value], f: impl Fn(&Value) -> bool) -> f64 {
    values.iter().filter(|v| f(v)).count() as f64 / values.len() as f64
}

fn counts(values: &[Value]) -> HashMap<String, usize> {
    let mut m = HashMap::new();
    for v in values {
        *m.entry(v.as_str().map_or_else(|| v.to_string(), str::to_string))
            .or_default() += 1;
    }
    m
}

#[test]
fn weights_are_validated_when_the_dictionary_loads() {
    let zero = Dictionary::parse(
        "version: 1\nfields:\n  mood: { gen: pick, from: [a, b], weights: { a: 0 } }\n",
    )
    .unwrap_err();
    assert!(
        zero.message.contains("at least one weight"),
        "{}",
        zero.message
    );
    let negative = Dictionary::parse(
        "version: 1\nfields:\n  mood: { gen: pick, from: [a], weights: { a: -1 } }\n",
    );
    assert!(negative.is_err());
    let list = Dictionary::parse(
        "version: 1\nfields:\n  mood: { gen: pick, from: [a, b], weights: [1, 2] }\n",
    );
    assert!(list.is_err(), "the list form is not supported");
    let bools = Dictionary::parse(
        "version: 1\nfields:\n  on: { gen: pick, from: [true, false], weights: { true: 3, false: 1 } }\n",
    );
    assert!(bools.is_ok(), "unquoted scalar keys read as their text");
    let days = Dictionary::parse("version: 1\nfields:\n  seen_at: { gen: timestamp, days: 0 }\n")
        .unwrap_err();
    assert!(
        days.message.contains("days must be between 1 and 36500"),
        "{}",
        days.message
    );
}

#[test]
fn enum_weights_keep_every_value_reachable() {
    let s = json!({ "type": "string", "enum": ["paid", "pending", "cancelled", "mystery"] });
    let seen = counts(&draws("status", &s, 10_000, &Options::default()));
    assert_eq!(seen.len(), 4, "{seen:?}");
    assert!(
        seen["paid"] > seen["pending"] && seen["pending"] > seen["cancelled"],
        "{seen:?}"
    );
    assert!(seen["mystery"] > 0, "an unlisted value weighs 1: {seen:?}");
}

#[test]
fn weights_on_pick_and_enum_share_one_shape() {
    let o = with_dictionary(
        "version: 1\nfields:\n  mood: { gen: pick, from: [a, b], weights: { a: 9, b: 1 } }\n",
    );
    let pick = draws("mood", &json!({ "type": "string" }), 5_000, &o);
    let enumd = draws(
        "mood",
        &json!({ "type": "string", "enum": ["a", "b"] }),
        5_000,
        &o,
    );
    for values in [pick, enumd] {
        let a = share(&values, |v| v == "a");
        assert!((0.85..0.95).contains(&a), "{a}");
    }
}

#[test]
fn a_project_plain_entry_turns_weights_off() {
    let o =
        with_dictionary("version: 1\nfields:\n  status: { gen: pick, from: [paid, pending] }\n");
    let s = json!({ "type": "string", "enum": ["paid", "pending", "cancelled"] });
    let values = draws("status", &s, 6_000, &o);
    for word in ["paid", "pending", "cancelled"] {
        let f = share(&values, |v| v == word);
        assert!((0.28..0.39).contains(&f), "{word} {f}");
    }
}

#[test]
fn booleans_are_mostly_true_for_active_flags() {
    let values = draws(
        "active",
        &json!({ "type": "boolean" }),
        5_000,
        &Options::default(),
    );
    let t = share(&values, |v| v == &json!(true));
    assert!((0.75..0.85).contains(&t), "{t}");
}

#[test]
fn ratings_lean_positive_and_every_star_is_possible() {
    let values = draws(
        "rating",
        &json!({ "type": "integer", "minimum": 1, "maximum": 5 }),
        6_000,
        &Options::default(),
    );
    assert!(share(&values, |v| v.as_i64() >= Some(4)) > 0.6);
    assert_eq!(counts(&values).len(), 5);
}

#[test]
fn quantity_is_mostly_one_to_three() {
    let s = json!({ "type": "integer", "minimum": 1, "maximum": 9 });
    let values = draws("quantity", &s, 6_000, &Options::default());
    assert!(share(&values, |v| v.as_i64() <= Some(3)) > 0.65);
    assert!(
        values.iter().any(|v| v.as_i64() >= Some(6)),
        "a large quantity stays possible"
    );
}

#[test]
fn money_is_skewed_cheap_and_stays_in_bounds() {
    let s = json!({ "type": "number", "minimum": 1, "maximum": 5000 });
    let values = draws("amount", &s, 10_000, &Options::default());
    assert!(
        values
            .iter()
            .all(|v| (1.0..=5000.0).contains(&v.as_f64().unwrap()))
    );
    // uniform would put 20 percent below 1000
    assert!(share(&values, |v| v.as_f64() < Some(1000.0)) > 0.6);
    assert!(
        values.iter().any(|v| v.as_f64() > Some(2500.0)),
        "a few expensive ones"
    );
}

#[test]
fn a_range_from_zero_is_drawn_uniformly() {
    let values = draws(
        "tax",
        &json!({ "type": "number", "minimum": 0, "maximum": 500 }),
        10_000,
        &Options::default(),
    );
    let low = share(&values, |v| v.as_f64() < Some(100.0));
    assert!((0.15..0.25).contains(&low), "{low}");
}

#[test]
#[expect(clippy::cast_possible_truncation, reason = "test prices are tiny")]
fn retail_prices_end_in_99_and_00() {
    let s = json!({ "type": "number", "minimum": 1, "maximum": 5000 });
    let values = draws("price", &s, 10_000, &Options::default());
    let cents = |v: &Value| (v.as_f64().unwrap() * 100.0).round() as i64 % 100;
    let nines = share(&values, |v| cents(v) == 99);
    let zeros = share(&values, |v| cents(v) == 0);
    assert!((0.42..0.60).contains(&nines), "{nines}");
    assert!((0.2..0.33).contains(&zeros), "{zeros}");
}

#[test]
fn retail_never_breaks_a_bound() {
    let s = json!({ "type": "number", "minimum": 10.2, "maximum": 10.5 });
    for v in draws("price", &s, 2_000, &Options::default()) {
        assert!((10.2..=10.5).contains(&v.as_f64().unwrap()), "{v}");
    }
}

#[test]
fn retail_is_ignored_on_an_integer_field() {
    let s = json!({ "type": "integer", "minimum": 1, "maximum": 500 });
    for v in draws("price", &s, 500, &Options::default()) {
        assert!(v.is_i64(), "{v}");
    }
}

#[test]
fn a_project_entry_can_make_money_uniform_and_plain() {
    let o = with_dictionary(
        "version: 1\nfields:\n  price: { gen: money, min: 1, max: 5000, dist: uniform, style: plain }\n",
    );
    let values = draws("price", &json!({ "type": "number" }), 10_000, &o);
    let low = share(&values, |v| v.as_f64() < Some(1000.0));
    assert!((0.15..0.25).contains(&low), "{low}");
}

#[test]
fn timestamps_lean_recent_inside_the_window() {
    let s = json!({ "type": "string", "format": "date-time" });
    let values = draws("loggedAt", &s, 6_000, &Options::default());
    // 365 / 3 days before 2026-01-01
    let recent = share(&values, |v| v.as_str().unwrap() >= "2025-09-01T08:00:00Z");
    assert!((0.62..0.72).contains(&recent), "{recent}");
    assert!(
        values
            .iter()
            .all(|v| v.as_str().unwrap() < "2026-01-01T00:00:00Z")
    );
    assert!(
        values.iter().any(|v| v.as_str().unwrap() < "2025-03-01"),
        "the whole window is used"
    );
}

#[test]
fn a_days_key_narrows_the_window() {
    let o = with_dictionary("version: 1\nfields:\n  seen_at: { gen: timestamp, days: 7 }\n");
    let values = draws("seen_at", &json!({ "type": "string" }), 1_000, &o);
    assert!(
        values
            .iter()
            .all(|v| v.as_str().unwrap() >= "2025-12-25T00:00:00Z")
    );
}

#[test]
fn ages_follow_the_four_weighted_bands() {
    let values = draws(
        "age",
        &json!({ "type": "integer" }),
        20_000,
        &Options::default(),
    );
    assert!(
        values
            .iter()
            .all(|v| (18..=80).contains(&v.as_i64().unwrap()))
    );
    let band = |lo, hi| share(&values, |v| (lo..=hi).contains(&v.as_i64().unwrap()));
    assert!((0.22..0.28).contains(&band(18, 32)), "{}", band(18, 32));
    assert!((0.27..0.33).contains(&band(33, 48)), "{}", band(33, 48));
    assert!((0.17..0.23).contains(&band(65, 80)), "{}", band(65, 80));
}

#[test]
fn same_seed_same_output_for_every_distribution() {
    let s = json!({ "type": "object", "properties": {
        "price": { "type": "number" }, "status": { "type": "string", "enum": ["paid", "failed"] },
        "seenAt": { "type": "string", "format": "date-time" }, "age": { "type": "integer" } } });
    for seed in 0..50 {
        assert_eq!(
            generate(&s, None, seed, &Options::default()),
            generate(&s, None, seed, &Options::default())
        );
    }
}

#[test]
fn days_and_years_have_sane_limits() {
    let big =
        Dictionary::parse("version: 1\nfields:\n  seen_at: { gen: timestamp, days: 4000000000 }\n")
            .unwrap_err();
    assert!(
        big.message.contains("between 1 and 36500"),
        "{}",
        big.message
    );
    let years = Dictionary::parse("version: 1\nfields:\n  born: { gen: date, years: [40, 18] }\n")
        .unwrap_err();
    assert!(
        years.message.contains("must be ordered"),
        "{}",
        years.message
    );
}

#[test]
fn birth_dates_without_an_age_are_adults() {
    // 18 to 80 years before 2026-01-01
    let values = draws(
        "dateOfBirth",
        &json!({ "type": "string", "format": "date" }),
        3_000,
        &Options::default(),
    );
    for v in &values {
        let d = v.as_str().unwrap();
        assert!(("1944-12-31"..="2008-01-01").contains(&d), "{d}");
    }
    assert!(
        values.iter().any(|v| v.as_str().unwrap() < "1965-01-01"),
        "older people occur"
    );
}

#[test]
fn whole_float_enum_values_match_whole_number_weights() {
    let o = with_dictionary(
        "version: 1\nfields:\n  grade: { gen: pick, from: [1, 2], weights: { 1: 9, 2: 1 } }\n",
    );
    let values = draws(
        "grade",
        &json!({ "type": "number", "enum": [1.0, 2.0] }),
        4_000,
        &o,
    );
    let ones = share(&values, |v| v.as_f64() == Some(1.0));
    assert!((0.85..0.95).contains(&ones), "{ones}");
}

#[test]
fn a_huge_money_range_and_a_tiny_minimum_still_draw() {
    // the top of a very wide range stays possible; a subnormal minimum falls back to uniform
    let wide = draws(
        "amount",
        &json!({ "type": "number", "minimum": 0.01, "maximum": 1e12 }),
        20_000,
        &Options::default(),
    );
    assert!(
        wide.iter().any(|v| v.as_f64() > Some(1e10)),
        "the last bucket reaches the maximum"
    );
    let tiny = draws(
        "amount",
        &json!({ "type": "number", "minimum": 5e-324, "maximum": 10.0 }),
        100,
        &Options::default(),
    );
    assert!(tiny.iter().all(|v| v.as_f64().is_some()));
}
