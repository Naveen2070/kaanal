//! The reconcile pass: time chains (D2 spec section 4.1).

use std::sync::Arc;

use kaanal_data::{Dictionary, Options, empty, generate};
use serde_json::{Value, json};

const BASE: &str = "2026-01-01T00:00:00Z";

fn props(names: &[&str]) -> Value {
    let p: serde_json::Map<String, Value> = names
        .iter()
        .map(|n| {
            (
                (*n).to_string(),
                json!({ "type": "string", "format": "date-time" }),
            )
        })
        .collect();
    json!({ "type": "object", "properties": p })
}

fn with_dictionary(src: &str) -> Options {
    Options {
        dictionary_project: Some(Arc::new(Dictionary::parse(src).unwrap())),
        ..Options::default()
    }
}

fn at<'a>(v: &'a Value, k: &str) -> &'a str {
    v[k].as_str().unwrap_or_else(|| panic!("{k} in {v}"))
}

#[test]
fn updated_not_before_created_over_1000_seeds() {
    let s = props(&["createdAt", "updatedAt", "deletedAt"]);
    for seed in 0..1000 {
        let v = generate(&s, None, seed, &Options::default());
        let (created, updated, deleted) = (
            at(&v, "createdAt"),
            at(&v, "updatedAt"),
            at(&v, "deletedAt"),
        );
        assert!(
            created <= updated && updated <= deleted && deleted < BASE,
            "{v}"
        );
    }
}

#[test]
fn every_builtin_chain_is_ordered() {
    for names in [
        ["startAt", "endAt"],
        ["orderedAt", "paidAt"],
        ["paidAt", "shippedAt"],
        ["shippedAt", "deliveredAt"],
        ["issuedAt", "dueAt"],
    ] {
        let s = props(&names);
        for seed in 0..300 {
            let v = generate(&s, None, seed, &Options::default());
            assert!(at(&v, names[0]) <= at(&v, names[1]), "{names:?} {v}");
        }
    }
}

#[test]
fn adding_updated_at_does_not_change_created_at() {
    let a = props(&["createdAt"]);
    let b = props(&["createdAt", "updatedAt"]);
    for seed in 0..200 {
        let (x, y) = (
            generate(&a, None, seed, &Options::default()),
            generate(&b, None, seed, &Options::default()),
        );
        assert_eq!(x["createdAt"], y["createdAt"]);
    }
}

#[test]
fn adding_an_unrelated_property_changes_nothing_else() {
    let a = props(&["createdAt", "updatedAt"]);
    let mut b = a.clone();
    b["properties"]["name"] = json!({ "type": "string" });
    for seed in 0..200 {
        let (x, y) = (
            generate(&a, None, seed, &Options::default()),
            generate(&b, None, seed, &Options::default()),
        );
        assert_eq!(x["createdAt"], y["createdAt"]);
        assert_eq!(x["updatedAt"], y["updatedAt"]);
    }
}

#[test]
fn date_only_chain_is_ordered_and_stays_a_date() {
    let s = json!({ "type": "object", "properties": {
        "startDate": { "type": "string", "format": "date" },
        "endDate": { "type": "string", "format": "date" }
    } });
    for seed in 0..500 {
        let v = generate(&s, None, seed, &Options::default());
        let (a, b) = (at(&v, "startDate"), at(&v, "endDate"));
        assert!(a.len() == 10 && b.len() == 10 && a <= b, "{v}");
    }
}

#[test]
fn a_date_and_a_date_time_are_left_alone() {
    let s = json!({ "type": "object", "properties": {
        "startDate": { "type": "string", "format": "date" },
        "endAt": { "type": "string", "format": "date-time" }
    } });
    let plain = json!({ "type": "object", "properties": {
        "startDate": { "type": "string", "format": "date" }
    } });
    let plain_end = json!({ "type": "object", "properties": {
        "endAt": { "type": "string", "format": "date-time" }
    } });
    for seed in 0..50 {
        let v = generate(&s, None, seed, &Options::default());
        let o = Options::default();
        assert_eq!(
            v["startDate"],
            generate(&plain, None, seed, &o)["startDate"]
        );
        assert_eq!(v["endAt"], generate(&plain_end, None, seed, &o)["endAt"]);
    }
}

#[test]
fn a_pinned_earlier_member_is_kept_and_later_ones_derive_from_it() {
    let mut s = props(&["createdAt", "updatedAt"]);
    s["properties"]["createdAt"]["example"] = json!("2025-06-01T10:00:00Z");
    for seed in 0..200 {
        let v = generate(&s, None, seed, &Options::default());
        assert_eq!(v["createdAt"], json!("2025-06-01T10:00:00Z"));
        assert!(
            at(&v, "updatedAt") >= "2025-06-01T10:00:00Z" && at(&v, "updatedAt") < BASE,
            "{v}"
        );
    }
}

#[test]
fn a_pinned_later_member_is_never_changed() {
    let mut s = props(&["createdAt", "updatedAt"]);
    s["properties"]["updatedAt"]["enum"] = json!(["2020-01-01T00:00:00Z"]);
    for seed in 0..50 {
        let v = generate(&s, None, seed, &Options::default());
        assert_eq!(v["updatedAt"], json!("2020-01-01T00:00:00Z"));
    }
}

