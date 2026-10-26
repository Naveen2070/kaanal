//! D2 output is the same on every OS (spec 8): the distributions use integer tables only, no
//! `ln`/`exp`/`powf`, and the regex sampler draws integers. CI runs `cargo test --workspace` on
//! Linux, Windows and macOS, so this checked-in golden is compared on all three.
//!
//! Record with `KAANAL_RECORD=1 cargo test -p kaanal-data --test golden_cross_os` on a
//! known-good tree, and list the re-pinned fields in the commit message.

use kaanal_data::{Options, generate};
use serde_json::{Value, json};

const FILE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/snapshots/d2_golden.json"
);

fn schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "createdAt": { "type": "string", "format": "date-time" },
            "updatedAt": { "type": "string", "format": "date-time" },
            "deletedAt": { "type": "string", "format": "date-time" },
            "age": { "type": "integer" },
            "dateOfBirth": { "type": "string", "format": "date" },
            "status": { "type": "string", "enum": ["paid", "pending", "cancelled", "mystery"] },
            "active": { "type": "boolean" },
            "rating": { "type": "integer", "minimum": 1, "maximum": 5 },
            "price": { "type": "number", "minimum": 1, "maximum": 5000 },
            "amount": { "type": "number", "minimum": 1, "maximum": 5000 },
            "seenAt": { "type": "string", "format": "date-time" },
            "code": { "type": "string", "pattern": "^[A-Z]{3}-\\d{4}$" },
            "sku": { "type": "string", "pattern": "^(foo|bar)-[a-f0-9]{8}$" },
            "step": { "type": "number", "multipleOf": 0.25, "minimum": 1, "maximum": 50 },
            "fives": { "type": "integer", "multipleOf": 5, "minimum": 7, "maximum": 93 },
            "tags": { "type": "array", "uniqueItems": true, "minItems": 4, "maxItems": 4,
                "items": { "type": "string", "enum": ["a", "b", "c", "d", "e", "f"] } },
            "locale": { "type": "string" },
            "language": { "type": "string" },
            "items": { "type": "array", "minItems": 2, "maxItems": 3, "items": {
                "type": "object", "properties": {
                    "price": { "type": "number", "minimum": 1, "maximum": 500 },
                    "quantity": { "type": "integer", "minimum": 1, "maximum": 9 },
                    "lineTotal": { "type": "number" } } } },
            "subtotal": { "type": "number" },
            "tax": { "type": "number" },
            "shipping": { "type": "number" },
            "discount": { "type": "number" },
            "total": { "type": "number" }
        }
    })
}

#[test]
fn d2_output_matches_the_checked_in_golden() {
    let s = schema();
    let got = Value::Array(
        (1..=5)
            .map(|seed| generate(&s, None, seed, &Options::default()))
            .collect(),
    );
    if std::env::var_os("KAANAL_RECORD").is_some() {
        std::fs::write(FILE, serde_json::to_string_pretty(&got).unwrap() + "\n").unwrap();
    }
    let want: Value = serde_json::from_str(&std::fs::read_to_string(FILE).unwrap()).unwrap();
    assert_eq!(got, want);
}
