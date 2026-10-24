//! Unit tests for the data generator.

use super::text::civil_from_days;

#[test]
fn stable_hash_matches_core_algorithm() {
    // same vectors as kaanal_core::stable_hash tests
    assert_eq!(super::stable_hash(&[]), 0xcbf2_9ce4_8422_2325);
    assert_eq!(super::stable_hash(&[b"a", b"bc"]), 0xa0a3_542c_19b9_00ab);
    assert_ne!(
        super::stable_hash(&[b"ab", b"c"]),
        super::stable_hash(&[b"a", b"bc"])
    );
}

#[test]
fn merge_all_of_unions_properties_and_keeps_the_first_type() {
    use serde_json::json;
    let s = json!({
        "description": "sibling",
        "required": ["a"],
        "properties": { "a": { "type": "integer" } },
        "allOf": [
            { "type": "object", "description": "part", "required": ["a", "b"],
              "properties": { "b": { "type": "string" } } },
            { "allOf": [{ "type": "array", "properties": { "a": { "type": "string" } } }] }
        ]
    });
    let (m, conflicts) = super::merge_all_of(&s);
    assert_eq!(
        m,
        json!({
            "description": "sibling",
            "type": "object",
            "required": ["a", "b"],
            "properties": { "a": { "type": "string" }, "b": { "type": "string" } }
        })
    );
    assert_eq!(conflicts.len(), 1, "{conflicts:?}");
}

#[test]
fn civil_dates() {
    assert_eq!(civil_from_days(0), (1970, 1, 1));
    assert_eq!(civil_from_days(super::BASE_EPOCH / 86_400), (2026, 1, 1));
    assert_eq!(civil_from_days(19_782), (2024, 2, 29));
}
