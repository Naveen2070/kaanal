//! Style detection from an endpoint's query parameters and response schema.

use kaanal_data::{Endpoint, Envelope, PageSpec, QueryParam, Style};
use serde_json::{Value, json};

fn qp(name: &str) -> QueryParam {
    QueryParam {
        name: name.into(),
        minimum: None,
        maximum: None,
        default: None,
        numeric: true,
    }
}

fn sqp(name: &str) -> QueryParam {
    QueryParam {
        numeric: false,
        ..qp(name)
    }
}

fn ep(method: &str, params: Vec<QueryParam>, schema: Value) -> Endpoint {
    Endpoint {
        method: method.into(),
        path: "/things".into(),
        status: 200,
        schema: Some(schema),
        query_params: params,
        file_in: false,
        file_out: false,
    }
}

fn get(names: &[&str]) -> Endpoint {
    ep(
        "GET",
        names.iter().map(|n| qp(n)).collect(),
        json!({"type": "array", "items": {"type": "string"}}),
    )
}

#[test]
fn page_param_gives_page_style() {
    let s = PageSpec::detect(&get(&["page", "limit"])).unwrap();
    assert_eq!(s.style, Style::Page);
    assert_eq!(s.index_param, "page");
    assert_eq!(s.size_param.as_deref(), Some("limit"));
}

#[test]
fn offset_and_skip_give_offset_style() {
    for n in ["offset", "skip"] {
        let s = PageSpec::detect(&get(&[n])).unwrap();
        assert_eq!(s.style, Style::Offset);
        assert_eq!(s.index_param, n);
    }
}

#[test]
fn cursor_after_pagetoken_give_cursor_style() {
    for n in [
        "cursor",
        "after",
        "pageToken",
        "next_page_token",
        "starting_after",
    ] {
        assert_eq!(
            PageSpec::detect(&get(&[n])).unwrap().style,
            Style::Cursor,
            "{n}"
        );
    }
}

#[test]
fn names_are_normalised() {
    for n in ["per_page", "perPage", "Page-Size"] {
        let s = PageSpec::detect(&get(&["page", n])).unwrap();
        assert_eq!(s.size_param.as_deref(), Some(n));
    }
}

#[test]
fn offset_beats_page_when_both_declared() {
    let s = PageSpec::detect(&get(&["page", "offset"])).unwrap();
    assert_eq!(s.style, Style::Offset);
}

#[test]
fn no_recognised_param_is_none() {
    assert!(PageSpec::detect(&get(&["q", "sort"])).is_none());
    assert!(PageSpec::detect(&get(&["limit"])).is_none());
}

#[test]
fn size_defaults_and_bounds() {
    let s = PageSpec::detect(&get(&["page", "limit"])).unwrap();
    assert_eq!((s.size_default, s.size_min, s.size_max), (20, 1, 100));
    let mut limit = qp("limit");
    limit.default = Some(10);
    limit.maximum = Some(50);
    limit.minimum = Some(5);
    let e = ep("GET", vec![qp("page"), limit], json!({"type": "array"}));
    let s = PageSpec::detect(&e).unwrap();
    assert_eq!((s.size_default, s.size_min, s.size_max), (10, 5, 50));
}

#[test]
fn non_get_is_none() {
    let e = ep("POST", vec![qp("page")], json!({"type": "array"}));
    assert!(PageSpec::detect(&e).is_none());
}

fn totals(schema: Value) -> (u32, u32) {
    let s = PageSpec::detect(&ep("GET", vec![qp("page")], schema)).unwrap();
    (s.total_min, s.total_max)
}

#[test]
fn array_min_max_items_set_total_bounds() {
    assert_eq!(
        totals(json!({"type": "array", "minItems": 30, "maxItems": 40})),
        (30, 40)
    );
    assert_eq!(totals(json!({"type": "array", "maxItems": 0})), (0, 0));
    assert_eq!(
        totals(json!({"type": "array", "maxItems": 99_999_999})),
        (23, 10_000)
    );
    assert_eq!(totals(json!({"type": "array"})), (23, 137));
    // an object envelope takes the bounds of its paged array, not its own
    assert_eq!(
        totals(json!({"type": "object", "minItems": 1, "properties": {
            "data": {"type": "array", "minItems": 30, "maxItems": 40}
        }})),
        (30, 40)
    );
}

#[test]
fn object_without_array_property_is_none() {
    for s in [
        json!({"type": "object"}),
        json!({"type": "object", "properties": {"n": {"type": "integer"}}}),
    ] {
        assert!(PageSpec::detect(&ep("GET", vec![qp("page")], s)).is_none());
    }
}

#[test]
fn typeless_object_with_properties_is_paged() {
    let s = json!({"properties": {"total": {"type": "integer"}, "data": {"type": "array"}}});
    let p = PageSpec::detect(&ep("GET", vec![qp("page")], s)).unwrap();
    assert_eq!(
        p.envelope,
        Envelope::Object {
            items: "data".into()
        }
    );
}

#[test]
fn all_of_envelope_is_paged() {
    let s = json!({"allOf": [
        {"type": "object", "properties": {"total": {"type": "integer"}}},
        {"properties": {"results": {"type": "array", "maxItems": 50}}}
    ]});
    let p = PageSpec::detect(&ep("GET", vec![qp("page")], s)).unwrap();
    assert_eq!(
        p.envelope,
        Envelope::Object {
            items: "results".into()
        }
    );
    assert_eq!((p.total_min, p.total_max), (23, 50));
}

