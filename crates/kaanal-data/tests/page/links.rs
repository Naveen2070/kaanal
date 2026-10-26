//! `x-total-count` and `Link` headers of a paged response.

use kaanal_data::{Envelope, PageSpec, Style, Window, encode_cursor};

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

fn win(total: u32, offset: u32, size: u32) -> Window {
    Window {
        total,
        offset,
        size,
    }
}

fn header(h: &[(&'static str, String)], name: &str) -> Option<String> {
    h.iter().find(|(k, _)| *k == name).map(|(_, v)| v.clone())
}

fn link(w: Window, s: &PageSpec, query: &[(&str, &str)]) -> String {
    header(&w.headers(s, "/things", &q(query)), "link").unwrap_or_default()
}

#[test]
fn link_has_first_prev_next_last_for_a_middle_page() {
    let h = win(95, 20, 20).headers(&spec(Style::Page), "/things", &q(&[("page", "2")]));
    assert_eq!(header(&h, "x-total-count").as_deref(), Some("95"));
    assert_eq!(
        header(&h, "link").unwrap(),
        "</things?page=1>; rel=\"first\", </things?page=1>; rel=\"prev\", \
         </things?page=3>; rel=\"next\", </things?page=5>; rel=\"last\""
    );
}

#[test]
fn no_prev_on_first_and_no_next_on_last() {
    let s = spec(Style::Page);
    assert_eq!(
        link(win(95, 0, 20), &s, &[]),
        "</things?page=1>; rel=\"first\", </things?page=2>; rel=\"next\", </things?page=5>; rel=\"last\""
    );
    assert_eq!(
        link(win(95, 80, 20), &s, &[("page", "5")]),
        "</things?page=1>; rel=\"first\", </things?page=4>; rel=\"prev\", </things?page=5>; rel=\"last\""
    );
}

#[test]
fn other_query_params_are_kept_and_paging_ones_replaced() {
    let l = link(
        win(95, 20, 20),
        &spec(Style::Page),
        &[("status", "open"), ("page", "2"), ("limit", "20")],
    );
    assert!(
        l.contains("</things?status=open&limit=20&page=3>; rel=\"next\""),
        "{l}"
    );
    assert!(!l.contains("page=2"), "{l}");
}

#[test]
fn cursor_style_links_use_the_cursor_param() {
    let s = spec(Style::Cursor);
    let l = link(win(95, 0, 20), &s, &[("status", "open")]);
    assert_eq!(
        l,
        format!(
            "</things?status=open&cursor={}>; rel=\"next\"",
            encode_cursor(20)
        )
    );
    assert_eq!(link(win(95, 80, 20), &s, &[]), "");
}

#[test]
fn values_are_percent_encoded() {
    let l = link(
        win(95, 0, 20),
        &spec(Style::Page),
        &[("q", "a b&c=d/é"), ("tag", "x~y-z_.")],
    );
    assert!(
        l.contains("q=a%20b%26c%3Dd%2F%C3%A9&tag=x~y-z_.&page=2"),
        "{l}"
    );
}

#[test]
fn empty_dataset_has_first_only_and_total_zero() {
    let h = win(0, 0, 20).headers(&spec(Style::Page), "/things", &[]);
    assert_eq!(header(&h, "x-total-count").as_deref(), Some("0"));
    assert_eq!(
        header(&h, "link").unwrap(),
        "</things?page=1>; rel=\"first\""
    );
}

#[test]
fn past_the_end_links_point_back_to_real_pages() {
    let l = link(win(95, 200, 20), &spec(Style::Page), &[("page", "11")]);
    assert!(l.contains("page=1>; rel=\"first\""), "{l}");
    assert!(l.contains("page=5>; rel=\"prev\""), "{l}");
    assert!(l.contains("page=5>; rel=\"last\""), "{l}");
    assert!(!l.contains("rel=\"next\""), "{l}");
}

#[test]
fn offset_style_links() {
    let s = spec(Style::Offset);
    assert_eq!(
        link(win(95, 20, 20), &s, &[("offset", "20")]),
        "</things?offset=0>; rel=\"first\", </things?offset=0>; rel=\"prev\", \
         </things?offset=40>; rel=\"next\", </things?offset=80>; rel=\"last\""
    );
    let l = link(win(95, 10, 20), &s, &[]);
    assert!(l.contains("offset=0>; rel=\"prev\""), "{l}");
}

#[test]
fn u32_max_does_not_overflow() {
    for st in [Style::Page, Style::Offset, Style::Cursor] {
        let s = spec(st);
        let a = win(u32::MAX, u32::MAX, 1).headers(&s, "/things", &[]);
        assert_eq!(header(&a, "x-total-count"), Some(u32::MAX.to_string()));
        let _ = win(u32::MAX, 0, u32::MAX).headers(&s, "/things", &[]);
    }
    assert_eq!(
        link(win(u32::MAX, 0, u32::MAX), &spec(Style::Page), &[]),
        "</things?page=1>; rel=\"first\", </things?page=1>; rel=\"last\""
    );
}

#[test]
fn offset_style_prev_past_the_end_is_clamped_to_last() {
    let l = link(win(95, 200, 20), &spec(Style::Offset), &[("offset", "200")]);
    assert_eq!(
        l,
        "</things?offset=0>; rel=\"first\", </things?offset=80>; rel=\"prev\", </things?offset=80>; rel=\"last\""
    );
}

#[test]
fn duplicated_index_param_is_replaced_once() {
    let l = link(
        win(95, 20, 20),
        &spec(Style::Page),
        &[("page", "2"), ("q", "x"), ("page", "3")],
    );
    assert_eq!(
        l,
        "</things?q=x&page=1>; rel=\"first\", </things?q=x&page=1>; rel=\"prev\", </things?q=x&page=3>; rel=\"next\", </things?q=x&page=5>; rel=\"last\""
    );
}

#[test]
fn size_zero_is_treated_as_one() {
    assert_eq!(
        link(win(3, 1, 0), &spec(Style::Page), &[]),
        "</things?page=1>; rel=\"first\", </things?page=1>; rel=\"prev\", </things?page=3>; rel=\"next\", </things?page=3>; rel=\"last\""
    );
}

#[test]
fn cursor_style_with_total_zero_has_no_link() {
    let h = win(0, 0, 20).headers(&spec(Style::Cursor), "/things", &[]);
    assert_eq!(header(&h, "x-total-count").as_deref(), Some("0"));
    assert_eq!(header(&h, "link"), None);
}

#[test]
fn zero_based_page_links_start_at_page_zero() {
    let mut s = spec(Style::Page);
    s.page_base = 0;
    assert_eq!(
        link(win(95, 20, 20), &s, &[("page", "1")]),
        "</things?page=0>; rel=\"first\", </things?page=0>; rel=\"prev\", \
         </things?page=2>; rel=\"next\", </things?page=4>; rel=\"last\""
    );
}
