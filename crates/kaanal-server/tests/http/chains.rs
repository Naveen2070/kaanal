//! Traces and chains: seeds, gatewayless and monolith configs, error maps, endpoints files, realistic replay.

use crate::common::*;

use kaanal_core::Config;

use kaanal_core::config::Determinism;

use serde_json::{Value, json};

#[tokio::test]
async fn trace_shows_sub_millisecond_hops_and_names_the_failing_node() {
    let s = start().await;
    let ok = s.get("/users/42").await;
    let trace = ok.headers()["x-kaanal-trace"].to_str().unwrap().to_string();
    // every hop has one decimal, e.g. `net=1.0ms`, never a truncated `0ms`
    assert!(
        trace.split(',').all(|h| {
            let v = h.split('=').nth(1).unwrap();
            v.ends_with("ms") && v.trim_end_matches("ms").contains('.')
        }),
        "{trace}"
    );
    s.put_chaos("db", json!({ "error_rate": 1.0, "error_status": 504 }))
        .await;
    let failed = s.get("/users/42").await;
    let trace = failed.headers()["x-kaanal-trace"]
        .to_str()
        .unwrap()
        .to_string();
    assert!(trace.ends_with(",db.error=504"), "{trace}");
}

#[tokio::test]
async fn config_seed_changes_generated_bodies() {
    let body = |s: Srv| async move { s.get("/users/42").await.json::<Value>().await.unwrap() };
    let a = body(start_seeded(Some(1)).await).await;
    assert_eq!(
        a,
        body(start_seeded(Some(1)).await).await,
        "same seed, same data"
    );
    let b = body(start_seeded(Some(2)).await).await;
    assert_ne!(
        a["email"], b["email"],
        "a new seed reshuffles the data: {a} vs {b}"
    );
    assert_eq!(b["id"], 42, "path params still win");
}

#[tokio::test]
async fn gatewayless_requests_enter_at_the_service() {
    let s = start_yaml(
        "version: 1\nnodes:\n  app: { type: service, spec: openapi.yaml, proc: 1ms }\n  db: { type: database, flavor: postgres }\nedges:\n  - { from: app, to: db, net: 1ms }\n",
    )
    .await;
    let r = s.get("/users/42").await;
    assert_eq!(r.status(), 200);
    let trace = r.headers()["x-kaanal-trace"].to_str().unwrap().to_string();
    assert!(
        trace.starts_with("app.proc=") && trace.contains("db.read="),
        "{trace}"
    );
}

#[tokio::test]
async fn a_lone_monolith_serves_its_spec() {
    let s = start_yaml(
        "version: 1\nnodes:\n  app: { type: monolith, spec: openapi.yaml, proc: 1ms, flavor: postgres }\n",
    )
    .await;
    let r = s.get("/users/42").await;
    assert_eq!(r.status(), 200);
    let trace = trace_of(&r);
    assert!(trace.starts_with("app.proc=1.0ms,app.read="), "{trace}");
    assert_eq!(
        trace.matches(',').count(),
        1,
        "proc, then the inline op: {trace}"
    );
}

#[tokio::test]
async fn trace_is_capped_and_counts_what_it_left_out() {
    // 2 003 hops: nginx.proc, net, app.proc, then 1 000 × (net, db.read). The request sleeps 1 s.
    let s = start_yaml("version: 1\nnodes:\n  nginx: { type: gateway }\n  app: { type: service, spec: openapi.yaml }\n  db: { type: database, ops: { read: 1ms, write: 1ms, scan: 1ms } }\nedges:\n  - { from: nginx, to: app }\n  - { from: app, to: db }\nendpoints:\n  \"GET /users/{id}\": { chain: [ { call: db, repeat: 1000 } ] }\n").await;
    let r = s.get("/users/1").await;
    assert_eq!(r.status(), 200);
    let trace = trace_of(&r);
    // Hand count (D13): the first three hops take 16 + 10 + 15 = 41 bytes, each
    // (`,net=0.0ms`, `,db.read=1.0ms`) pair 24 more. A hop is kept while the text plus it stays
    // within 4096 − 14 (room for `,...+NNNN hops`): 41 + 24 × 168 = 4073 fits, the next `net`
    // would make 4083. Left out: 2003 − (3 + 2 × 168) = 1664 hops.
    assert_eq!(trace.len(), 4087, "{trace}");
    assert!(
        trace.starts_with("nginx.proc=0.0ms,net=0.0ms,app.proc=0.0ms,net=0.0ms,db.read=1.0ms,"),
        "{trace}"
    );
    assert!(
        trace.ends_with(",net=0.0ms,db.read=1.0ms,...+1664 hops"),
        "{trace}"
    );
    assert_eq!(trace.matches("db.read=").count(), 168);
}

