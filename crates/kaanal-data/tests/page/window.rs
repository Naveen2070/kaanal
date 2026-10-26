//! Window resolution: query parameters and seed to (total, offset, size).

use kaanal_data::{Envelope, PageSpec, Style, decode_cursor, encode_cursor};

fn spec(style: Style) -> PageSpec {
    PageSpec {
        style,
        index_param: match style {
            Style::Page => "page",
            Style::Offset => "offset",
            Style::Cursor => "cursor",
        }
        .into(),
        size_param: Some("limit".into()),
        size_default: 20,
        size_min: 1,
        size_max: 100,
        total_min: 23,
        total_max: 137,
        envelope: Envelope::Array,
        page_base: 1,
        record_cap: 10_000,
    }
}

fn q(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).into(), (*v).into()))
        .collect()
}

fn bad(s: &PageSpec, pairs: &[(&str, &str)], param: &str) {
    let e = s.window(&q(pairs), 7, None).unwrap_err();
    let b = e.body();
    assert_eq!(b["error"], "invalid query parameter");
    assert_eq!(b["param"], param);
    assert!(b["detail"].as_str().is_some_and(|d| !d.is_empty()));
}

#[test]
fn absent_params_give_first_page_default_size() {
    for st in [Style::Page, Style::Offset, Style::Cursor] {
        let w = spec(st).window(&[], 7, None).unwrap();
        assert_eq!((w.offset, w.size), (0, 20));
    }
}

#[test]
fn page_n_maps_to_offset() {
    let w = spec(Style::Page)
        .window(&q(&[("page", "3"), ("limit", "10")]), 7, None)
        .unwrap();
    assert_eq!((w.offset, w.size), (20, 10));
}

#[test]
fn size_above_maximum_is_clamped() {
    let w = spec(Style::Page)
        .window(&q(&[("limit", "5000")]), 7, None)
        .unwrap();
    assert_eq!(w.size, 100);
}

#[test]
fn size_zero_is_400() {
    bad(&spec(Style::Page), &[("limit", "0")], "limit");
}

#[test]
fn page_zero_is_400() {
    bad(&spec(Style::Page), &[("page", "0")], "page");
}

#[test]
fn negative_offset_is_400() {
    bad(&spec(Style::Offset), &[("offset", "-1")], "offset");
}

#[test]
fn non_integer_is_400() {
    bad(&spec(Style::Offset), &[("offset", "abc")], "offset");
    bad(&spec(Style::Page), &[("limit", "1.5")], "limit");
    bad(&spec(Style::Page), &[("page", "")], "page");
}

#[test]
fn size_below_spec_minimum_is_400() {
    let mut s = spec(Style::Page);
    s.size_min = 5;
    bad(&s, &[("limit", "4")], "limit");
    assert_eq!(s.window(&q(&[("limit", "5")]), 7, None).unwrap().size, 5);
}

#[test]
fn page_past_end_is_empty_window() {
    let w = spec(Style::Page)
        .window(&q(&[("page", "9999")]), 7, None)
        .unwrap();
    assert!(w.offset >= w.total);
}

#[test]
fn huge_page_does_not_overflow() {
    let s = spec(Style::Page);
    let w = s
        .window(&q(&[("page", "4294967295"), ("limit", "100")]), 7, None)
        .unwrap();
    assert!(w.offset >= w.total);
    // above u32: a plain 400, never a panic
    bad(&s, &[("page", "99999999999")], "page");
    let o = spec(Style::Offset)
        .window(&q(&[("offset", "4294967295")]), 7, None)
        .unwrap();
    assert!(o.offset >= o.total);
}

#[test]
fn offset_above_u32_says_it_is_too_large() {
    let e = spec(Style::Offset)
        .window(&q(&[("offset", "99999999999")]), 7, None)
        .unwrap_err();
    let d = e.body()["detail"].as_str().unwrap().to_string();
    assert_eq!(
        d,
        "`99999999999` exceeds the maximum supported value (4294967295)"
    );
    // a sign or a letter is still "not a non-negative integer"
    let e = spec(Style::Offset)
        .window(&q(&[("offset", "-1")]), 7, None)
        .unwrap_err();
    assert_eq!(e.body()["detail"], "`-1` is not a non-negative integer");
}

#[test]
fn zero_based_page_maps_page_zero_to_offset_zero() {
    let mut s = spec(Style::Page);
    s.page_base = 0;
    let off = |p: &str| {
        s.window(&q(&[("page", p), ("limit", "10")]), 7, None)
            .unwrap()
            .offset
    };
    assert_eq!((off("0"), off("1"), off("3")), (0, 10, 30));
    bad(&s, &[("page", "-1")], "page");
    // a 1-based route still refuses page=0
    let e = spec(Style::Page)
        .window(&q(&[("page", "0")]), 7, None)
        .unwrap_err();
    assert_eq!(e.body()["detail"], "must be at least 1");
}

