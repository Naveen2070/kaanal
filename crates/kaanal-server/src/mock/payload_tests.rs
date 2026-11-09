//! Live payload edits (step 8A): what a request gets, what outranks an edit, what refuses one.

use super::tests::{YAML, app, app_with, call, entries};
use super::*;
use crate::payload::PayloadError;
use kaanal_core::PinEntry;
use kaanal_core::config::Override;

fn edit(status: u16, body: Value, h: &[(&str, &str)]) -> Override {
    Override {
        status,
        body,
        headers: h
            .iter()
            .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
            .collect(),
    }
}

async fn json_of(r: Response) -> Value {
    let b = axum::body::to_bytes(r.into_body(), usize::MAX)
        .await
        .unwrap();
    serde_json::from_slice(&b).unwrap_or(Value::Null)
}

fn with_ov_route() -> String {
    YAML.replace(
        "  \"POST /orders\": { chain: [ h ] }\n",
        "  \"POST /orders\": { chain: [ h ] }\n  \"GET /ov\": { override: { body: { a: 1 }, headers: { x-yaml: \"1\" } } }\n",
    )
}

#[tokio::test]
async fn an_edit_replaces_status_body_and_headers_and_keeps_the_trace() {
    let st = app();
    let g = st.generation().unwrap();
    st.set_payload(
        &g,
        "GET /users/{id}",
        edit(503, json!({"down": true}), &[("x-demo", "1")]),
    )
    .unwrap();
    let r = call(&st, "GET", "/users/1", &[]).await;
    assert_eq!(r.status(), 503);
    assert_eq!(r.headers()["x-demo"], "1");
    assert!(r.headers().contains_key("x-kaanal-trace") && r.headers().contains_key("x-request-id"));
    assert_eq!(json_of(r).await, json!({"down": true}));
    assert!(entries(&st).last().unwrap().edited);
}

#[tokio::test]
async fn without_an_edit_nothing_is_marked() {
    let st = app();
    call(&st, "GET", "/users/1", &[]).await;
    assert!(!entries(&st).last().unwrap().edited);
}

#[tokio::test]
async fn chaos_beats_an_edit() {
    let st = app();
    let g = st.generation().unwrap();
    st.set_payload(&g, "GET /users/{id}", edit(200, json!({"edited": 1}), &[]))
        .unwrap();
    {
        let mut c = crate::control::lock(&st);
        let sc = c.scenarios.clone();
        c.stack.activate(&sc, "down").unwrap();
        st.publish(&c).unwrap();
    }
    let r = call(&st, "GET", "/users/1", &[]).await;
    assert_eq!(r.status(), 503);
    assert_ne!(json_of(r).await, json!({"edited": 1}));
    assert!(!entries(&st).last().unwrap().edited);
}

fn pin(st: &Arc<AppState>, state: PinState) {
    let mut c = crate::control::lock(st);
    c.stack.live_pins_mut().insert(
        "GET /users/{id}".into(),
        PinEntry {
            state,
            status: None,
            delay: None,
        },
    );
    st.publish(&c).unwrap();
}

#[tokio::test]
async fn a_pinned_error_beats_an_edit_and_a_pinned_success_serves_it() {
    let st = app();
    let g = st.generation().unwrap();
    st.set_payload(&g, "GET /users/{id}", edit(202, json!({"edited": 1}), &[]))
        .unwrap();
    pin(&st, PinState::Error);
    let r = call(&st, "GET", "/users/1", &[]).await;
    assert_eq!(r.status(), 500);
    assert!(!entries(&st).last().unwrap().edited);
    pin(&st, PinState::Success);
    let r = call(&st, "GET", "/users/1", &[]).await;
    assert_eq!(r.status(), 202);
    assert!(entries(&st).last().unwrap().edited);
}

#[tokio::test]
async fn an_edit_beats_the_yaml_override_and_the_override_keeps_its_headers_without_one() {
    let st = app_with(&with_ov_route());
    let r = call(&st, "GET", "/ov", &[]).await;
    assert_eq!(
        (r.status().as_u16(), r.headers()["x-yaml"].to_str().unwrap()),
        (200, "1")
    );
    assert_eq!(json_of(r).await, json!({"a": 1}));
    let g = st.generation().unwrap();
    st.set_payload(&g, "GET /ov", edit(202, json!({"b": 2}), &[]))
        .unwrap();
    let r = call(&st, "GET", "/ov", &[]).await;
    assert_eq!(r.status(), 202);
    assert!(
        !r.headers().contains_key("x-yaml"),
        "the edit replaces the override whole"
    );
    assert_eq!(json_of(r).await, json!({"b": 2}));
}

#[tokio::test]
async fn a_scoped_request_does_not_see_the_edit() {
    let st = app();
    let g = st.generation().unwrap();
    st.set_payload(&g, "GET /users/{id}", edit(202, json!({"edited": 1}), &[]))
        .unwrap();
    let r = call(&st, "GET", "/users/1", &[("x-kaanal-scenario", "cachelag")]).await;
    assert_eq!(r.status(), 200);
    assert!(!entries(&st).last().unwrap().edited);
}

