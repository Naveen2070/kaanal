//! Unit tests for the mock request path.

use super::*;
use axum::http::Request as HttpRequest;
use kaanal_core::Config;
use kaanal_core::replay::Engine;

const YAML: &str = "version: 1
seed: 42
nodes:
  nginx: { type: gateway, proc: 1ms }
  app: { type: service, spec: openapi.yaml, proc: 1ms }
  db: { type: database, ops: { read: 1ms, write: 1ms, scan: 1ms } }
  h: { type: external-api, proc: 1ms, timeout: 150ms }
  c: { type: cache, hit_ratio: 1.0, hit: 1ms, miss: 2ms }
edges:
  - { from: nginx, to: app, net: 1ms }
  - { from: app, to: db, net: 1ms }
  - { from: app, to: h, net: 1ms }
  - { from: app, to: c, net: 1ms }
endpoints:
  \"GET /users/{id}\": { chain: [ db ] }
  \"GET /users\": { chain: [ { cache: c } ] }
  \"POST /orders\": { chain: [ h ] }
scenarios:
  hang: { chaos: [ { node: h, error_rate: 1.0, error_kind: hang, latency_ms: 30 } ] }
  cachelag: { chaos: [ { node: c, latency_ms: 40 } ] }
  eplag: { chaos: [ { endpoint: \"GET /users/{id}\", latency_ms: 20 } ] }
  dbhang: { chaos: [ { node: db, error_rate: 1.0, error_kind: hang, latency_ms: 30 } ] }
  hreset:
    chaos: [ { node: db, error_rate: 1.0, error_kind: reset } ]
    pins: [ { endpoint: \"GET /users/{id}\", state: success } ]
  lag: { chaos: [ { node: db, latency_ms: 300 } ] }
  down: { chaos: [ { node: db, error_rate: 1.0, error_status: 503 } ] }
  hidden:
    chaos: [ { node: db, error_rate: 1.0, error_status: 503 } ]
    pins: [ { endpoint: \"GET /users/{id}\", state: success } ]
";

type Req<'a> = (&'a str, &'a str, &'a [(&'a str, &'a str)]);

/// The server state of `YAML` with a live engine, driven by calling [`handle`] directly: the
/// ring is read through `st.log`, which no route serves yet.
fn app() -> Arc<AppState> {
    app_with(YAML)
}

fn app_with(yaml: &str) -> Arc<AppState> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let cfg = Config::from_yaml(yaml).unwrap();
    let mut st = AppState::build(&cfg, &dir, "t".into(), 0).unwrap();
    let g = st.gen_mut();
    g.engine = Some(crate::actor::spawn(Engine::new(&g.topo)));
    Arc::new(st)
}

async fn call(st: &Arc<AppState>, method: &str, uri: &str, headers: &[(&str, &str)]) -> Response {
    let mut b = HttpRequest::builder().method(method).uri(uri);
    for (k, v) in headers {
        b = b.header(*k, *v);
    }
    handle(State(Arc::clone(st)), b.body(Body::from("{}")).unwrap()).await
}

fn entries(st: &AppState) -> Vec<LogEntry> {
    st.log
        .lock()
        .unwrap()
        .query(&log::Filter::default(), 1000)
        .entries
}

#[tokio::test]
async fn each_request_makes_exactly_one_entry_and_one_event() {
    let st = app();
    let mut rx = st.events.subscribe();
    let reqs: [Req; 7] = [
        ("GET", "/users/1", &[]),
        ("GET", "/users", &[]),
        ("GET", "/nope", &[]),
        ("DELETE", "/users/1", &[]),
        ("GET", "/users/2", &[("x-kaanal-scenario", "bogus")]),
        ("GET", "/users/3", &[("x-kaanal-scenario", "down")]),
        ("OPTIONS", "/users/3", &[]),
    ];
    let mut ids = Vec::new();
    for (m, u, h) in reqs {
        let r = call(&st, m, u, h).await;
        if m != "OPTIONS" {
            ids.push(r.headers()["x-request-id"].to_str().unwrap().to_string());
        }
    }
    let logged = entries(&st);
    let mut events = Vec::new();
    while let Ok(BusEvent::Request(e)) = rx.try_recv() {
        events.push(e);
    }
    assert_eq!((logged.len(), events.len(), ids.len()), (6, 6, 6));
    assert_eq!(
        logged.iter().map(|e| e.id.clone()).collect::<Vec<_>>(),
        ids,
        "one entry per response, in order"
    );
    assert_eq!(events.iter().map(|e| e.id.clone()).collect::<Vec<_>>(), ids);
    ids.dedup();
    assert_eq!(ids.len(), 6);
}

#[tokio::test]
async fn unmatched_and_bad_scope_are_logged_with_rejected() {
    let st = app();
    let mut rx = st.events.subscribe();
    call(&st, "GET", "/nope", &[]).await;
    call(&st, "DELETE", "/users/1", &[]).await;
    call(&st, "GET", "/users/1", &[("x-kaanal-scenario", "bogus")]).await;
    let got = entries(&st);
    let shape: Vec<_> = got
        .iter()
        .map(|e| (e.status, e.rejected, e.endpoint.as_deref(), e.hops.len()))
        .collect();
    assert_eq!(
        shape,
        [
            (404, Some("unmatched"), None, 0),
            (405, Some("unmatched"), None, 0),
            (400, Some("bad-scope"), Some("GET /users/{id}"), 0),
        ]
    );
    assert!(
        got.iter()
            .all(|e| e.trace.is_empty() && e.failed_node.is_none())
    );
    for want in ["unmatched", "unmatched", "bad-scope"] {
        let Ok(BusEvent::Request(e)) = rx.try_recv() else {
            panic!("an event per rejection")
        };
        assert_eq!(e.rejected, Some(want));
        assert_eq!(e.nodes.len(), 0);
    }
}

#[tokio::test]
async fn a_rejected_request_does_not_touch_the_counter() {
    let st = app();
    let count = |st: &AppState| -> u64 {
        let g = st.generation().unwrap();
        g.counters.iter().map(|c| c.load(Ordering::Relaxed)).sum()
    };
    call(&st, "GET", "/users/1", &[]).await;
    let before = count(&st);
    call(&st, "GET", "/users/1", &[("x-kaanal-scenario", "bogus")]).await;
    call(&st, "GET", "/nope", &[]).await;
    call(&st, "DELETE", "/users/1", &[]).await;
    assert_eq!(count(&st), before);
    assert_eq!(entries(&st).len(), 4);
}

#[tokio::test]
async fn secrets_are_never_stored() {
    let st = app();
    let mut rx = st.events.subscribe();
    let r = HttpRequest::builder()
        .method("GET")
        .uri("/users/1?token=secret-query-789")
        .header("authorization", "Bearer secret-header-456")
        .header("x-custom", "secret-header-456")
        .header("x-request-id", "client-7")
        .body(Body::from("{\"password\":\"secret-body-123\"}"))
        .unwrap();
    handle(State(Arc::clone(&st)), r).await;
    let mut seen = serde_json::to_string(&entries(&st)).unwrap();
    while let Ok(BusEvent::Request(e)) = rx.try_recv() {
        seen.push_str(&serde_json::to_string(&e).unwrap());
    }
    for secret in ["secret-body-123", "secret-header-456", "secret-query-789"] {
        assert!(!seen.contains(secret), "{secret} leaked: {seen}");
    }
    assert!(seen.contains("client-7") && seen.contains("\"path\":\"/users/1\""));
}

#[tokio::test]
async fn pinned_request_keeps_chain_error() {
    let st = app();
    let r = call(&st, "GET", "/users/1", &[("x-kaanal-scenario", "hidden")]).await;
    assert_eq!(r.status(), 200);
    let e = &entries(&st)[0];
    assert_eq!(e.pin, Some("success"));
    assert_eq!(e.failed_node, None);
    let c = e.chain_error.as_ref().expect("the pin hid a chain error");
    assert_eq!(
        (c.node.as_str(), c.status, c.kind, c.cause),
        ("db", 503, "status", "chaos")
    );
    assert!(
        e.trace.ends_with("db.error=503,pinned=success"),
        "{}",
        e.trace
    );
    assert_eq!(e.flags, ["FI", "PN"]);
    // Unpinned, the same failure is the client's.
    call(&st, "GET", "/users/1", &[("x-kaanal-scenario", "down")]).await;
    let e = &entries(&st)[1];
    assert_eq!((e.status, e.failed_node.as_deref()), (503, Some("db")));
    assert_eq!(e.chain_error.as_ref().map(|c| c.status), Some(503));
}

#[tokio::test]
async fn out_of_order_completion_lands() {
    let st = app();
    let slow = tokio::spawn({
        let st = Arc::clone(&st);
        async move {
            call(&st, "GET", "/users/9", &[("x-kaanal-scenario", "lag")])
                .await
                .status()
        }
    });
    tokio::time::sleep(Duration::from_millis(30)).await;
    for _ in 0..3 {
        call(&st, "GET", "/users/1", &[]).await;
    }
    assert_eq!(entries(&st).len(), 3, "the slow request is still in flight");
    assert_eq!(slow.await.unwrap(), 200);
    let got = entries(&st);
    let last = got.last().unwrap();
    assert_eq!(
        (last.path.as_str(), last.seq, last.done),
        ("/users/9", 1, 4)
    );
    assert!(got[..3].iter().all(|e| e.seq > 1));
    // The latency chaos is attributed to the node's own hop, and flagged.
    assert_eq!(last.flags, ["DI"]);
    let db = last.hops.iter().find(|h| h.kind == "op").unwrap();
    assert_eq!(db.why.as_deref(), Some("chaos +300ms"));
}

#[tokio::test]
async fn concurrent_requests_all_recorded_once() {
    let st = app();
    let tasks: Vec<_> = (0..50)
        .map(|i| {
            let st = Arc::clone(&st);
            tokio::spawn(async move {
                let uri = if i % 5 == 0 { "/nope" } else { "/users/1" };
                call(&st, "GET", uri, &[]).await.status()
            })
        })
        .collect();
    for t in tasks {
        t.await.unwrap();
    }
    let got = entries(&st);
    assert_eq!(got.len(), 50);
    let mut seqs: Vec<_> = got.iter().map(|e| e.seq).collect();
    seqs.sort_unstable();
    assert_eq!(seqs, (1..=50).collect::<Vec<_>>());
    let mut done: Vec<_> = got.iter().map(|e| e.done).collect();
    done.sort_unstable();
    assert_eq!(done, (1..=50).collect::<Vec<_>>());
}

#[tokio::test]
async fn reset_empties_the_ring_but_ids_keep_counting() {
    let st = app();
    call(&st, "GET", "/users/1", &[]).await;
    crate::control::reset(State(Arc::clone(&st))).await.unwrap();
    assert_eq!(entries(&st).len(), 0);
    let r = call(&st, "GET", "/users/1", &[]).await;
    assert!(
        r.headers()["x-request-id"]
            .to_str()
            .unwrap()
            .ends_with("-0002")
    );
    assert_eq!(entries(&st).len(), 1);
}

#[tokio::test]
async fn engine_stopped_is_logged_with_an_id() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let st =
        Arc::new(AppState::build(&Config::from_yaml(YAML).unwrap(), &dir, "t".into(), 0).unwrap());
    let mut rx = st.events.subscribe();
    let r = call(&st, "GET", "/users/1", &[("x-kaanal-scenario", "hang")]).await;
    assert_eq!(r.status(), 500);
    let id = r.headers()["x-request-id"].to_str().unwrap().to_string();
    let got = entries(&st);
    assert_eq!(got.len(), 1);
    assert_eq!(
        (got[0].id.as_str(), got[0].rejected),
        (id.as_str(), Some("engine-stopped"))
    );
    assert_eq!(got[0].endpoint.as_deref(), Some("GET /users/{id}"));
    assert_eq!(got[0].scope.as_deref(), Some("hang"));
    let Ok(BusEvent::Request(e)) = rx.try_recv() else {
        panic!("one event")
    };
    assert_eq!(e.scope.as_deref(), Some("hang"));
    assert_eq!(
        (e.id, e.rejected, e.status),
        (id, Some("engine-stopped"), 500)
    );
    assert!(rx.try_recv().is_err(), "exactly one event");
}

#[tokio::test]
async fn a_dropped_request_leaves_one_client_gone_entry() {
    let st = app();
    let mut rx = st.events.subscribe();
    // The hang holds the request for the node's 150 ms timeout; the client gives up at 40 ms.
    let gone = tokio::time::timeout(
        Duration::from_millis(40),
        call(&st, "POST", "/orders", &[("x-kaanal-scenario", "hang")]),
    )
    .await;
    assert!(gone.is_err(), "the request was still running");
    let got = entries(&st);
    assert_eq!(got.len(), 1);
    let e = &got[0];
    assert_eq!((e.status, e.rejected), (499, Some("client-gone")));
    assert_eq!(e.endpoint.as_deref(), Some("POST /orders"));
    assert_eq!(e.scope.as_deref(), Some("hang"));
    assert_eq!((e.hops.len(), e.trace.as_str()), (0, ""));
    let Ok(BusEvent::Request(ev)) = rx.try_recv() else {
        panic!("one event")
    };
    assert_eq!(
        (ev.id.as_str(), ev.rejected),
        (e.id.as_str(), Some("client-gone"))
    );
    // The engine finishes the abandoned plan later; nothing is logged twice.
    tokio::time::sleep(Duration::from_millis(250)).await;
    assert_eq!(entries(&st).len(), 1);
    assert!(rx.try_recv().is_err());
    // A request that completes is pushed once, not also as client-gone.
    let r = call(&st, "GET", "/users/1", &[]).await;
    assert_eq!(r.status(), 200);
    let got = entries(&st);
    assert_eq!(got.len(), 2);
    assert_eq!(got[1].rejected, None);
}

#[tokio::test]
async fn a_hang_is_never_reported_as_injected_delay() {
    let st = app();
    let r = call(&st, "POST", "/orders", &[("x-kaanal-scenario", "hang")]).await;
    assert_eq!(r.status(), 504);
    let e = &entries(&st)[0];
    // The node's own `timeout:` ends the hang, so the engine calls it a deadline; either way
    // the long hop is not injected latency (the FI-only case is in `record.rs`).
    assert_eq!(e.flags, ["DT"]);
    assert!(e.hops.iter().all(|h| h.why.is_none()), "{:?}", e.hops);
    assert_eq!(e.chain_error.as_ref().unwrap().kind, "timeout");
}

#[tokio::test]
async fn injected_latency_is_attributed_on_nodes_caches_and_endpoints() {
    let st = app();
    call(&st, "GET", "/users/1", &[("x-kaanal-scenario", "eplag")]).await;
    call(&st, "GET", "/users", &[("x-kaanal-scenario", "cachelag")]).await;
    let got = entries(&st);
    let ep = got[0].hops.iter().find(|h| h.kind == "delay").unwrap();
    assert_eq!(ep.why.as_deref(), Some("chaos +20ms"));
    assert_eq!(got[0].flags, ["DI"]);
    let hit = got[1].hops.iter().find(|h| h.kind == "hit").unwrap();
    assert_eq!(hit.why.as_deref(), Some("chaos +40ms"));
    assert_eq!(got[1].flags, ["DI"]);
    let op = got[0].hops.iter().find(|h| h.kind == "op").unwrap();
    assert_eq!(op.op, Some("read"));
}

#[tokio::test]
async fn a_pin_over_a_reset_keeps_the_reset_kind() {
    let st = app();
    let r = call(&st, "GET", "/users/1", &[("x-kaanal-scenario", "hreset")]).await;
    assert_eq!(r.status(), 200);
    let e = &entries(&st)[0];
    let c = e.chain_error.as_ref().unwrap();
    assert_eq!((c.node.as_str(), c.kind, c.cause), ("db", "reset", "chaos"));
    assert!(e.trace.contains("db.error=reset"), "{}", e.trace);
}

#[tokio::test]
async fn long_paths_and_methods_are_capped_in_the_log() {
    let st = app();
    let path = format!("/{}é", "x".repeat(3000));
    call(&st, "GET", &path, &[]).await;
    call(&st, "PROPFINDPROPFINDPROPFIND", "/users/1", &[]).await;
    let got = entries(&st);
    assert!(got[0].path.len() <= 2048 && got[0].path.ends_with('…'));
    assert!(got[0].path.starts_with("/xxx"));
    assert_eq!(got[0].status, 404);
    assert!(got[1].method.len() <= 16 && got[1].method.ends_with('…'));
    // Short values are kept whole.
    call(&st, "GET", "/users/1", &[]).await;
    assert_eq!(entries(&st)[2].path, "/users/1");
}

/// Paused time: the engine's 60 s `HANG_CAP` sleep auto-advances instead of taking a minute.
#[tokio::test(start_paused = true)]
async fn a_hang_on_a_node_without_a_timeout_is_fault_injection() {
    let st = app();
    let r = call(&st, "GET", "/users/1", &[("x-kaanal-scenario", "dbhang")]).await;
    assert_eq!(r.status(), 504);
    let e = &entries(&st)[0];
    assert_eq!(e.flags, ["FI"]);
    assert!(e.hops.iter().all(|h| h.why.is_none()), "{:?}", e.hops);
    let c = e.chain_error.as_ref().unwrap();
    assert_eq!((c.kind, c.cause), ("timeout", "chaos"));
}

/// `YAML` with a 1000 B/s link between `app` and `db`.
fn narrow() -> Arc<AppState> {
    // no headers: these tests check what the body costs (the default adds 600 B and 400 B)
    app_with(
        &YAML
            .replace(
                "- { from: app, to: db, net: 1ms }",
                "- { from: app, to: db, net: 1ms, bandwidth: 1000 }",
            )
            .replacen("version: 1\n", "version: 1\nheader_bytes: 0\n", 1),
    )
}

#[tokio::test]
async fn a_mock_with_a_narrow_link_delays_the_response() {
    let st = narrow();
    call(&st, "GET", "/users/1", &[("content-length", "50")]).await;
    let e = &entries(&st)[0];
    let links: Vec<_> = e
        .hops
        .iter()
        .filter(|h| h.kind.starts_with("link"))
        .collect();
    // 50 B over 1000 B/s is 50 ms on the way in; the generated body comes back over the same link
    assert_eq!((links[0].kind, links[0].dur_ms), ("link", 50.0));
    assert_eq!(links[1].kind, "link-resp");
    assert!(links[1].dur_ms > 0.0, "the response has a size");
    assert!(e.trace.contains("db.link=50"), "{}", e.trace);
    assert!(e.ms >= 50, "the client waited the transfer: {} ms", e.ms);
}

#[tokio::test]
async fn a_chunked_upload_counts_the_bytes_the_client_really_sent() {
    // no Content-Length: three 20 B chunks over a 1000 B/s link are 60 ms on the way in
    let st = narrow();
    let chunks: Vec<Result<Bytes, std::io::Error>> =
        (0..3).map(|_| Ok(Bytes::from(vec![b'x'; 20]))).collect();
    let body = Body::from_stream(tokio_stream::iter(chunks));
    let req = HttpRequest::builder()
        .method("GET")
        .uri("/users/1")
        .body(body)
        .unwrap();
    handle(State(Arc::clone(&st)), req).await;
    let e = &entries(&st)[0];
    let first = e
        .hops
        .iter()
        .find(|h| h.kind == "link")
        .expect("a request transfer");
    assert_eq!(first.dur_ms, 60.0, "{}", e.trace);
}

#[tokio::test]
async fn no_payload_costs_means_no_link_hops_whatever_the_body() {
    let st = app();
    assert!(!st.generation().unwrap().topo.payload_costs());
    call(&st, "GET", "/users/1", &[("content-length", "5000")]).await;
    let e = &entries(&st)[0];
    assert!(e.hops.iter().all(|h| !h.kind.starts_with("link")));
}

#[tokio::test]
async fn a_head_response_carries_no_response_transfer() {
    let st = narrow();
    call(&st, "HEAD", "/users/1", &[("content-length", "50")]).await;
    let e = &entries(&st)[0];
    let kinds: Vec<_> = e
        .hops
        .iter()
        .map(|h| h.kind)
        .filter(|k| k.starts_with("link"))
        .collect();
    assert_eq!(kinds, ["link"], "hyper strips a HEAD body");
}

#[tokio::test]
async fn a_blank_state_answers_404_and_a_system_answers_again_once_installed() {
    let st = app();
    let g = st.generation().unwrap();
    st.set_generation(None);
    let r = call(&st, "GET", "/users/1", &[]).await;
    assert_eq!(r.status(), 404);
    assert!(entries(&st).is_empty(), "no id, no counter, no log entry");
    st.set_generation(Some(g));
    assert_eq!(call(&st, "GET", "/users/1", &[]).await.status(), 200);
}