#[test]
fn project_static_entry_pins_a_member() {
    let o = with_dictionary(
        "version: 1\nfields:\n  created_at: { gen: static, value: \"2024-02-03T04:05:06Z\" }\n",
    );
    let s = props(&["createdAt", "updatedAt"]);
    let v = generate(&s, None, 3, &o);
    assert_eq!(v["createdAt"], json!("2024-02-03T04:05:06Z"));
    assert!(at(&v, "updatedAt") >= "2024-02-03T04:05:06Z", "{v}");
}

#[test]
fn project_chains_append_and_chains_off_removes() {
    let s = props(&["packedAt", "sealedAt"]);
    let o = with_dictionary("version: 1\nchains:\n  - [packed_at, sealed_at]\n");
    for seed in 0..300 {
        let v = generate(&s, None, seed, &o);
        assert!(at(&v, "packedAt") <= at(&v, "sealedAt"), "{v}");
    }
    // off: the built-in created/updated chain no longer applies, so some seed is out of order
    let off = with_dictionary("version: 1\nchains_off: [created_at]\n");
    let c = props(&["createdAt", "updatedAt"]);
    assert!(
        (0..300).any(|seed| {
            let v = generate(&c, None, seed, &off);
            at(&v, "createdAt") > at(&v, "updatedAt")
        }),
        "chains_off had no effect"
    );
}

#[test]
fn chains_need_two_names_and_non_empty_keys() {
    let d = Dictionary::parse("version: 1\nchains:\n  - [only_one]\n").unwrap();
    assert!(
        d.warnings().iter().any(|w| w.contains("at least two")),
        "{:?}",
        d.warnings()
    );
    let e = Dictionary::parse("version: 1\nchains:\n  - [a, \"__\"]\n").unwrap_err();
    assert!(
        e.message.contains("empty after normalising"),
        "{}",
        e.message
    );
}

#[test]
fn the_empty_form_keeps_the_ordered_scalars_of_the_full_body() {
    let s = props(&["createdAt", "updatedAt"]);
    for seed in 0..100 {
        let full = generate(&s, None, seed, &Options::default());
        let e = empty(&s, seed, &Options::default()).unwrap();
        assert_eq!(full, e);
    }
}

#[test]
fn a_project_suffix_static_entry_pins_members() {
    // `*at` matches `createdAt` and `updatedAt` by their last word; both are the developer's
    let o = with_dictionary(
        "version: 1\nfields:\n  \"*at\": { gen: static, value: \"2024-02-03T04:05:06Z\" }\n",
    );
    let s = props(&["createdAt", "updatedAt"]);
    for seed in 0..20 {
        let v = generate(&s, None, seed, &o);
        assert_eq!(v["createdAt"], json!("2024-02-03T04:05:06Z"), "{v}");
        assert_eq!(v["updatedAt"], json!("2024-02-03T04:05:06Z"), "{v}");
    }
}

#[test]
fn a_project_generator_that_is_not_static_or_pick_is_not_pinned() {
    let o = with_dictionary("version: 1\nfields:\n  created_at: { gen: timestamp }\n");
    let s = props(&["createdAt", "updatedAt"]);
    for seed in 0..200 {
        let v = generate(&s, None, seed, &o);
        assert!(at(&v, "createdAt") <= at(&v, "updatedAt"), "{v}");
    }
}

#[test]
fn a_project_chain_overlapping_a_builtin_one_keeps_the_builtin_order() {
    let opts = with_dictionary("version: 1\nchains:\n  - [created_at, deleted_at]\n");
    let schema = props(&["createdAt", "updatedAt", "deletedAt"]);
    for seed in 0..500 {
        let v = generate(&schema, None, seed, &opts);
        let (created, updated, deleted) = (
            at(&v, "createdAt"),
            at(&v, "updatedAt"),
            at(&v, "deletedAt"),
        );
        assert!(created <= updated && updated <= deleted, "{v}");
    }
}

#[test]
fn an_impossible_pinned_date_is_not_chained() {
    let mut s = props(&["createdAt", "updatedAt"]);
    s["properties"]["createdAt"]["example"] = json!("2025-13-45T99:00:00Z");
    let alone = props(&["updatedAt"]);
    for seed in 0..30 {
        let v = generate(&s, None, seed, &Options::default());
        assert_eq!(v["createdAt"], json!("2025-13-45T99:00:00Z"));
        assert_eq!(
            v["updatedAt"],
            generate(&alone, None, seed, &Options::default())["updatedAt"]
        );
    }
}

#[test]
fn a_member_whose_pattern_the_derived_value_would_break_is_left_alone() {
    let mut s = props(&["createdAt", "updatedAt"]);
    s["properties"]["updatedAt"]["pattern"] = json!("^2025-12-31T");
    for seed in 0..100 {
        let v = generate(&s, None, seed, &Options::default());
        assert!(at(&v, "updatedAt").starts_with("2025-12-31T"), "{v}");
    }
}
