//! Fingerprint primitives: the canonical digest of a config text and the 12-digit combination.
//! Oracles: expected hex values come from `sha256sum` on the exact bytes (raw text, or compact
//! key-sorted JSON), not from this code.

use kaanal_core::config::Config;
use kaanal_core::fingerprint::{canonical_digest, combine};

fn hex(digest: &[u8; 32]) -> String {
    use std::fmt::Write;
    digest.iter().fold(String::new(), |mut out, byte| {
        write!(out, "{byte:02x}").unwrap();
        out
    })
}

const MIN: &str = "version: 1\nseed: 7\nnodes:\n  a: { type: service }\n";

#[test]
fn comments_whitespace_and_key_order_do_not_change_the_digest() {
    let one = "a: 1\nb: {x: 2, y: 3}\n";
    let two = "# note\nb:\n  y: 3\n  x: 2\n\n\na: 1 # one\n";
    assert_eq!(canonical_digest(one), canonical_digest(two));
}

#[test]
fn digest_is_sha256_of_compact_sorted_json() {
    assert_eq!(
        hex(&canonical_digest(
            "b: {y: 3, x: 2}
a: 1
"
        )),
        "46828c419428ba79d8942f422c83f507e73d888f31bb4801b14fb07e9ed378b3"
    );
}

#[test]
fn a_changed_value_changes_the_digest() {
    assert_ne!(canonical_digest("a: 1\n"), canonical_digest("a: 2\n"));
}

#[test]
fn unparseable_text_hashes_its_bytes() {
    assert_ne!(canonical_digest("a: [1"), canonical_digest("a: [2"));
    assert_eq!(
        hex(&canonical_digest("a: [1")),
        "35f4154eb8c7036bba506ece292385c3603264bd099fe2553ea5fdc6fb5d5d5b"
    );
}

#[test]
fn combine_is_order_independent_and_12_hex_digits() {
    let (spec, scen) = ([1u8; 32], [2u8; 32]);
    let fwd = vec![("spec".to_string(), spec), ("scenario".to_string(), scen)];
    let rev = vec![("scenario".to_string(), scen), ("spec".to_string(), spec)];
    let id = combine(&fwd);
    assert_eq!(id, combine(&rev));
    assert_eq!(id.len(), 12);
    assert!(id.bytes().all(|c| matches!(c, b'0'..=b'9' | b'a'..=b'f')));
    let changed = vec![
        ("spec".to_string(), [3u8; 32]),
        ("scenario".to_string(), scen),
    ];
    assert_ne!(id, combine(&changed));
    let relabel = vec![("spec2".to_string(), spec), ("scenario".to_string(), scen)];
    assert_ne!(id, combine(&relabel));
}

#[test]
fn from_yaml_sets_the_digest() {
    let cfg = Config::from_yaml(MIN).unwrap();
    assert_eq!(cfg.digest, canonical_digest(MIN));
    let commented = format!("# hello\n{MIN}");
    assert_eq!(Config::from_yaml(&commented).unwrap().digest, cfg.digest);
    let reseeded = MIN.replace("seed: 7", "seed: 8");
    assert_ne!(Config::from_yaml(&reseeded).unwrap().digest, cfg.digest);
}

/// Six levels of ten aliases each: 10^6 nodes, past the checker's `50_000` bound, from ~200 bytes.
const BOMB: &str = "a: &a [x, x, x, x, x, x, x, x, x, x]
b: &b [*a, *a, *a, *a, *a, *a, *a, *a, *a, *a]
c: &c [*b, *b, *b, *b, *b, *b, *b, *b, *b, *b]
d: &d [*c, *c, *c, *c, *c, *c, *c, *c, *c, *c]
e: &e [*d, *d, *d, *d, *d, *d, *d, *d, *d, *d]
f: &f [*e, *e, *e, *e, *e, *e, *e, *e, *e, *e]
";

#[test]
fn an_alias_bomb_hashes_its_bytes_without_expanding() {
    let start = std::time::Instant::now();
    let digest = canonical_digest(BOMB);
    assert!(start.elapsed() < std::time::Duration::from_secs(5));
    assert_eq!(
        hex(&digest),
        "dfce0503aa496b676593f7a74a2d4dfac9014f4c23de0b06bc9c4c1d4e0be55a"
    );
}
