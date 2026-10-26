//! `multipleOf` (D2 spec section 5.5).

use kaanal_data::{ApiModel, Options, generate};
use serde_json::{Value, json};

fn one(s: &Value, seed: u64) -> Value {
    generate(s, None, seed, &Options::default())
}

fn warnings_of(schema: &str) -> Vec<String> {
    let spec = format!(
        r#"
openapi: 3.0.3
info: {{ title: t, version: "1" }}
paths:
  /x:
    get:
      responses:
        "200":
          description: ok
          content:
            application/json:
              schema: {{ type: object, properties: {{ n: {schema} }} }}
"#
    );
    ApiModel::from_openapi(&spec).unwrap().warnings()
}

#[test]
fn money_multiple_of_a_cent_is_exact() {
    let s = json!({ "type": "number", "multipleOf": 0.01, "minimum": 0, "maximum": 1000 });
    for seed in 0..1000 {
        let v = one(&s, seed);
        let cents = (v.as_f64().unwrap() * 100.0).round();
        assert!((v.as_f64().unwrap() * 100.0 - cents).abs() < 1e-6, "{v}");
        assert!((0.0..=1000.0).contains(&v.as_f64().unwrap()), "{v}");
    }
}

#[test]
fn a_quarter_step_gives_only_quarters() {
    let s = json!({ "type": "number", "multipleOf": 0.25, "minimum": 1, "maximum": 50 });
    for seed in 0..500 {
        let x = one(&s, seed).as_f64().unwrap();
        assert_eq!((x * 4.0).fract(), 0.0, "{x}");
        assert!((1.0..=50.0).contains(&x));
    }
}

#[test]
fn integer_multiple_of_five_within_bounds() {
    let s = json!({ "type": "integer", "multipleOf": 5, "minimum": 7, "maximum": 93 });
    for seed in 0..500 {
        let v = one(&s, seed).as_i64().unwrap();
        assert!(v % 5 == 0 && (7..=93).contains(&v), "{v}");
    }
}

#[test]
fn exclusive_bounds_are_respected() {
    let s = json!({ "type": "integer", "multipleOf": 5, "minimum": 10, "exclusiveMinimum": true, "maximum": 30, "exclusiveMaximum": true });
    for seed in 0..300 {
        let v = one(&s, seed).as_i64().unwrap();
        assert!(v % 5 == 0 && v > 10 && v < 30, "{v}");
    }
}

#[test]
fn a_pinned_value_is_not_moved() {
    let s = json!({ "type": "integer", "multipleOf": 5, "example": 7 });
    assert_eq!(one(&s, 1), json!(7));
    let e = json!({ "type": "integer", "multipleOf": 5, "enum": [7] });
    assert_eq!(one(&e, 1), json!(7));
}

#[test]
fn no_multiple_that_fits_warns_and_skips() {
    let w = warnings_of("{ type: integer, multipleOf: 10, minimum: 1, maximum: 9 }");
    assert_eq!(w.len(), 1, "{w:?}");
    assert!(
        w[0].contains("multipleOf 10 has no multiple between minimum and maximum"),
        "{w:?}"
    );
    let s = json!({ "type": "integer", "multipleOf": 10, "minimum": 1, "maximum": 9 });
    let v = one(&s, 3).as_i64().unwrap();
    assert!((1..=9).contains(&v));
}

#[test]
fn a_fractional_step_on_an_integer_field_warns_unless_every_integer_meets_it() {
    // 0.5 and 0.25 divide every integer: nothing to say
    for m in ["0.5", "0.25", "1"] {
        let w = warnings_of(&format!("{{ type: integer, multipleOf: {m} }}"));
        assert!(w.is_empty(), "{m}: {w:?}");
    }
    let w = warnings_of("{ type: integer, multipleOf: 0.3 }");
    assert_eq!(w.len(), 1, "{w:?}");
    assert!(
        w[0].contains("on an integer field is not supported"),
        "{w:?}"
    );
}

#[test]
fn bounds_with_float_error_still_admit_their_multiple() {
    // 1.1 * 100 = 110.00000000000001 and 0.57 * 100 = 56.99999999999999
    let w = warnings_of("{ type: number, multipleOf: 0.01, minimum: 1.1, maximum: 1.1 }");
    assert!(w.is_empty(), "{w:?}");
    let s = json!({ "type": "number", "multipleOf": 0.01, "minimum": 0.57, "maximum": 0.57 });
    assert_eq!(one(&s, 1), json!(0.57));
}

#[test]
fn bounds_beyond_the_counting_range_do_not_panic() {
    let w = warnings_of("{ type: number, multipleOf: 2, minimum: 1.0e300 }");
    assert_eq!(w.len(), 1, "{w:?}");
    assert!(w[0].contains("beyond 1e15 units"), "{w:?}");
    let s = json!({ "type": "number", "multipleOf": 2, "minimum": 1e300 });
    assert!(one(&s, 1).as_f64().unwrap() >= 1e300);
}

#[test]
fn a_usable_multiple_of_has_no_warning() {
    let w = warnings_of("{ type: number, multipleOf: 0.01, minimum: 0, maximum: 10 }");
    assert!(w.is_empty(), "{w:?}");
}

#[test]
fn adding_a_sibling_property_does_not_change_the_snapped_value() {
    let a = json!({ "type": "object", "properties": { "n": { "type": "number", "multipleOf": 0.5, "minimum": 0, "maximum": 99 } } });
    let mut b = a.clone();
    b["properties"]["other"] = json!({ "type": "string" });
    for seed in 0..50 {
        assert_eq!(one(&a, seed)["n"], one(&b, seed)["n"]);
    }
}
