//! `pattern` in generated strings (D2 spec section 6).

use kaanal_data::{ApiModel, Options, generate};
use serde_json::{Value, json};

fn one(s: &Value, seed: u64) -> Value {
    generate(s, None, seed, &Options::default())
}

fn matches(p: &str, v: &Value) -> bool {
    jsonschema::options()
        .build(&json!({ "type": "string", "pattern": p }))
        .unwrap()
        .is_valid(v)
}

#[test]
fn free_string_with_pattern_matches_over_1000_seeds() {
    for p in [
        r"^[A-Z]{3}-\d{4}$",
        r"^(foo|bar)-[a-f0-9]{8}$",
        r"^[a-z]+(\.[a-z]+)*$",
    ] {
        let s = json!({ "type": "string", "pattern": p });
        for seed in 0..1000 {
            let v = one(&s, seed);
            assert!(matches(p, &v), "{p}: {v}");
        }
    }
}

#[test]
fn realistic_email_kept_when_it_already_matches() {
    let s = json!({ "type": "string", "format": "email", "pattern": ".+@.+" });
    let plain = json!({ "type": "string", "format": "email" });
    for seed in 0..50 {
        assert_eq!(one(&s, seed), one(&plain, seed));
    }
}

#[test]
fn mismatch_falls_back_to_the_sampler() {
    let s = json!({ "type": "string", "format": "email", "pattern": r"^[A-Z]{5}$" });
    for seed in 0..100 {
        let v = one(&s, seed);
        assert!(matches(r"^[A-Z]{5}$", &v), "{v}");
    }
}

#[test]
fn pinned_values_are_not_replaced() {
    let ex = json!({ "type": "string", "pattern": "^[A-Z]+$", "example": "abc" });
    assert_eq!(one(&ex, 1), json!("abc"));
    let en = json!({ "type": "string", "pattern": "^[A-Z]+$", "enum": ["xyz"] });
    assert_eq!(one(&en, 1), json!("xyz"));
}

#[test]
fn pattern_and_length_bounds_are_met_together() {
    let s = json!({ "type": "string", "pattern": r"^[a-z]+$", "minLength": 4, "maxLength": 6 });
    for seed in 0..500 {
        let v = one(&s, seed);
        let n = v.as_str().unwrap().chars().count();
        assert!((4..=6).contains(&n) && matches("^[a-z]+$", &v), "{v}");
    }
}

#[test]
fn unmeetable_length_keeps_the_plain_value_and_warns() {
    let spec = r#"
openapi: 3.0.3
info: { title: t, version: "1" }
paths:
  /x:
    get:
      responses:
        "200":
          description: ok
          content:
            application/json:
              schema: { type: object, properties: { code: { type: string, pattern: "^[A-Z]{8}$", maxLength: 4 } } }
"#;
    let model = ApiModel::from_openapi(spec).unwrap();
    let w = model.warnings();
    assert_eq!(w.len(), 1, "{w:?}");
    assert!(
        w[0].starts_with(
            "GET /x .code: pattern ^[A-Z]{8}$ cannot be met within minLength/maxLength"
        ),
        "{w:?}"
    );
    let v = generate(
        model.endpoints[0].schema.as_ref().unwrap(),
        None,
        1,
        &Options::default(),
    );
    assert!(v["code"].is_string());
}

#[test]
fn unsupported_pattern_warns_with_specific_text() {
    let spec = r#"
openapi: 3.0.3
info: { title: t, version: "1" }
paths:
  /x:
    get:
      responses:
        "200":
          description: ok
          content:
            application/json:
              schema: { type: object, properties: { code: { type: string, pattern: "^(?=a)b" } } }
"#;
    let w = ApiModel::from_openapi(spec).unwrap().warnings();
    assert_eq!(w.len(), 1, "{w:?}");
    assert_eq!(
        w[0],
        "GET /x .code: pattern ^(?=a)b is not supported (lookaround, back-reference or too large); generated values may not match"
    );
}

#[test]
fn supported_pattern_has_no_warning() {
    let spec = r#"
openapi: 3.0.3
info: { title: t, version: "1" }
paths:
  /x:
    get:
      responses:
        "200":
          description: ok
          content:
            application/json:
              schema: { type: object, properties: { code: { type: string, pattern: "^[A-Z]{3}$" } } }
"#;
    let w = ApiModel::from_openapi(spec).unwrap().warnings();
    assert!(w.is_empty(), "{w:?}");
}

#[test]
fn adding_a_sibling_property_does_not_change_the_patterned_value() {
    let a = json!({ "type": "object", "properties": { "code": { "type": "string", "pattern": "^[A-Z]{6}$" } } });
    let b = json!({ "type": "object", "properties": {
        "code": { "type": "string", "pattern": "^[A-Z]{6}$" }, "other": { "type": "string" } } });
    for seed in 0..50 {
        assert_eq!(one(&a, seed)["code"], one(&b, seed)["code"]);
    }
}
