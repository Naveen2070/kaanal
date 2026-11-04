//! Envelope detection and page generation: the slice, and meta siblings filled consistently.

use kaanal_data::{
    Endpoint, Envelope, Options, PageSpec, QueryParam, encode_cursor, generate, generate_page,
    item_seed,
};
use serde_json::{Value, json};

const SEED: u64 = 0x5eed;

fn qp(name: &str) -> QueryParam {
    QueryParam {
        name: name.into(),
        minimum: None,
        maximum: None,
        default: None,
        numeric: true,
    }
}

fn spec(schema: &Value, params: &[&str]) -> PageSpec {
    PageSpec::detect(&Endpoint {
        method: "GET".into(),
        path: "/things".into(),
        status: 200,
        schema: Some(schema.clone()),
        query_params: params.iter().map(|n| qp(n)).collect(),
        file_in: false,
        file_out: false,
    })
    .expect("paged")
}

/// The page `query` addresses, with the total forced to `total`.
fn page(schema: &Value, params: &[&str], query: &[(&str, &str)], total: u32) -> Value {
    let s = spec(schema, params);
    let q: Vec<(String, String)> = query
        .iter()
        .map(|(k, v)| ((*k).into(), (*v).into()))
        .collect();
    let w = s.window(&q, SEED, Some(total)).unwrap();
    generate_page(schema, &s, &w, SEED, &Options::default(), "/things", &q)
}

fn record(items: &Value, i: u32) -> Value {
    generate(items, None, item_seed(SEED, i), &Options::default())
}

/// What the generator gives the whole schema today (unpaged).
fn today(schema: &Value) -> Value {
    generate(schema, None, SEED, &Options::default())
}

fn item() -> Value {
    json!({"type": "object", "properties": {
        "id": {"type": "string", "format": "uuid"},
        "name": {"type": "string"}
    }})
}

#[test]
fn array_schema_returns_the_slice() {
    let schema = json!({"type": "array", "items": item()});
    let body = page(
        &schema,
        &["page", "limit"],
        &[("page", "2"), ("limit", "10")],
        47,
    );
    let want: Vec<Value> = (10..20).map(|i| record(&item(), i)).collect();
    assert_eq!(body, Value::Array(want));
    let last = page(
        &schema,
        &["page", "limit"],
        &[("page", "5"), ("limit", "10")],
        47,
    );
    assert_eq!(last.as_array().unwrap().len(), 7);
    let past = page(
        &schema,
        &["page", "limit"],
        &[("page", "6"), ("limit", "10")],
        47,
    );
    assert_eq!(past, json!([]));
}

fn meta_schema() -> Value {
    json!({"type": "object", "properties": {
        "data": {"type": "array", "items": item()},
        "total": {"type": "integer"},
        "page": {"type": "integer"},
        "pageSize": {"type": "integer"},
        "totalPages": {"type": "integer"},
        "hasMore": {"type": "boolean"}
    }})
}

#[test]
fn object_envelope_fills_meta() {
    let s = meta_schema();
    let params = ["page", "pageSize"];
    let body = page(&s, &params, &[("page", "2"), ("pageSize", "10")], 47);
    assert_eq!(body["page"], 2);
    assert_eq!(body["pageSize"], 10);
    assert_eq!(body["totalPages"], 5);
    assert_eq!(body["hasMore"], true);
    assert_eq!(body["total"], 47);
    assert_eq!(body["data"].as_array().unwrap().len(), 10);
    assert_eq!(body["data"][0], record(&item(), 10));
    // last page, then past the end: metadata stays right
    let last = page(&s, &params, &[("page", "5"), ("pageSize", "10")], 47);
    assert_eq!(last["hasMore"], false);
    assert_eq!(last["data"].as_array().unwrap().len(), 7);
    let past = page(&s, &params, &[("page", "9"), ("pageSize", "10")], 47);
    assert_eq!(past["data"], json!([]));
    assert_eq!((&past["page"], &past["totalPages"]), (&json!(9), &json!(5)));
    assert_eq!(
        (&past["total"], &past["hasMore"]),
        (&json!(47), &json!(false))
    );
    let empty = page(&s, &params, &[], 0);
    assert_eq!(
        (&empty["total"], &empty["totalPages"]),
        (&json!(0), &json!(0))
    );
    assert_eq!(empty["data"], json!([]));
}

#[test]
fn cursor_envelope_fills_next() {
    let s = json!({"type": "object", "properties": {
        "items": {"type": "array", "items": item()},
        "nextCursor": {"type": "string"},
        "prevCursor": {"type": ["string", "null"]}
    }});
    let params = ["cursor", "limit"];
    let first = page(&s, &params, &[], 47);
    assert_eq!(first["items"].as_array().unwrap().len(), 20);
    assert_eq!(first["nextCursor"], encode_cursor(20));
    assert_eq!(first["prevCursor"], Value::Null);
    let c = encode_cursor(40);
    let last = page(&s, &params, &[("cursor", &c)], 47);
    assert_eq!(last["items"].as_array().unwrap().len(), 7);
    assert_eq!(last["items"][0], record(&item(), 40));
    assert_eq!(last["nextCursor"], Value::Null);
    assert_eq!(last["prevCursor"], encode_cursor(20));
}

