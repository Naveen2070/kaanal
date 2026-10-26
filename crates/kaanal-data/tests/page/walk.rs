//! The oracle (spec §7): walking every page equals the record sequence generated one by one.

use kaanal_data::{
    Endpoint, Options, PageSpec, QueryParam, Style, generate, generate_page, item_seed,
};
use serde_json::{Value, json};
use std::collections::HashSet;

const SEED: u64 = 0xd3;
const MAX: u32 = 100;

fn item() -> Value {
    json!({"type": "object", "properties": {
        "id": {"type": "string", "format": "uuid"},
        "name": {"type": "string"},
        "tags": {"type": "array", "items": {"type": "string"}}
    }})
}

fn envelope() -> Value {
    json!({"type": "object", "properties": {
        "data": {"type": "array", "items": item()},
        "total": {"type": "integer"},
        "hasMore": {"type": "boolean"},
        "nextCursor": {"type": "string"}
    }})
}

fn spec(schema: &Value, style: Style) -> PageSpec {
    let index = match style {
        Style::Page => "page",
        Style::Offset => "offset",
        Style::Cursor => "cursor",
    };
    let qp = |name: &str, maximum| QueryParam {
        name: name.into(),
        minimum: None,
        maximum,
        default: None,
        numeric: true,
    };
    PageSpec::detect(&Endpoint {
        method: "GET".into(),
        path: "/things".into(),
        status: 200,
        schema: Some(schema.clone()),
        query_params: vec![qp(index, None), qp("limit", Some(i64::from(MAX)))],
    })
    .expect("paged")
}

fn fetch(schema: &Value, s: &PageSpec, query: &[(&str, String)], n: u32) -> Value {
    let q: Vec<(String, String)> = query
        .iter()
        .map(|(k, v)| ((*k).into(), v.clone()))
        .collect();
    let w = s.window(&q, SEED, Some(n)).unwrap();
    generate_page(schema, s, &w, SEED, &Options::default(), "/things", &q)
}

/// Records `0..n`, generated one by one.
fn sequence(n: u32) -> Vec<Value> {
    (0..n)
        .map(|i| generate(&item(), None, item_seed(SEED, i), &Options::default()))
        .collect()
}

/// Every page of the envelope schema, walked by its own style, concatenated.
fn walk_envelope(style: Style, size: u32, n: u32) -> Vec<Value> {
    let schema = envelope();
    let s = spec(&schema, style);
    let mut out = Vec::new();
    let (mut page, mut cursor) = (1u32, None::<String>);
    for _ in 0..=n + 1 {
        let mut q = vec![("limit", size.to_string())];
        match style {
            Style::Page => q.push(("page", page.to_string())),
            Style::Offset => q.push(("offset", out.len().to_string())),
            Style::Cursor => q.extend(cursor.clone().map(|c| ("cursor", c))),
        }
        let body = fetch(&schema, &s, &q, n);
        assert_eq!(body["total"], n, "total is constant on every page");
        out.extend(body["data"].as_array().unwrap().iter().cloned());
        let more = body["hasMore"].as_bool().unwrap();
        assert_eq!(more, body["nextCursor"].is_string(), "{body}");
        if !more {
            assert_eq!(out.len(), n as usize, "the last page has no next");
            return out;
        }
        page += 1;
        cursor = body["nextCursor"].as_str().map(String::from);
    }
    panic!("walk did not end: style {style:?} size {size} n {n}");
}

/// Every page of the plain array schema, until a short page.
fn walk_array(style: Style, size: u32, n: u32) -> Vec<Value> {
    let schema = json!({"type": "array", "items": item()});
    let s = spec(&schema, style);
    let mut out: Vec<Value> = Vec::new();
    for p in 1..=n + 1 {
        let q = match style {
            Style::Page => ("page", p.to_string()),
            Style::Offset => ("offset", out.len().to_string()),
            Style::Cursor => (
                "cursor",
                kaanal_data::encode_cursor(u32::try_from(out.len()).unwrap()),
            ),
        };
        let body = fetch(&schema, &s, &[q, ("limit", size.to_string())], n);
        let got = body.as_array().unwrap();
        out.extend(got.iter().cloned());
        if got.len() < size as usize {
            return out;
        }
    }
    panic!("walk did not end: style {style:?} size {size} n {n}");
}

