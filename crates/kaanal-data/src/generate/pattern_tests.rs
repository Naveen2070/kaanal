//! Unit tests for the `pattern` sampler (D2 spec section 6).

use rand::SeedableRng;
use rand_chacha::ChaCha8Rng;
use serde_json::{Value, json};

use super::pattern::{cached, compile};

const GOLDEN: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/snapshots/pattern.json");

fn draw(p: &str, seed: u64, max_len: Option<usize>) -> String {
    compile(p)
        .unwrap_or_else(|e| panic!("{p}: {e}"))
        .sample(&mut ChaCha8Rng::seed_from_u64(seed), (0, max_len))
}

#[test]
fn sample_matches_every_supported_construct() {
    for p in [
        r"^[A-Z]{3}-\d{4}$",
        r"^(foo|bar)-[a-f0-9]{8}$",
        r"^\+?[0-9]{10,12}$",
        r"^[a-z]+(\.[a-z]+)*$",
        r"[A-Z]{2}\d+",
        r"^(?i)abc[0-9]?$",
        r"^.{3,6}$",
    ] {
        let c = compile(p).unwrap();
        for seed in 0..500 {
            let s = c.sample(&mut ChaCha8Rng::seed_from_u64(seed), (0, None));
            assert!(c.is_match(&s), "{p}: {s:?} does not match");
        }
    }
}

#[test]
fn classes_are_ascii() {
    for p in [r"^\w+$", r"^\d{4}$", r"^[[:alpha:]]+$", r"^\S+$"] {
        for seed in 0..500 {
            let s = draw(p, seed, None);
            assert!(s.is_ascii() && !s.is_empty(), "{p}: {s:?}");
        }
    }
}

#[test]
fn dot_and_negated_class_give_printable_characters() {
    for seed in 0..500 {
        let s = draw(r"^[^a].$", seed, None);
        assert!(
            s.chars().all(|c| (' '..='~').contains(&c)) && !s.starts_with('a'),
            "{s:?}"
        );
    }
}

#[test]
fn anchors_are_stripped_at_the_edges_and_empty_elsewhere() {
    assert_eq!(draw(r"^abc$", 1, None), "abc");
    assert_eq!(draw(r"a\bb", 1, None), "ab");
}

#[test]
fn empty_ascii_intersection_uses_the_original_class() {
    let s = draw(r"^\p{Han}{3}$", 4, None);
    assert_eq!(s.chars().count(), 3, "{s:?}");
    assert!(s.chars().all(|c| !c.is_ascii()), "{s:?}");
}

#[test]
fn open_repeat_is_capped_by_the_length_budget() {
    for seed in 0..200 {
        assert!(draw(r"^a+$", seed, Some(3)).len() <= 3);
        assert_eq!(draw(r"^[a-z]*$", seed, Some(0)), "");
        assert!(draw(r"^a*$", seed, None).len() <= 8);
    }
}

#[test]
fn hostile_nested_repeat_is_unsupported() {
    assert!(compile(r"(a{1000}){1000}").is_err());
    assert!(compile(r"a{100000}").is_err());
}

#[test]
fn lookaround_and_backreference_are_unsupported() {
    assert!(compile(r"(?=a)b").is_err());
    assert!(compile(r"(a)\1").is_err());
    assert!(compile(r"[").is_err());
}

#[test]
fn is_match_agrees_with_the_validator_not_the_regex_crate() {
    let c = compile(r"^\w+$").unwrap();
    assert!(c.is_match("Mueller"));
    assert!(!c.is_match("Müller"));
}

#[test]
fn compiled_patterns_are_cached() {
    assert!(std::sync::Arc::ptr_eq(&cached("^x{2}$"), &cached("^x{2}$")));
    assert!(cached("(").is_err_entry());
}

trait IsErrEntry {
    fn is_err_entry(&self) -> bool;
}

impl IsErrEntry for std::sync::Arc<Result<super::pattern::Compiled, String>> {
    fn is_err_entry(&self) -> bool {
        self.as_ref().is_err()
    }
}

#[test]
fn same_seed_same_output_and_golden() {
    let patterns = [
        r"^[A-Z]{3}-\d{4}$",
        r"^(foo|bar)-[a-f0-9]{8}$",
        r"^\+?[0-9]{10,12}$",
        r"^[a-z]+(\.[a-z]+)*$",
        r"^[^a].{2}$",
    ];
    let got: Vec<Value> = patterns
        .iter()
        .map(|p| Value::Array((1..=3).map(|s| json!(draw(p, s, None))).collect()))
        .collect();
    let again: Vec<Value> = patterns
        .iter()
        .map(|p| Value::Array((1..=3).map(|s| json!(draw(p, s, None))).collect()))
        .collect();
    assert_eq!(got, again);
    let got = Value::Array(got);
    if std::env::var_os("KAANAL_RECORD").is_some() {
        std::fs::write(GOLDEN, serde_json::to_string_pretty(&got).unwrap() + "\n").unwrap();
    }
    let want: Value = serde_json::from_str(&std::fs::read_to_string(GOLDEN).unwrap()).unwrap();
    assert_eq!(got, want);
}

#[test]
fn an_open_repeat_reaches_the_minimum_length() {
    let c = compile("^[A-Za-z0-9]+$").unwrap();
    for seed in 0..300 {
        let s = c.sample(&mut ChaCha8Rng::seed_from_u64(seed), (10, Some(12)));
        assert!((10..=12).contains(&s.len()), "{s:?}");
    }
}