#[test]
fn count_is_the_total_like_drf() {
    let s = json!({"type": "object", "properties": {
        "results": {"type": "array", "items": item()},
        "count": {"type": "integer"}
    }});
    let body = page(
        &s,
        &["offset", "limit"],
        &[("offset", "40"), ("limit", "10")],
        47,
    );
    assert_eq!(body["count"], 47);
    assert_eq!(body["results"].as_array().unwrap().len(), 7);
}

#[test]
fn count_and_a_total_sibling_both_carry_the_total() {
    let s = json!({"type": "object", "properties": {
        "results": {"type": "array", "items": item()},
        "count": {"type": "integer"},
        "totalCount": {"type": "integer"}
    }});
    let body = page(
        &s,
        &["offset", "limit"],
        &[("offset", "40"), ("limit", "10")],
        47,
    );
    assert_eq!(body["totalCount"], 47);
    assert_eq!(body["count"], 47);
}

#[test]
fn unknown_siblings_are_generated_as_today() {
    let s = json!({"type": "object", "properties": {
        "data": {"type": "array", "items": item()},
        "meta": {"type": "object", "properties": {"region": {"type": "string"}}},
        "name": {"type": "string"},
        "page": {"type": "string"},
        "hasMore": {"type": "integer"}
    }});
    let body = page(&s, &["page"], &[("page", "2")], 47);
    let was = today(&s);
    // unknown names, and known names of the wrong JSON type, are left as generated
    for k in ["meta", "name", "page", "hasMore"] {
        assert_eq!(body[k], was[k], "{k}");
    }
}

#[test]
fn nested_arrays_inside_items_are_untouched() {
    let it = json!({"type": "object", "properties": {
        "tags": {"type": "array", "items": {"type": "string"}, "maxItems": 3}
    }});
    let s = json!({"type": "array", "items": it});
    let body = page(
        &s,
        &["offset", "limit"],
        &[("offset", "5"), ("limit", "4")],
        47,
    );
    let body = body.as_array().unwrap();
    assert_eq!(body.len(), 4);
    for (r, i) in body.iter().zip(5..) {
        assert_eq!(*r, record(&it, i));
        assert!(r["tags"].as_array().unwrap().len() <= 3);
    }
}

#[test]
fn two_array_props_pages_the_first() {
    let arr = json!({"type": "array", "items": item()});
    let s = json!({"type": "object", "properties": {"alpha": arr, "beta": arr}});
    let p = spec(&s, &["page"]);
    assert_eq!(
        p.envelope,
        Envelope::Object {
            items: "alpha".into()
        }
    );
    let body = page(&s, &["page"], &[("page", "2")], 47);
    assert_eq!(body["alpha"].as_array().unwrap().len(), 20);
    assert_eq!(body["beta"], today(&s)["beta"]);
    // a recognised name beats key order
    let s = json!({"type": "object", "properties": {"alpha": arr, "records": arr}});
    assert_eq!(
        spec(&s, &["page"]).envelope,
        Envelope::Object {
            items: "records".into()
        }
    );
}

#[test]
fn named_array_beats_unnamed() {
    let arr = json!({"type": "array", "items": item()});
    // two named candidates and an unnamed one that sorts first: the first named one wins
    let s = json!({"type": "object", "properties": {"alpha": arr, "items": arr, "data": arr}});
    assert_eq!(
        spec(&s, &["page"]).envelope,
        Envelope::Object {
            items: "data".into()
        }
    );
}

#[test]
fn unnamed_arrays_page_the_first_in_key_order() {
    // serde_json maps keep keys sorted (no `preserve_order`): `alpha` wins although `zeta`
    // is written first.
    let arr = json!({"type": "array", "items": item()});
    let s = json!({"type": "object", "properties": {"zeta": arr, "alpha": arr}});
    assert_eq!(
        spec(&s, &["page"]).envelope,
        Envelope::Object {
            items: "alpha".into()
        }
    );
}

#[test]
fn preview_field_is_not_a_prev_meta_field() {
    let s = json!({"type": "object", "properties": {
        "items": {"type": "array", "items": item()},
        "preview": {"type": "string"},
        "previewUrl": {"type": "string"},
        "prevCursor": {"type": "string"},
        "prev_page_token": {"type": "string"}
    }});
    let c = encode_cursor(20);
    let body = page(&s, &["cursor", "limit"], &[("cursor", &c)], 47);
    let was = today(&s);
    assert_eq!(body["preview"], was["preview"]);
    assert_eq!(body["previewUrl"], was["previewUrl"]);
    assert!(body["preview"].is_string());
    assert_eq!(body["prevCursor"], encode_cursor(0));
    assert_eq!(body["prev_page_token"], encode_cursor(0));
}