#[test]
fn record_cap_shrinks_the_page_size() {
    let size = |cap: u32, p: &[(&str, &str)]| {
        let mut s = spec(Style::Offset);
        s.record_cap = cap;
        s.window(&q(p), 7, None).unwrap().size
    };
    assert_eq!(size(7, &[("limit", "100")]), 7);
    assert_eq!(size(7, &[]), 7);
    assert_eq!(size(7, &[("limit", "3")]), 3);
    // never below one record
    assert_eq!(size(0, &[("limit", "100")]), 1);
}

#[test]
fn offset_wins_over_page() {
    let w = spec(Style::Offset)
        .window(&q(&[("page", "9"), ("offset", "5")]), 7, None)
        .unwrap();
    assert_eq!(w.offset, 5);
}

#[test]
fn cursor_round_trip() {
    for o in [0, 1, 20, 137, 10_000, u32::MAX] {
        assert_eq!(decode_cursor(&encode_cursor(o)), Some(o));
    }
    assert_eq!(encode_cursor(20), "azE6MjA");
    let c = encode_cursor(40);
    let w = spec(Style::Cursor)
        .window(&q(&[("cursor", &c)]), 7, None)
        .unwrap();
    assert_eq!(w.offset, 40);
}

#[test]
fn forged_cursor_is_400() {
    // base64url of "x:5" and of "k1:4294967296"
    for c in ["!!", "", "eDo1", "azE6NDI5NDk2NzI5Ng"] {
        assert_eq!(decode_cursor(c), None, "{c}");
        bad(&spec(Style::Cursor), &[("cursor", c)], "cursor");
    }
}

#[test]
fn total_is_stable_and_in_bounds() {
    let s = spec(Style::Page);
    let t = |seed| s.window(&[], seed, None).unwrap().total;
    assert_eq!(t(42), t(42));
    for seed in 0..200 {
        assert!((23..=137).contains(&t(seed)), "seed {seed}");
    }
}

#[test]
fn total_override_wins_and_is_capped_at_10000() {
    let s = spec(Style::Page);
    assert_eq!(s.window(&[], 7, Some(5)).unwrap().total, 5);
    assert_eq!(s.window(&[], 7, Some(0)).unwrap().total, 0);
    assert_eq!(s.window(&[], 7, Some(50_000)).unwrap().total, 10_000);
}

#[test]
fn total_respects_min_max_items() {
    let mut s = spec(Style::Page);
    (s.total_min, s.total_max) = (5, 5);
    assert_eq!(s.window(&[], 7, None).unwrap().total, 5);
    (s.total_min, s.total_max) = (0, 0);
    assert_eq!(s.window(&[], 7, None).unwrap().total, 0);
}

#[test]
fn size_bounds_are_normalised() {
    let w = |s: &PageSpec, p: &[(&str, &str)]| s.window(&q(p), 7, None).unwrap().size;
    // default above maximum is clamped down
    let mut s = spec(Style::Page);
    (s.size_default, s.size_max) = (50, 30);
    assert_eq!(w(&s, &[]), 30);
    // minimum 0 means 1: limit=0 is still 400
    let mut s = spec(Style::Page);
    s.size_min = 0;
    bad(&s, &[("limit", "0")], "limit");
    assert_eq!(w(&s, &[("limit", "1")]), 1);
    // negative maximum (clamped to 0 by detect) never leaves max below min
    let mut s = spec(Style::Page);
    (s.size_max, s.size_default) = (0, 0);
    assert_eq!(w(&s, &[]), 1);
    assert_eq!(w(&s, &[("limit", "9")]), 1);
}

#[test]
fn non_canonical_cursor_is_400() {
    // "azE6MjB" decodes to "k1:20" like "azE6MjA" but sets the unused trailing bits;
    // "azE6MDA3" is "k1:007" (leading zeros); the encoder writes neither.
    for c in ["azE6MjB", "azE6MDA3"] {
        assert_eq!(decode_cursor(c), None, "{c}");
        bad(&spec(Style::Cursor), &[("cursor", c)], "cursor");
    }
    assert_eq!(decode_cursor("azE6MjA"), Some(20));
}

#[test]
fn repeated_query_key_takes_the_first() {
    let w = spec(Style::Offset)
        .window(&q(&[("limit", "5"), ("limit", "7")]), 7, None)
        .unwrap();
    assert_eq!(w.size, 5);
}

#[test]
fn only_ascii_digits_are_accepted() {
    for v in ["+5", " 5", "5 ", "\u{0665}"] {
        bad(&spec(Style::Offset), &[("limit", v)], "limit");
        bad(&spec(Style::Offset), &[("offset", v)], "offset");
    }
}