#[tokio::test]
async fn a_head_follows_its_get_edit() {
    let st = app();
    let g = st.generation().unwrap();
    st.set_payload(&g, "GET /users/{id}", edit(202, json!({"edited": 1}), &[]))
        .unwrap();
    let r = call(&st, "HEAD", "/users/1", &[]).await;
    assert_eq!(r.status(), 202);
}

#[tokio::test]
async fn an_edit_counts_toward_sizes() {
    let st = app();
    let g = st.generation().unwrap();
    let body = json!({"edited": "x".repeat(300)});
    st.set_payload(&g, "GET /users/{id}", edit(200, body.clone(), &[]))
        .unwrap();
    let m = g.paths.at("/users/1").unwrap();
    let route = m.value.iter().find(|r| r.method == "GET").unwrap();
    let ask = Ask {
        method: "GET",
        path: "/users/1",
        params: &[],
        query: &[],
    };
    assert_eq!(
        resp_bytes((&st, &g), route, None, &ask, true),
        serde_json::to_vec(&body).unwrap().len() as u64
    );
}

#[tokio::test]
async fn set_refuses_collection_synthesized_unknown_and_invalid_and_stores_nothing() {
    let yaml = YAML.replace(
        "  \"GET /users\": { chain: [ { cache: c } ] }\n",
        "  \"GET /users\": { collection: people }\n",
    );
    let st = app_with(&yaml);
    let g = st.generation().unwrap();
    let ok = || edit(200, json!({}), &[]);
    assert!(matches!(
        st.set_payload(&g, "GET /users", ok()),
        Err(PayloadError::Collection(_))
    ));
    assert!(matches!(
        st.set_payload(&g, "HEAD /users/{id}", ok()),
        Err(PayloadError::Synthesized(_))
    ));
    assert!(matches!(
        st.set_payload(&g, "GET /nope", ok()),
        Err(PayloadError::NoRoute(_))
    ));
    assert!(matches!(
        st.set_payload(&g, "GET /users/{id}", edit(99, json!({}), &[])),
        Err(PayloadError::Invalid(_))
    ));
    assert!(matches!(
        st.set_payload(&g, "bad key", ok()),
        Err(PayloadError::Invalid(_))
    ));
    assert!(st.payloads().is_empty());
}

#[tokio::test]
async fn clear_reset_and_prune() {
    let st = app();
    let g = st.generation().unwrap();
    let ok = || edit(200, json!({}), &[]);
    st.set_payload(&g, "GET /users/{id}", ok()).unwrap();
    st.set_payload(&g, "POST /orders", ok()).unwrap();
    assert!(st.clear_payload("POST /orders"));
    assert!(!st.clear_payload("POST /orders"));
    st.set_payload(&g, "POST /orders", ok()).unwrap();
    // a system where GET /users/{id} became a collection: its edit goes, the other stays
    let yaml = YAML.replace(
        "  \"GET /users/{id}\": { chain: [ db ] }\n",
        "  \"GET /users/{id}\": { collection: people }\n",
    );
    let g2 = app_with(&yaml).generation().unwrap();
    let dropped = st.prune_payloads(&g2);
    assert_eq!(dropped, vec!["GET /users/{id}".to_string()]);
    assert_eq!(st.payloads().iter().count(), 1);
    st.clear_payloads();
    assert!(st.payloads().is_empty());
}

#[tokio::test]
async fn a_scoped_request_is_sized_by_the_body_it_gets_not_by_the_edit() {
    let st = app();
    let g = st.generation().unwrap();
    let body = json!({"edited": "x".repeat(300)});
    st.set_payload(&g, "GET /users/{id}", edit(200, body.clone(), &[]))
        .unwrap();
    let m = g.paths.at("/users/1").unwrap();
    let route = m.value.iter().find(|r| r.method == "GET").unwrap();
    let ask = Ask {
        method: "GET",
        path: "/users/1",
        params: &[],
        query: &[],
    };
    let generated = serde_json::to_vec(&reply::success(&g, route, &ask).1)
        .unwrap()
        .len() as u64;
    assert_eq!(resp_bytes((&st, &g), route, None, &ask, false), generated);
    assert_ne!(generated, serde_json::to_vec(&body).unwrap().len() as u64);
}

#[tokio::test]
async fn custom_headers_are_exposed_to_a_browser_on_another_origin() {
    let st = app_with(&with_ov_route());
    let base = "x-kaanal-trace, x-kaanal-scope, x-request-id, retry-after, x-total-count, link";
    let plain = call(&st, "GET", "/users/1", &[]).await;
    assert_eq!(plain.headers()["access-control-expose-headers"], base);
    // a file override's header, and an edit's, can be read by `fetch` on another origin
    let r = call(&st, "GET", "/ov", &[]).await;
    let exp = r.headers()["access-control-expose-headers"]
        .to_str()
        .unwrap()
        .to_string();
    assert!(exp.starts_with(base) && exp.contains("x-yaml"), "{exp}");
    let g = st.generation().unwrap();
    st.set_payload(
        &g,
        "GET /users/{id}",
        edit(200, json!({}), &[("X-Feature", "on")]),
    )
    .unwrap();
    let r = call(&st, "GET", "/users/1", &[]).await;
    let exp = r.headers()["access-control-expose-headers"]
        .to_str()
        .unwrap()
        .to_string();
    assert!(exp.starts_with(base) && exp.contains("x-feature"), "{exp}");
}