#[test]
fn array_schema_has_array_envelope() {
    assert_eq!(
        PageSpec::detect(&get(&["page"])).unwrap().envelope,
        Envelope::Array
    );
}

#[test]
fn scalar_schema_is_none() {
    let e = ep("GET", vec![qp("page")], json!({"type": "string"}));
    assert!(PageSpec::detect(&e).is_none());
}

fn get_with(params: Vec<QueryParam>) -> Endpoint {
    ep(
        "GET",
        params,
        json!({"type": "array", "items": {"type": "string"}}),
    )
}

#[test]
fn string_cursor_gives_cursor_style() {
    let s = PageSpec::detect(&get_with(vec![sqp("cursor"), qp("limit")])).unwrap();
    assert_eq!(s.style, Style::Cursor);
    assert_eq!(s.index_param, "cursor");
    assert_eq!(s.size_param.as_deref(), Some("limit"));
}

#[test]
fn string_page_param_is_not_page_style() {
    assert!(PageSpec::detect(&get_with(vec![sqp("page")])).is_none());
    assert!(PageSpec::detect(&get_with(vec![sqp("offset")])).is_none());
    // a string size param is not a size param
    let s = PageSpec::detect(&get_with(vec![qp("page"), sqp("limit")])).unwrap();
    assert_eq!(s.size_param, None);
}

#[test]
fn cursor_beats_offset_and_page() {
    let s = PageSpec::detect(&get(&["page", "offset", "cursor"])).unwrap();
    assert_eq!((s.style, s.index_param.as_str()), (Style::Cursor, "cursor"));
}

#[test]
fn index_and_offset_aliases_are_normalised() {
    for n in ["Page", "page_number", "PageNumber"] {
        let s = PageSpec::detect(&get(&[n])).unwrap();
        assert_eq!((s.style, s.index_param.as_str()), (Style::Page, n));
    }
    let s = PageSpec::detect(&get(&["Skip"])).unwrap();
    assert_eq!((s.style, s.index_param.as_str()), (Style::Offset, "Skip"));
}

#[test]
fn size_aliases_are_recognised() {
    for n in ["size", "count", "maxResults"] {
        let s = PageSpec::detect(&get(&["page", n])).unwrap();
        assert_eq!(s.size_param.as_deref(), Some(n));
    }
}

#[test]
fn lowercase_get_is_detected() {
    let e = ep("get", vec![qp("page")], json!({"type": "array"}));
    assert!(PageSpec::detect(&e).is_some());
}

#[test]
fn min_items_only_raises_both_bounds() {
    assert_eq!(
        totals(json!({"type": "array", "minItems": 200})),
        (200, 200)
    );
    assert_eq!(totals(json!({"type": "array", "minItems": 50})), (50, 137));
}

#[test]
fn max_items_only_lowers_both_bounds() {
    assert_eq!(totals(json!({"type": "array", "maxItems": 10})), (10, 10));
    assert_eq!(totals(json!({"type": "array", "maxItems": 100})), (23, 100));
}

#[test]
fn min_items_above_max_items_wins_like_the_generator() {
    assert_eq!(
        totals(json!({"type": "array", "minItems": 5, "maxItems": 2})),
        (5, 5)
    );
}

#[test]
fn page_base_follows_the_declared_minimum_or_default() {
    let base = |minimum, default| {
        let page = QueryParam {
            minimum,
            default,
            ..qp("page")
        };
        PageSpec::detect(&get_with(vec![page, qp("size")]))
            .unwrap()
            .page_base
    };
    assert_eq!(base(Some(0), None), 0, "minimum 0: Spring-style 0-based");
    assert_eq!(base(None, Some(0)), 0, "default 0 and no minimum");
    assert_eq!(base(Some(-1), Some(0)), 0, "default 0, minimum not above 0");
    assert_eq!(base(Some(1), Some(0)), 1, "a minimum above 0 wins");
    assert_eq!(base(None, None), 1);
    assert_eq!(base(Some(1), None), 1);
    assert_eq!(base(None, Some(1)), 1);
    // only the page style has a page index
    let off = QueryParam {
        minimum: Some(0),
        ..qp("offset")
    };
    assert_eq!(PageSpec::detect(&get_with(vec![off])).unwrap().page_base, 1);
}

#[test]
fn record_cap_keeps_a_page_within_the_value_budget() {
    // 1 (record) + 1 (array) + 100 integers = at most 102 values per record
    let heavy = json!({"type": "object", "properties": {
        "vals": {"type": "array", "minItems": 100, "maxItems": 100, "items": {"type": "integer"}}
    }});
    let cap = |schema: Value| {
        PageSpec::detect(&ep("GET", vec![qp("offset")], schema))
            .unwrap()
            .record_cap
    };
    assert_eq!(cap(json!({"type": "array", "items": heavy})), 10_000 / 102);
    // an object envelope's own values (object, emptied array, total) come first
    let env = json!({"type": "object", "properties": {
        "data": {"type": "array", "items": heavy},
        "total": {"type": "integer"}
    }});
    assert_eq!(cap(env), (10_000 - 3) / 102);
    assert_eq!(
        cap(json!({"type": "array", "items": {"type": "string"}})),
        10_000
    );
}
