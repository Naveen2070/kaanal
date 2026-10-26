//! Windows over HTTP: walking all pages, the end, invalid input and the unpaged route.

use serde_json::{Value, json};

use crate::common::{fixture, header, start, total};

#[tokio::test]
async fn offset_page_two_matches_slice_of_full_walk() {
    let s = start().await;
    let r = s.get("/items?limit=200").await;
    let n = total(&r);
    let full: Vec<Value> = r.json().await.unwrap();
    assert_eq!(full.len(), n);
    let page2 = s.json("/items?limit=7&offset=7").await;
    assert_eq!(page2.as_array().unwrap()[..], full[7..14]);
    let mut walked = Vec::new();
    let mut offset = 0;
    while offset < n {
        let page = s.json(&format!("/items?limit=7&offset={offset}")).await;
        walked.extend(page.as_array().unwrap().iter().cloned());
        offset += 7;
    }
    assert_eq!(walked, full);
    // The page style walks the same records by page number.
    let r = s.get("/orders?pageSize=100").await;
    let n = total(&r);
    let all: Value = r.json().await.unwrap();
    let mut walked = Vec::new();
    for page in 1..=n.div_ceil(9) {
        let b = s.json(&format!("/orders?page={page}&pageSize=9")).await;
        walked.extend(b["data"].as_array().unwrap().iter().cloned());
    }
    let all = all["data"].as_array().unwrap();
    assert_eq!((walked.len(), all.len()), (n, n.min(100)));
    assert_eq!(walked[..all.len()], all[..]);
}

#[tokio::test]
async fn invalid_limit_is_400_with_body() {
    let s = start().await;
    let r = s.get("/items?limit=0").await;
    assert_eq!(r.status(), 400);
    // The 400 is the route's answer, so the chain ran: its trace is there.
    let trace = header(&r, "x-kaanal-trace").unwrap();
    assert!(trace.contains("app"), "{trace}");
    assert_eq!(header(&r, "x-total-count"), None);
    assert_eq!(header(&r, "link"), None);
    let body: Value = r.json().await.unwrap();
    assert_eq!(
        body,
        json!({"error": "invalid query parameter", "param": "limit", "detail": "must be at least 1"})
    );
    let r = s.get("/items?offset=abc").await;
    assert_eq!(r.status(), 400);
    let r = s.get("/feed?cursor=forged").await;
    assert_eq!(r.status(), 400);
}

#[tokio::test]
async fn page_past_end_is_200_empty() {
    let s = start().await;
    let r = s.get("/orders?page=999").await;
    assert_eq!(r.status(), 200);
    let n = total(&r);
    let b: Value = r.json().await.unwrap();
    assert_eq!(b["data"], json!([]));
    assert_eq!(b["total"], n);
    assert_eq!(b["hasMore"], false);
    assert_eq!(s.json("/items?offset=100000").await, json!([]));
}

#[tokio::test]
async fn cursor_walk_reaches_the_end() {
    let s = start().await;
    let r = s.get("/feed?limit=10").await;
    let n = total(&r);
    let mut b: Value = r.json().await.unwrap();
    let mut seen = b["items"].as_array().unwrap().len();
    let mut steps = 0;
    while let Some(c) = b["nextCursor"].as_str() {
        // a nextCursor that repeats with empty pages must fail, not hang
        steps += 1;
        assert!(steps <= n / 10 + 2, "too many pages: {steps} for {n} items");
        let r = s.get(&format!("/feed?limit=10&cursor={c}")).await;
        assert_eq!(r.status(), 200);
        let last = header(&r, "link").is_none();
        b = r.json().await.unwrap();
        seen += b["items"].as_array().unwrap().len();
        assert!(seen <= n, "the walk runs past the total: {seen} > {n}");
        // Only the last page has no `next` link.
        assert_eq!(last, b["nextCursor"].is_null());
    }
    assert_eq!(seen, n);
}

#[tokio::test]
async fn unpaged_route_body_is_unchanged() {
    let s = start().await;
    let r = s.get("/plain").await;
    assert_eq!(header(&r, "x-total-count"), None);
    assert_eq!(header(&r, "link"), None);
    let body: Value = r.json().await.unwrap();
    let api =
        kaanal_data::ApiModel::from_openapi(&std::fs::read_to_string(fixture()).unwrap()).unwrap();
    let ep = api.endpoints.iter().find(|e| e.path == "/plain").unwrap();
    let seed = kaanal_core::stable_hash(&[&42u64.to_le_bytes(), b"app", b"/plain"]);
    let want = kaanal_data::generate(
        ep.schema.as_ref().unwrap(),
        None,
        seed,
        &kaanal_data::Options::default(),
    );
    assert_eq!(body, want);
    // A query on an unpaged route changes nothing.
    assert_eq!(s.json("/plain?limit=1").await, want);
}

