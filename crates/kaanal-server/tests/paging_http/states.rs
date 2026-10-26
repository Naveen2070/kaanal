//! Chaos, pins and scopes on a paged route.

use serde_json::{Value, json};

use crate::common::{header, start};

#[tokio::test]
async fn chaos_error_wins_over_paging() {
    let s = start().await;
    s.activate("items-503").await;
    let r = s.get("/items?limit=0").await;
    assert_eq!(r.status(), 503);
    let r = s.get("/items").await;
    assert_eq!(r.status(), 503);
    assert_eq!(header(&r, "x-total-count"), None);
    assert_eq!(header(&r, "link"), None);
}

#[tokio::test]
async fn pinned_empty_state_returns_the_empty_form() {
    let s = start().await;
    s.activate("p-empty").await;
    let r = s.get("/items?limit=5&offset=5").await;
    assert_eq!(r.status(), 200);
    assert_eq!(header(&r, "x-total-count"), None);
    assert_eq!(header(&r, "link"), None);
    let b: Value = r.json().await.unwrap();
    assert_eq!(b, json!([]));
}

#[tokio::test]
async fn scoped_requests_page_the_same() {
    let s = start().await;
    let plain = s.get("/items?limit=5&offset=10").await;
    let scoped = s
        .client
        .get(format!("{}/items?limit=5&offset=10", s.base))
        .header("x-kaanal-scenario", "slow-app")
        .send()
        .await
        .unwrap();
    assert_eq!(scoped.status(), 200);
    assert_eq!(
        header(&scoped, "x-kaanal-scope").as_deref(),
        Some("slow-app")
    );
    assert_eq!(
        header(&scoped, "x-total-count"),
        header(&plain, "x-total-count")
    );
    assert_eq!(header(&scoped, "link"), header(&plain, "link"));
    let a: Value = plain.json().await.unwrap();
    let b: Value = scoped.json().await.unwrap();
    assert_eq!(a, b);
}

#[tokio::test]
async fn wire_chaos_on_a_paged_route_has_no_paging_headers() {
    for name in ["items-malformed", "items-reset"] {
        let s = start().await;
        s.activate(name).await;
        let r = s
            .client
            .get(format!("{}/items?limit=5", s.base))
            .send()
            .await;
        // only a reset may fail the request itself (then there are no headers at all)
        let r = match r {
            Ok(r) => r,
            Err(_) if name == "items-reset" => continue,
            Err(e) => panic!("{name}: {e}"),
        };
        assert_eq!(header(&r, "x-total-count"), None, "{name}");
        assert_eq!(header(&r, "link"), None, "{name}");
    }
}

#[tokio::test]
async fn success_and_slow_pins_keep_the_paging_headers() {
    for name in ["p-success", "p-slow"] {
        let s = start().await;
        let plain = s.get("/items?limit=5&offset=5").await;
        s.activate(name).await;
        let r = s.get("/items?limit=5&offset=5").await;
        assert_eq!(r.status(), 200, "{name}");
        assert_eq!(
            header(&r, "x-total-count"),
            header(&plain, "x-total-count"),
            "{name}"
        );
        assert!(header(&r, "link").is_some(), "{name}");
        assert_eq!(header(&r, "link"), header(&plain, "link"), "{name}");
        let b: Value = r.json().await.unwrap();
        assert_eq!(b.as_array().unwrap().len(), 5, "{name}");
    }
}