#[test]
fn walking_all_pages_equals_the_sequence() {
    let mut combos = 0u32;
    for (k, size) in (0u32..).zip([1, 3, 20, MAX]) {
        for style in [Style::Page, Style::Offset, Style::Cursor] {
            // seeded: a total that is no edge case
            let random = u32::try_from(item_seed(SEED, k * 3 + combos) % 138).unwrap();
            for n in [0, 1, size - 1, size, size + 1, random] {
                let want = sequence(n);
                let distinct: HashSet<String> = want.iter().map(Value::to_string).collect();
                assert_eq!(distinct.len(), want.len(), "no duplicates, n {n}");
                assert_eq!(walk_envelope(style, size, n), want, "{style:?} {size} {n}");
                assert_eq!(walk_array(style, size, n), want, "{style:?} {size} {n}");
                combos += 1;
            }
        }
    }
    assert_eq!(combos, 72);
}

#[test]
fn record_i_is_identical_across_sizes_and_styles() {
    let n = 47;
    let want = sequence(n);
    for size in [1, 3, 7, 20, MAX] {
        for style in [Style::Page, Style::Offset, Style::Cursor] {
            assert_eq!(walk_envelope(style, size, n), want, "{style:?} {size}");
        }
    }
    // an offset that is no multiple of the size still lands on the same records
    let schema = json!({"type": "array", "items": item()});
    let s = spec(&schema, Style::Offset);
    let q = [("offset", "5".to_string()), ("limit", "3".to_string())];
    assert_eq!(fetch(&schema, &s, &q, n), Value::Array(want[5..8].to_vec()));
    // and the same page asked twice is the same bytes
    assert_eq!(fetch(&schema, &s, &q, n), fetch(&schema, &s, &q, n));
}

/// JSON values in `v`, each object, array and scalar counted once (the generator's unit).
fn values(v: &Value) -> usize {
    1 + match v {
        Value::Array(a) => a.iter().map(values).sum(),
        Value::Object(o) => o.values().map(values).sum(),
        _ => 0,
    }
}

#[test]
fn heavy_records_shrink_the_page_and_walk_without_gaps() {
    // up to 1 + 1 + 100 values a record, so 10 000 records would be ~10^6 values
    let heavy = json!({"type": "object", "properties": {
        "vals": {"type": "array", "minItems": 100, "maxItems": 100, "items": {"type": "integer"}}
    }});
    let total = 250;
    let want: Vec<Value> = (0..total)
        .map(|i| generate(&heavy, None, item_seed(SEED, i), &Options::default()))
        .collect();
    for (style, index) in [(Style::Offset, "offset"), (Style::Page, "page")] {
        let schema = json!({"type": "array", "items": heavy});
        let qp = |name: &str, maximum| QueryParam {
            name: name.into(),
            minimum: None,
            maximum,
            default: None,
            numeric: true,
        };
        let spec = PageSpec::detect(&Endpoint {
            method: "GET".into(),
            path: "/things".into(),
            status: 200,
            schema: Some(schema.clone()),
            query_params: vec![qp(index, None), qp("limit", Some(10_000))],
        })
        .unwrap();
        assert_eq!(spec.style, style);
        let mut walked: Vec<Value> = Vec::new();
        let mut at = match style {
            Style::Page => "1".to_string(),
            _ => "0".to_string(),
        };
        loop {
            let q = vec![
                ("limit".to_string(), "10000".to_string()),
                (index.to_string(), at.clone()),
            ];
            let win = spec.window(&q, SEED, Some(total)).unwrap();
            assert_eq!(
                win.size,
                10_000 / 102,
                "{style:?}: shrunk to fit the value cap"
            );
            let body = generate_page(
                &schema,
                &spec,
                &win,
                SEED,
                &Options::default(),
                "/things",
                &q,
            );
            assert!(
                values(&body) <= 10_000,
                "{style:?}: {} values",
                values(&body)
            );
            walked.extend(body.as_array().unwrap().iter().cloned());
            let h = win.headers(&spec, "/things", &q);
            assert!(h.contains(&("x-total-count", total.to_string())));
            let link = h.iter().find(|(k, _)| *k == "link").unwrap().1.clone();
            // follow rel="next": it continues exactly where this page ended
            let Some(next) = link
                .split(", ")
                .find(|l| l.ends_with("rel=\"next\""))
                .and_then(|l| l.split(&format!("{index}=")).nth(1))
                .and_then(|l| l.split('>').next())
            else {
                break;
            };
            if style == Style::Offset {
                assert_eq!(next, walked.len().to_string());
            }
            at = next.to_string();
        }
        assert_eq!(walked, want, "{style:?}: no gaps, no duplicates");
    }
}
