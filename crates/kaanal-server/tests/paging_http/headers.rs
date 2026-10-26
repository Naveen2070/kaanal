//! `X-Total-Count`, `Link`, CORS exposure, HEAD and the `total:` override.

use crate::common::{NODES, check_yaml, header, start, start_yaml, total};

#[tokio::test]
async fn first_page_has_default_size_and_headers() {
    let s = start().await;
    let r = s.get("/items").await;
    assert_eq!(r.status(), 200);
    let n = total(&r);
    assert!((23..=137).contains(&n), "{n}");
    let link = header(&r, "link").unwrap();
    assert!(link.contains("</items?offset=20>; rel=\"next\""), "{link}");
    assert!(link.contains("</items?offset=0>; rel=\"first\""), "{link}");
    assert!(!link.contains("rel=\"prev\""), "{link}");
    let body: serde_json::Value = r.json().await.unwrap();
    assert_eq!(body.as_array().unwrap().len(), 20);

    // The object envelope: the spec default size (10) and consistent metadata.
    let r = s.get("/orders").await;
    let n = total(&r);
    let body: serde_json::Value = r.json().await.unwrap();
    assert_eq!(body["data"].as_array().unwrap().len(), 10);
    assert_eq!(body["total"], n);
    assert_eq!(body["page"], 1);
    assert_eq!(body["pageSize"], 10);
    assert_eq!(body["totalPages"], n.div_ceil(10));
    assert_eq!(body["hasMore"], true);
}

#[tokio::test]
async fn x_total_count_and_link_are_exposed_to_browsers() {
    let s = start().await;
    let r = s.get("/items").await;
    let exposed = header(&r, "access-control-expose-headers").unwrap();
    assert!(exposed.contains("x-total-count, link"), "{exposed}");
}

#[tokio::test]
async fn link_uses_the_raw_request_path() {
    let s = start().await;
    let r = s.get("/shelves/a%20b/items?limit=5").await;
    assert_eq!(r.status(), 200);
    let link = header(&r, "link").unwrap();
    assert!(
        link.contains("</shelves/a%20b/items?limit=5&offset=5>; rel=\"next\""),
        "{link}"
    );
}

#[tokio::test]
async fn head_matches_get_without_body() {
    let s = start().await;
    let get = s.get("/items?limit=7&offset=7").await;
    let head = s
        .client
        .head(format!("{}/items?limit=7&offset=7", s.base))
        .send()
        .await
        .unwrap();
    assert_eq!(head.status(), 200);
    assert!(header(&get, "x-total-count").is_some());
    assert!(header(&get, "link").is_some());
    assert_eq!(
        header(&head, "x-total-count"),
        header(&get, "x-total-count")
    );
    assert_eq!(header(&head, "link"), header(&get, "link"));
    assert!(head.bytes().await.unwrap().is_empty());
}

#[tokio::test]
async fn total_key_overrides_the_total() {
    let s = start_yaml(&format!(
        "{NODES}endpoints:\n  \"GET /items\": {{ total: 250 }}\n"
    ))
    .await;
    let r = s.get("/items?limit=200&offset=200").await;
    assert_eq!(header(&r, "x-total-count").as_deref(), Some("250"));
    let body: serde_json::Value = r.json().await.unwrap();
    assert_eq!(body.as_array().unwrap().len(), 50);
}

#[tokio::test]
async fn a_head_entry_without_total_inherits_the_get_total() {
    let s = start_yaml(&format!(
        "{NODES}endpoints:\n  \"GET /items\": {{ total: 250 }}\n  \"HEAD /items\": {{ op: read }}\n"
    ))
    .await;
    let get = s.get("/items?limit=5").await;
    let head = s
        .client
        .head(format!("{}/items?limit=5", s.base))
        .send()
        .await
        .unwrap();
    assert_eq!(header(&get, "x-total-count").as_deref(), Some("250"));
    assert_eq!(header(&head, "x-total-count").as_deref(), Some("250"));
    assert_eq!(header(&head, "link"), header(&get, "link"));
}

#[tokio::test]
async fn a_spec_head_without_paging_params_is_not_paged() {
    let s = start().await;
    let get = s.get("/rows?limit=5").await;
    assert!(header(&get, "x-total-count").is_some());
    let head = s
        .client
        .head(format!("{}/rows?limit=5", s.base))
        .send()
        .await
        .unwrap();
    assert_eq!(head.status(), 200);
    assert_eq!(header(&head, "x-total-count"), None);
    assert_eq!(header(&head, "link"), None);
}

#[test]
fn total_on_an_unpaged_route_warns() {
    let w = check_yaml(&format!(
        "{NODES}endpoints:
  \"GET /plain\": {{ total: 5 }}
  \"GET /items\": {{ total: 5 }}
"
    ));
    let hits: Vec<&String> = w.iter().filter(|m| m.contains("total")).collect();
    assert_eq!(hits.len(), 1, "{w:?}");
    assert!(
        hits[0].contains("'GET /plain'") && hits[0].contains("no effect"),
        "{w:?}"
    );
}