#[tokio::test]
async fn trace_brackets_parallel_branches_and_marks_cache_hits() {
    let yaml = "version: 1\nnodes:\n  nginx: { type: gateway, proc: 1ms }\n  app: { type: service, spec: openapi.yaml, proc: 2ms }\n  audit: { type: service, proc: 1ms }\n  pay: { type: external-api, proc: 4ms }\n  c: { type: cache, hit_ratio: 1.0, hit: 200us, miss: 1ms }\nedges:\n  - { from: nginx, to: app }\n  - { from: app, to: audit }\n  - { from: app, to: pay }\n  - { from: app, to: c }\nendpoints:\n  \"GET /users/{id}\": { chain: [ { parallel: [ audit, pay ] }, { cache: c } ] }\n";
    let s = start_yaml(yaml).await;
    let r = s.get("/users/1").await;
    assert_eq!(
        trace_of(&r),
        "nginx.proc=1.0ms,net=0.0ms,app.proc=2.0ms,[net=0.0ms,audit.proc=1.0ms|net=0.0ms,pay.proc=4.0ms],net=0.0ms,c.hit=0.2ms"
    );
    // pay fails at its hop start (3 ms): audit's hops start at 3 ms too, so they are dropped.
    s.put_chaos("pay", json!({ "down": true })).await;
    let r = s.get("/users/1").await;
    assert_eq!(r.status(), 503);
    assert_eq!(
        trace_of(&r),
        "nginx.proc=1.0ms,net=0.0ms,app.proc=2.0ms,[|net=0.0ms],pay.error=503"
    );
}

#[tokio::test]
async fn trace_drops_blocks_a_failure_cancelled_before_they_started() {
    // Branch 0 would open a nested `parallel` at 2 ms, the instant branch 1 fails: none of the
    // nested hops start, so the block must not print as `[|]`.
    let yaml = "version: 1\nnodes:\n  app: { type: service, spec: openapi.yaml }\n  s1: { type: external-api, proc: 2ms }\n  a: { type: external-api, proc: 1ms }\n  b: { type: external-api, proc: 1ms }\n  w: { type: external-api, proc: 2ms }\n  f: { type: external-api }\nedges:\n  - { from: app, to: s1 }\n  - { from: app, to: a }\n  - { from: app, to: b }\n  - { from: app, to: w }\n  - { from: app, to: f }\nendpoints:\n  \"GET /users/{id}\": { chain: [ { parallel: [ [ s1, { parallel: [ a, b ] } ], [ w, f ] ] } ] }\n";
    let s = start_yaml(yaml).await;
    s.put_chaos("f", json!({ "down": true })).await;
    let r = s.get("/users/1").await;
    assert_eq!(r.status(), 503);
    assert_eq!(
        trace_of(&r),
        "app.proc=0.0ms,[net=0.0ms,s1.proc=2.0ms|net=0.0ms,w.proc=2.0ms,net=0.0ms],f.error=503"
    );
}

#[tokio::test]
async fn error_map_shapes_status_and_body() {
    let s = start_yaml(&format!(
        "{GW_APP_DB}  \"GET /users/{{id}}\":\n    error_map: {{ db: {{ status: 502, body: {{ error: upstream }} }}, \"5xx\": 500 }}\n"
    ))
    .await;
    s.put_chaos("db", json!({ "error_rate": 1.0, "error_status": 504 }))
        .await;
    let r = s.get("/users/1").await;
    assert_eq!(r.status(), 502, "by node");
    assert!(
        trace_of(&r).ends_with(",db.error=504"),
        "the trace keeps the real failure"
    );
    assert_eq!(
        r.json::<Value>().await.unwrap(),
        json!({ "error": "upstream" })
    );
    s.auth(s.client.delete(format!("{}/__kaanal/chaos/db", s.base)))
        .send()
        .await
        .unwrap();
    s.put_chaos("app", json!({ "down": true })).await;
    let r = s.get("/users/1").await;
    assert_eq!(r.status(), 500, "by class");
    assert_eq!(
        r.json::<Value>().await.unwrap(),
        json!({ "error": "chaos", "node": "app" })
    );
    let r = s
        .client
        .post(format!("{}/orders", s.base))
        .send()
        .await
        .unwrap();
    assert_eq!(
        r.status(),
        503,
        "an endpoint without a map passes the status through"
    );
}