#[test]
fn min_items_does_not_cost_extra_generation() {
    // The paged array asks for 10 000 records. Generating it for the envelope would spend the
    // whole value budget, leaving the siblings after it (in key order) `null`.
    let s = json!({"type": "object", "properties": {
        "data": {"type": "array", "minItems": 10000, "items": {"type": "string"}},
        "name": {"type": "string"},
        "zone": {"type": "object", "properties": {"id": {"type": "integer"}}}
    }});
    let body = page(&s, &["page"], &[("page", "2")], 10_000);
    assert!(body["name"].is_string(), "{}", body["name"]);
    assert!(body["zone"]["id"].is_i64(), "{}", body["zone"]);
    assert_eq!(body["data"].as_array().unwrap().len(), 20);
}

#[test]
fn total_pages_when_size_exceeds_total() {
    let s = meta_schema();
    let body = page(&s, &["page", "pageSize"], &[("pageSize", "10")], 5);
    assert_eq!(body["totalPages"], 1);
    assert_eq!(body["hasMore"], false);
    assert_eq!(body["data"].as_array().unwrap().len(), 5);
}

#[test]
fn numeric_next_and_prev_are_page_numbers() {
    let s = json!({"type": "object", "properties": {
        "data": {"type": "array", "items": item()},
        "next": {"type": "integer"},
        "prev": {"type": "integer", "nullable": true}
    }});
    let p = ["page", "limit"];
    let mid = page(&s, &p, &[("page", "2"), ("limit", "10")], 47);
    assert_eq!((&mid["next"], &mid["prev"]), (&json!(3), &json!(1)));
    let first = page(&s, &p, &[("limit", "10")], 47);
    assert_eq!((&first["next"], &first["prev"]), (&json!(2), &Value::Null));
    let last = page(&s, &p, &[("page", "5"), ("limit", "10")], 47);
    assert_eq!((&last["next"], &last["prev"]), (&Value::Null, &json!(4)));
}

#[test]
fn huge_offset_meta_does_not_overflow() {
    let s = json!({"type": "object", "properties": {
        "data": {"type": "array", "items": item()},
        "page": {"type": "integer"},
        "prev": {"type": "integer"}
    }});
    let body = page(
        &s,
        &["offset", "limit"],
        &[("offset", "4294967295"), ("limit", "1")],
        47,
    );
    assert_eq!(body["page"], u32::MAX);
    assert_eq!(body["data"], json!([]));
}

#[test]
fn string_next_and_previous_are_urls_for_offset_and_page_styles() {
    // DRF style: `next`/`previous` are the URLs of those pages, the same the Link header carries
    let s = json!({"type": "object", "properties": {
        "results": {"type": "array", "items": item()},
        "next": {"type": "string", "nullable": true},
        "previous": {"type": "string", "nullable": true}
    }});
    let o = ["limit", "offset"];
    let first = page(&s, &o, &[("limit", "10")], 47);
    assert_eq!(first["next"], "/things?limit=10&offset=10");
    assert_eq!(first["previous"], Value::Null);
    let mid = page(&s, &o, &[("limit", "10"), ("offset", "20")], 47);
    assert_eq!(mid["next"], "/things?limit=10&offset=30");
    assert_eq!(mid["previous"], "/things?limit=10&offset=10");
    let last = page(&s, &o, &[("limit", "10"), ("offset", "40")], 47);
    assert_eq!(last["next"], Value::Null);
    let p = ["page", "limit"];
    let two = page(&s, &p, &[("page", "2"), ("limit", "10")], 47);
    assert_eq!(two["next"], "/things?limit=10&page=3");
    assert_eq!(two["previous"], "/things?limit=10&page=1");
}

#[test]
fn zero_based_page_numbers_in_meta() {
    let s = json!({"type": "object", "properties": {
        "content": {"type": "array", "items": item()},
        "number": {"type": "integer"},
        "page": {"type": "integer"},
        "nextPage": {"type": "integer"},
        "prevPage": {"type": "integer"},
        "totalPages": {"type": "integer"}
    }});
    let mut params = vec![qp("page"), qp("size")];
    params[0].minimum = Some(0);
    let spec = PageSpec::detect(&Endpoint {
        method: "GET".into(),
        path: "/things".into(),
        status: 200,
        schema: Some(s.clone()),
        query_params: params,
        file_in: false,
        file_out: false,
    })
    .unwrap();
    assert_eq!(spec.page_base, 0);
    let at = |p: &str| {
        let q = vec![
            ("page".to_string(), p.to_string()),
            ("size".to_string(), "10".to_string()),
        ];
        let w = spec.window(&q, SEED, Some(47)).unwrap();
        generate_page(&s, &spec, &w, SEED, &Options::default(), "/things", &q)
    };
    let first = at("0");
    assert_eq!(first["content"][0], record(&item(), 0));
    assert_eq!(
        (&first["page"], &first["nextPage"], &first["prevPage"]),
        (&json!(0), &json!(1), &Value::Null)
    );
    assert_eq!(first["totalPages"], 5);
    let last = at("4");
    assert_eq!(last["content"].as_array().unwrap().len(), 7);
    assert_eq!(
        (&last["page"], &last["nextPage"], &last["prevPage"]),
        (&json!(4), &Value::Null, &json!(3))
    );
}
