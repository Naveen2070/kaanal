//! The reconcile pass: money sums (D2 spec section 4.3).

use kaanal_data::{ApiModel, Options, PageSpec, generate, generate_page};
use serde_json::{Value, json};

/// An order with line items. `price` is the schema of the unit price only; every other money
/// field is a plain field of the same type, so a bound on `price` never blocks a derived value.
#[expect(
    clippy::needless_pass_by_value,
    reason = "a test helper called with json! literals"
)]
fn order(price: Value, extra: Value) -> Value {
    let money = json!({ "type": price["type"] });
    let mut props = json!({
        "items": { "type": "array", "minItems": 2, "maxItems": 4, "items": { "type": "object",
            "properties": {
                "price": price,
                "quantity": { "type": "integer", "minimum": 1, "maximum": 9 },
                "lineTotal": money
            } } },
        "subtotal": money, "tax": money, "shipping": money, "discount": money, "total": money
    });
    if let (Some(p), Some(e)) = (props.as_object_mut(), extra.as_object()) {
        p.extend(e.clone());
    }
    json!({ "type": "object", "properties": props })
}

#[expect(clippy::cast_possible_truncation, reason = "test amounts are tiny")]
fn cents(v: &Value) -> i128 {
    (v.as_f64().unwrap() * 100.0).round() as i128
}

fn decimals_ok(v: &Value) -> bool {
    let t = v.to_string();
    t.split_once('.').is_none_or(|(_, d)| d.len() <= 2)
}

fn without_items(s: &Value) -> Value {
    let mut s = s.clone();
    s["properties"].as_object_mut().unwrap().remove("items");
    s
}

#[test]
fn total_equals_sum_of_lines_exactly() {
    let s = order(json!({ "type": "number" }), json!({}));
    for seed in 0..500 {
        let v = generate(&s, None, seed, &Options::default());
        let mut sum = 0;
        for line in v["items"].as_array().unwrap() {
            let expect = cents(&line["price"]) * i128::from(line["quantity"].as_i64().unwrap());
            assert_eq!(cents(&line["lineTotal"]), expect, "{v}");
            assert!(decimals_ok(&line["lineTotal"]), "{v}");
            sum += expect;
        }
        assert_eq!(cents(&v["subtotal"]), sum, "{v}");
        let discount = cents(&v["discount"]).min(sum);
        let total = sum + cents(&v["tax"]) + cents(&v["shipping"]) - discount;
        assert_eq!(cents(&v["total"]), total, "{v}");
        assert!(
            decimals_ok(&v["total"]) && decimals_ok(&v["subtotal"]),
            "{v}"
        );
        assert!(
            cents(&v["total"]) >= 0 && cents(&v["discount"]) <= sum,
            "{v}"
        );
    }
}

#[test]
fn tax_is_a_fraction_of_the_subtotal() {
    let s = order(json!({ "type": "number" }), json!({}));
    for seed in 0..300 {
        let v = generate(&s, None, seed, &Options::default());
        assert!(
            cents(&v["tax"]) * 1000 <= cents(&v["subtotal"]) * 200 + 1000,
            "{v}"
        );
    }
}

