//! The control guard: token, Host, query token, caching headers.

use crate::common::*;

#[tokio::test]
async fn control_requires_token() {
    let s = start().await;
    let r = s.get("/__kaanal/topology").await;
    assert_eq!(r.status(), 401);
    let r = s
        .client
        .get(format!("{}/__kaanal/topology", s.base))
        .bearer_auth("wrong")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 401);
}

#[tokio::test]
async fn control_rejects_foreign_host() {
    let s = start().await;
    let r = s
        .auth(s.client.get(format!("{}/__kaanal/topology", s.base)))
        .header("host", "evil.example")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 403);
}

#[tokio::test]
async fn host_guard_covers_the_whole_control_namespace() {
    let s = start().await;
    for p in ["/__kaanal/health", "/__kaanal/ui"] {
        let r = s
            .client
            .get(format!("{}{p}", s.base))
            .header("host", "evil.example")
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 403, "{p}");
    }
}

#[tokio::test]
async fn unauthenticated_calls_are_rejected_before_the_body_is_parsed() {
    let s = start().await;
    let r = s
        .client
        .put(format!("{}/__kaanal/chaos/db", s.base))
        .header("content-type", "application/json")
        .body("{ not json")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 401);
}

#[tokio::test]
async fn query_token_only_works_on_events() {
    let s = start().await;
    let r = s
        .client
        .get(format!("{}/__kaanal/topology?token={}", s.base, s.token))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 401);
}

#[tokio::test]
async fn control_responses_are_not_cacheable() {
    let s = start().await;
    let r = s.get_authed("/__kaanal/topology").await;
    assert_eq!(r.headers()["cache-control"], "no-store");
    assert_eq!(r.headers()["x-content-type-options"], "nosniff");
}