/// Walks `route` by page number from `first` until `more` says it is the last page; the records
/// of every page in order, with no record seen twice.
async fn walk_pages(
    s: &crate::common::Sc,
    route: &str,
    first: u32,
    data: &str,
    more: impl Fn(&Value, u32) -> bool,
) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    for page in first.. {
        let b = s.json(&format!("{route}&page={page}")).await;
        out.extend(b[data].as_array().unwrap().iter().cloned());
        if !more(&b, page) {
            break;
        }
        assert!(page < 200, "the walk does not end");
    }
    let distinct: std::collections::HashSet<String> = out.iter().map(Value::to_string).collect();
    assert_eq!(distinct.len(), out.len(), "{route}: no duplicates");
    out
}

#[tokio::test]
async fn zero_and_one_based_page_walks_reach_the_end() {
    let s = start().await;
    // Spring: page 0 is the first page, `totalPages` pages in all
    let r = s.get("/spring?page=0&size=20").await;
    let n = total(&r);
    let first: Value = r.json().await.unwrap();
    assert_eq!(first["totalElements"], n);
    assert_eq!(first["totalPages"], n.div_ceil(20));
    let link = header(&s.get("/spring?page=0&size=20").await, "link").unwrap();
    assert!(
        link.starts_with("</spring?size=20&page=0>; rel=\"first\""),
        "{link}"
    );
    assert!(
        link.contains("</spring?size=20&page=1>; rel=\"next\""),
        "{link}"
    );
    let pages = u32::try_from(n.div_ceil(20)).unwrap();
    let walked = walk_pages(&s, "/spring?size=20", 0, "content", |_, p| p + 1 < pages).await;
    assert_eq!(walked.len(), n);
    assert_eq!(walked[..20], first["content"].as_array().unwrap()[..]);
    assert_eq!(s.get("/spring?page=-1").await.status(), 400);
    // 1-based: page 0 is a 400, the walk ends where `hasMore` says
    let r = s.get("/orders?page=0").await;
    assert_eq!(r.status(), 400);
    let body: Value = r.json().await.unwrap();
    assert_eq!(body["detail"], "must be at least 1");
    let n = total(&s.get("/orders").await);
    let walked = walk_pages(&s, "/orders?pageSize=9", 1, "data", |b, _| {
        b["hasMore"] == true
    })
    .await;
    assert_eq!(walked.len(), n);
}

#[tokio::test]
async fn drf_next_urls_walk_to_the_end() {
    let s = start().await;
    let r = s.get("/drf?limit=10").await;
    let n = total(&r);
    let mut b: Value = r.json().await.unwrap();
    assert_eq!(b["previous"], Value::Null);
    let mut seen = b["results"].as_array().unwrap().clone();
    while let Some(next) = b["next"].as_str() {
        assert!(seen.len() <= n, "the walk runs past the total");
        let r = s.get(next).await;
        assert_eq!(r.status(), 200, "{next}");
        // the body's `next` is the URL the Link header carries
        let link = header(&r, "link").unwrap();
        b = r.json().await.unwrap();
        if let Some(n) = b["next"].as_str() {
            assert!(link.contains(&format!("<{n}>; rel=\"next\"")), "{link}");
        }
        assert!(b["previous"].is_string());
        seen.extend(b["results"].as_array().unwrap().iter().cloned());
    }
    assert_eq!(seen.len(), n);
    // the same records as one big page (the size maximum is 100)
    let big = s.json("/drf?limit=100").await;
    let big = big["results"].as_array().unwrap();
    assert_eq!(seen[..big.len()], big[..]);
}

#[tokio::test]
async fn path_parameters_seed_and_fill_a_paged_route() {
    let s = start().await;
    let a = s.json("/shelves/a/stock?limit=5").await;
    let b = s.json("/shelves/b/stock?limit=5").await;
    // the path parameter fills the envelope's same-named field
    assert_eq!((&a["shelf"], &b["shelf"]), (&json!("a"), &json!("b")));
    // each path is its own dataset: other records (and, most likely, another total)
    assert_ne!(a["items"], b["items"]);
    // and stays stable: page 2 is the slice of a larger first page
    let big = s.json("/shelves/a/stock?limit=10").await;
    let two = s.json("/shelves/a/stock?limit=5&offset=5").await;
    assert_eq!(
        two["items"].as_array().unwrap()[..],
        big["items"].as_array().unwrap()[5..10]
    );
    assert_eq!(two["shelf"], "a");
}