#[test]
fn integer_money_uses_integer_math() {
    let s = order(
        json!({ "type": "integer", "minimum": 1, "maximum": 500 }),
        json!({}),
    );
    for seed in 0..300 {
        let v = generate(&s, None, seed, &Options::default());
        let sum: i64 = v["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|l| l["price"].as_i64().unwrap() * l["quantity"].as_i64().unwrap())
            .sum();
        assert_eq!(v["subtotal"], json!(sum), "{v}");
        assert!(v["total"].is_i64(), "{v}");
    }
}

#[test]
fn mixed_integer_and_number_skips() {
    let s = order(
        json!({ "type": "integer", "minimum": 1, "maximum": 500 }),
        json!({ "subtotal": { "type": "number" } }),
    );
    let plain = without_items(&s);
    for seed in 0..50 {
        let v = generate(&s, None, seed, &Options::default());
        assert_eq!(
            v["subtotal"],
            generate(&plain, None, seed, &Options::default())["subtotal"]
        );
    }
}

#[test]
fn plain_listing_total_untouched() {
    let s = json!({ "type": "object", "properties": {
        "items": { "type": "array", "items": { "type": "object", "properties": {
            "name": { "type": "string" }, "price": { "type": "number" } } } },
        "total": { "type": "integer" }
    } });
    let plain = without_items(&s);
    for seed in 0..100 {
        let v = generate(&s, None, seed, &Options::default());
        assert_eq!(
            v["total"],
            generate(&plain, None, seed, &Options::default())["total"]
        );
    }
}

#[test]
fn bare_total_without_line_items_untouched() {
    let s = json!({ "type": "object", "properties": {
        "subtotal": { "type": "number" }, "total": { "type": "integer" }
    } });
    let only_total = json!({ "type": "object", "properties": { "total": { "type": "integer" } } });
    for seed in 0..50 {
        assert_eq!(
            generate(&s, None, seed, &Options::default())["total"],
            generate(&only_total, None, seed, &Options::default())["total"]
        );
    }
}

#[test]
fn paged_envelope_total_stays_the_item_count() {
    let spec = r#"
openapi: 3.0.3
info: { title: t, version: "1" }
paths:
  /orders:
    get:
      parameters:
        - { name: page, in: query, schema: { type: integer } }
      responses:
        "200":
          description: ok
          content:
            application/json:
              schema:
                type: object
                properties:
                  total: { type: integer }
                  items:
                    type: array
                    items:
                      type: object
                      properties:
                        price: { type: number }
                        quantity: { type: integer }
"#;
    let model = ApiModel::from_openapi(spec).unwrap();
    let e = &model.endpoints[0];
    let page = PageSpec::detect(e).unwrap();
    for seed in 0..50 {
        let w = page.window(&[], seed, None).unwrap();
        let body = generate_page(
            e.schema.as_ref().unwrap(),
            &page,
            &w,
            seed,
            &Options::default(),
            "/orders",
            &[],
        );
        assert_eq!(body["total"], json!(w.total), "{body}");
    }
}

#[test]
fn discount_is_capped_at_the_subtotal_and_the_total_never_negative() {
    let s = order(
        json!({ "type": "number", "minimum": 1, "maximum": 10 }),
        json!({}),
    );
    let mut capped = 0;
    for seed in 0..300 {
        let v = generate(&s, None, seed, &Options::default());
        assert!(cents(&v["discount"]) <= cents(&v["subtotal"]), "{v}");
        assert!(cents(&v["total"]) >= 0, "{v}");
        capped += usize::from(cents(&v["discount"]) == cents(&v["subtotal"]));
    }
    assert!(capped > 0, "the cap never applied");
}

#[test]
fn a_pinned_discount_is_kept_and_total_still_never_negative() {
    let mut s = order(json!({ "type": "number" }), json!({}));
    s["properties"]["discount"] = json!({ "type": "number", "example": 99999.0 });
    for seed in 0..50 {
        let v = generate(&s, None, seed, &Options::default());
        assert_eq!(v["discount"], json!(99999.0));
        assert!(cents(&v["total"]) >= 0, "{v}");
    }
}

#[test]
fn overflow_skips_the_object() {
    let big = json!({ "type": "integer", "example": i64::MAX });
    let mut s = order(big.clone(), json!({}));
    s["properties"]["items"]["items"]["properties"]["quantity"] = big;
    let plain = without_items(&s);
    let v = generate(&s, None, 1, &Options::default());
    assert_eq!(
        v["subtotal"],
        generate(&plain, None, 1, &Options::default())["subtotal"]
    );
}

#[test]
fn bounds_win() {
    let s = order(
        json!({ "type": "number", "minimum": 1, "maximum": 100 }),
        json!({ "subtotal": { "type": "number", "minimum": 0, "maximum": 1 } }),
    );
    for seed in 0..100 {
        let v = generate(&s, None, seed, &Options::default());
        assert!(cents(&v["subtotal"]) <= 100, "{v}");
    }
}

#[test]
fn a_pinned_line_amount_is_counted_not_rewritten() {
    let mut s = order(json!({ "type": "number" }), json!({}));
    s["properties"]["items"]["items"]["properties"]["lineTotal"] =
        json!({ "type": "number", "example": 10.0 });
    let v = generate(&s, None, 3, &Options::default());
    let n = v["items"].as_array().unwrap().len();
    assert!(
        v["items"]
            .as_array()
            .unwrap()
            .iter()
            .all(|l| l["lineTotal"] == json!(10.0))
    );
    assert_eq!(cents(&v["subtotal"]), 1000 * n as i128, "{v}");
}

#[test]
fn adding_an_unrelated_property_changes_no_money_field() {
    let a = order(json!({ "type": "number" }), json!({}));
    let mut b = a.clone();
    b["properties"]["note"] = json!({ "type": "string" });
    for seed in 0..100 {
        let (x, y) = (
            generate(&a, None, seed, &Options::default()),
            generate(&b, None, seed, &Options::default()),
        );
        assert_eq!(x, {
            let mut y = y;
            y.as_object_mut().unwrap().remove("note");
            y
        });
    }
}

#[test]
fn an_integer_total_beside_line_items_alone_is_a_count() {
    let mut s = order(
        json!({ "type": "integer", "minimum": 1, "maximum": 500 }),
        json!({}),
    );
    for k in ["subtotal", "tax", "shipping", "discount"] {
        s["properties"].as_object_mut().unwrap().remove(k);
    }
    let plain = without_items(&s);
    for seed in 0..100 {
        let v = generate(&s, None, seed, &Options::default());
        assert_eq!(
            v["total"],
            generate(&plain, None, seed, &Options::default())["total"]
        );
    }
}

#[test]
fn a_float_total_beside_line_items_alone_is_summed() {
    let mut s = order(
        json!({ "type": "number", "minimum": 1, "maximum": 500 }),
        json!({}),
    );
    for k in ["subtotal", "tax", "shipping", "discount"] {
        s["properties"].as_object_mut().unwrap().remove(k);
    }
    let v = generate(&s, None, 3, &Options::default());
    let sum: i128 = v["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| cents(&l["lineTotal"]))
        .sum();
    assert_eq!(cents(&v["total"]), sum, "{v}");
}

#[test]
fn a_non_numeric_pinned_field_skips_the_object_before_anything_is_written() {
    let mut s = order(json!({ "type": "number" }), json!({}));
    s["properties"]["tax"] = json!({ "type": "string", "example": "n/a" });
    let plain = without_items(&s);
    for seed in 0..30 {
        let v = generate(&s, None, seed, &Options::default());
        let o = Options::default();
        assert_eq!(
            v["subtotal"],
            generate(&plain, None, seed, &o)["subtotal"],
            "{v}"
        );
        // no line amount was rewritten either: a random `lineTotal` is not price x quantity
        for line in v["items"].as_array().unwrap() {
            let sum = cents(&line["price"]) * i128::from(line["quantity"].as_i64().unwrap());
            assert_ne!(cents(&line["lineTotal"]), sum, "{v}");
        }
    }
}

#[test]
fn a_total_that_must_be_a_multiple_is_left_alone_when_the_sum_is_not_one() {
    let mut s = order(
        json!({ "type": "integer", "minimum": 1, "maximum": 500 }),
        json!({}),
    );
    s["properties"]["total"] = json!({ "type": "integer", "multipleOf": 7919 });
    for seed in 0..100 {
        let v = generate(&s, None, seed, &Options::default());
        assert_eq!(v["total"].as_i64().unwrap() % 7919, 0, "{v}");
    }
}