#[tokio::test]
async fn head_inherits_the_get_entry() {
    let s = start_yaml(&format!(
        "{GW_APP_DB}  \"GET /users/{{id}}\": {{ override: {{ status: 202, body: {{ x: 1 }} }} }}\n  \"HEAD /users\": {{ override: {{ status: 204 }} }}\n"
    ))
    .await;
    let r = s
        .client
        .head(format!("{}/users/7", s.base))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 202, "HEAD without an entry copies GET's (R6.8)");
    assert!(trace_of(&r).contains("db.read="));
    assert_eq!(
        s.get("/users/7").await.json::<Value>().await.unwrap(),
        json!({ "x": 1 })
    );
    let r = s
        .client
        .head(format!("{}/users", s.base))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
    let trace = trace_of(&r);
    assert!(
        trace.contains("app.proc=") && trace.contains("db.scan="),
        "an explicit HEAD keeps GET's owner and op: {trace}"
    );
}

#[tokio::test]
async fn endpoints_file_and_inline_serve_the_same_chain() {
    let entry = "\"GET /users/{id}\": { chain: [ { call: db, repeat: 2 } ] }\n";
    let inline = start_yaml(&format!("{GW_APP_DB}  {entry}")).await;
    let dir = std::env::temp_dir().join(format!("kaanal-http-eps-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::copy(fixtures().join("openapi.yaml"), dir.join("openapi.yaml")).unwrap();
    std::fs::write(dir.join("kaanal.endpoints.yaml"), entry).unwrap();
    let cfg =
        Config::from_yaml(&GW_APP_DB.replace("endpoints:\n", "endpoints: kaanal.endpoints.yaml\n"))
            .unwrap();
    let file = start_in(&cfg, &dir).await;
    let want =
        "nginx.proc=1.0ms,net=1.0ms,app.proc=1.0ms,net=1.0ms,db.read=1.0ms,net=1.0ms,db.read=1.0ms";
    assert_eq!(trace_of(&inline.get("/users/1").await), want);
    assert_eq!(trace_of(&file.get("/users/1").await), want);
}

#[tokio::test]
async fn realistic_runs_replay_under_controlled_with_the_printed_seed() {
    let fixture = std::fs::read_to_string(fixtures().join("kaanal.yaml")).unwrap();
    let mut cfg = Config::from_yaml(&fixture).unwrap();
    cfg.determinism = Determinism::Realistic;
    let real = start_in(&cfg, &fixtures()).await;
    let drawn = real.drawn_seed.expect("realistic mode draws a seed");
    let other = start_in(&cfg, &fixtures()).await;
    assert_ne!(other.drawn_seed, Some(drawn), "a fresh seed per start");
    // D14: bodies keep `seed:`; only chain draws follow the drawn seed.
    let body = |r: reqwest::Response| async move { r.json::<Value>().await.unwrap() };
    assert_eq!(
        body(other.get("/users/42").await).await,
        body(start().await.get("/users/42").await).await
    );
    // Replay: `controlled` with the drawn seed gives the same outcomes, request by request.
    let mut replay = Config::from_yaml(&fixture).unwrap();
    replay.seed = drawn;
    let replay = start_in(&replay, &fixtures()).await;
    assert_eq!(replay.drawn_seed, None);
    // Bodies follow `seed:` in both modes, so a replay under the drawn seed generates other
    // data (documented: only timings and failures replay). Compared before `statuses` turns
    // chaos on: with the same chain seed both servers fail the same request, and a chaos
    // error body is identical on both.
    assert_ne!(
        body(real.get("/users/42").await).await,
        body(replay.get("/users/42").await).await
    );
    assert_eq!(statuses(&real).await, statuses(&replay).await);
}

#[tokio::test]
async fn gatewayless_custom_chain_runs_in_order() {
    let s = start_yaml("version: 1\nnodes:\n  app: { type: service, spec: openapi.yaml, proc: 1ms }\n  db: { type: database, ops: { read: 1ms, write: 1ms, scan: 1ms } }\nedges:\n  - { from: app, to: db, net: 1ms }\nendpoints:\n  \"GET /users/{id}\": { chain: [ { call: db, repeat: 2 } ] }\n  \"POST /orders\": { override: { status: 202, body: { queued: true } } }\n").await;
    assert_eq!(
        trace_of(&s.get("/users/1").await),
        "app.proc=1.0ms,net=1.0ms,db.read=1.0ms,net=1.0ms,db.read=1.0ms"
    );
    let r = s
        .client
        .post(format!("{}/orders", s.base))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 202);
    assert_eq!(
        trace_of(&r),
        "app.proc=1.0ms,net=1.0ms,db.write=1.0ms",
        "an override still runs the chain"
    );
    assert_eq!(r.json::<Value>().await.unwrap(), json!({ "queued": true }));
}
