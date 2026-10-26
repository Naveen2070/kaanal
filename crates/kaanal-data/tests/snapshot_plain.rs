//! Output of a schema that uses none of the D2 features must never change (D2 plan, Task 1):
//! no weighted flag, status, price, timestamp, age, pattern, `multipleOf` or `uniqueItems`.
//! Record with `KAANAL_RECORD=1 cargo test -p kaanal-data --test snapshot_plain` on a known-good tree.

use kaanal_data::{Options, generate};
use serde_json::{Value, json};

const FILE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/snapshots/plain.json");

fn schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "name": { "type": "string" },
            "email": { "type": "string", "format": "email" },
            "id": { "type": "string", "format": "uuid" },
            "title": { "type": "string" },
            "count": { "type": "integer", "minimum": 1, "maximum": 9 },
            "flag": { "type": "boolean" },
            "address": { "type": "object", "properties": {
                "city": { "type": "string" },
                "postcode": { "type": "string" }
            } },
            "tags": { "type": "array", "items": { "type": "string" }, "minItems": 2, "maxItems": 3 }
        }
    })
}

#[test]
fn plain_schema_output_is_stable() {
    let s = schema();
    let got: Vec<Value> = (1..=20)
        .map(|seed| generate(&s, None, seed, &Options::default()))
        .collect();
    let got = Value::Array(got);
    if std::env::var_os("KAANAL_RECORD").is_some() {
        std::fs::write(FILE, serde_json::to_string_pretty(&got).unwrap() + "\n").unwrap();
    }
    let want: Value = serde_json::from_str(&std::fs::read_to_string(FILE).unwrap()).unwrap();
    assert_eq!(got, want);
}
