use kaanal_core::Config;
use kaanal_core::config::Determinism;
use kaanal_server::ServeOpts;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};

pub struct Srv {
    pub base: String,
    pub token: String,
    pub client: reqwest::Client,
    pub drawn_seed: Option<u64>,
}

pub async fn start() -> Srv {
    start_seeded(None).await
}

fn fixtures() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

/// Serves `cfg` with spec paths resolved against `dir`.
pub async fn start_in(cfg: &Config, dir: &Path) -> Srv {
    let token = kaanal_server::new_token();
    let opts = ServeOpts {
        host: "127.0.0.1".parse().unwrap(),
        port: 0,
        token: token.clone(),
    };
    let s = kaanal_server::serve(cfg, dir, opts).await.unwrap();
    Srv {
        base: format!("http://127.0.0.1:{}", s.addr.port()),
        token,
        client: reqwest::Client::new(),
        drawn_seed: s.drawn_seed,
    }
}

/// Serves a whole `kaanal.yaml` given as text, next to the fixture's `openapi.yaml`.
pub async fn start_yaml(src: &str) -> Srv {
    start_in(&Config::from_yaml(src).unwrap(), &fixtures()).await
}

/// Like [`start`], optionally overriding the fixture's `seed:`.
pub async fn start_seeded(seed: Option<u64>) -> Srv {
    let mut cfg =
        Config::from_yaml(&std::fs::read_to_string(fixtures().join("kaanal.yaml")).unwrap())
            .unwrap();
    if let Some(seed) = seed {
        cfg.seed = seed;
    }
    start_in(&cfg, &fixtures()).await
}

/// Gateway, the fixture's service and a database, every hop 1 ms fixed; tests append entries.
const GW_APP_DB: &str = "version: 1\nnodes:\n  nginx: { type: gateway, proc: 1ms }\n  app: { type: service, spec: openapi.yaml, proc: 1ms }\n  db: { type: database, ops: { read: 1ms, write: 1ms, scan: 1ms } }\nedges:\n  - { from: nginx, to: app, net: 1ms }\n  - { from: app, to: db, net: 1ms }\nendpoints:\n";

fn trace_of(r: &reqwest::Response) -> String {
    r.headers()["x-kaanal-trace"].to_str().unwrap().to_string()
}

impl Srv {
    pub async fn get(&self, path: &str) -> reqwest::Response {
        self.client
            .get(format!("{}{path}", self.base))
            .send()
            .await
            .unwrap()
    }
}

#[tokio::test]
async fn users_42_is_stable_traced_and_uses_path_id() {
    let s = start().await;
    let r1 = s.get("/users/42").await;
    assert_eq!(r1.status(), 200);
    let trace = r1.headers()["x-kaanal-trace"].to_str().unwrap().to_string();
    assert!(
        trace.starts_with("nginx.proc=") && trace.contains("db.read="),
        "{trace}"
    );
    let b1: Value = r1.json().await.unwrap();
    let b2: Value = s.get("/users/42").await.json().await.unwrap();
    assert_eq!(b1, b2);
    assert_eq!(b1["id"], 42);
    assert!(b1["email"].as_str().unwrap().ends_with("@example.com"));
}

#[tokio::test]
async fn non_numeric_id_for_integer_field_still_returns_200() {
    let s = start().await;
    let r = s.get("/users/abc").await;
    assert_eq!(r.status(), 200);
    assert!(r.json::<Value>().await.unwrap()["id"].is_i64());
}

#[tokio::test]
async fn static_override_beats_generated() {
    let s = start().await;
    let body: Value = s.get("/users").await.json().await.unwrap();
    assert_eq!(body, json!([{ "id": 1 }]));
}

#[tokio::test]
async fn static_only_route_works() {
    let s = start().await;
    let r = s.get("/health").await;
    assert_eq!(r.status(), 200);
    assert_eq!(r.json::<Value>().await.unwrap(), json!({ "ok": true }));
}

#[tokio::test]
async fn post_orders_returns_201() {
    let s = start().await;
    let r = s
        .client
        .post(format!("{}/orders", s.base))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 201);
    let b: Value = r.json().await.unwrap();
    assert_eq!(b["currency"], "INR");
}

#[tokio::test]
async fn unknown_and_trailing_slash_paths_are_404() {
    let s = start().await;
    for p in ["/nope", "/users/42/", "/__kaanal/nope"] {
        let r = s.get(p).await;
        assert_eq!(r.status(), 404, "{p}");
        assert_eq!(r.json::<Value>().await.unwrap()["error"], "no route");
    }
}

#[tokio::test]
async fn wrong_method_is_405() {
    let s = start().await;
    let r = s
        .client
        .delete(format!("{}/users/1", s.base))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 405);
}

#[tokio::test]
async fn mocks_send_cors_headers_and_answer_preflight() {
    let s = start().await;
    let r = s.get("/users/1").await;
    assert_eq!(r.headers()["access-control-allow-origin"], "*");
    let pre = s
        .client
        .request(reqwest::Method::OPTIONS, format!("{}/orders", s.base))
        .send()
        .await
        .unwrap();
    assert_eq!(pre.status(), 204);
    assert_eq!(pre.headers()["access-control-allow-origin"], "*");
}

#[tokio::test]
async fn health_needs_no_token_and_has_no_cors() {
    let s = start().await;
    let r = s.get("/__kaanal/health").await;
    assert_eq!(r.status(), 200);
    assert!(r.headers().get("access-control-allow-origin").is_none());
}

#[tokio::test]
async fn control_namespace_never_gets_cors_even_for_404_and_preflight() {
    let s = start().await;
    let r = s.get("/__kaanal/nope").await;
    assert_eq!(r.status(), 404);
    assert!(r.headers().get("access-control-allow-origin").is_none());
    let pre = s
        .client
        .request(
            reqwest::Method::OPTIONS,
            format!("{}/__kaanal/chaos", s.base),
        )
        .send()
        .await
        .unwrap();
    assert!(pre.headers().get("access-control-allow-origin").is_none());
}

#[tokio::test]
async fn head_answers_like_get_without_a_body() {
    let s = start().await;
    let r = s
        .client
        .head(format!("{}/users/42", s.base))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    assert!(r.headers().contains_key("x-kaanal-trace"));
    assert_eq!(r.bytes().await.unwrap(), "");
}

#[tokio::test]
async fn head_has_its_own_counter_and_never_shifts_get() {
    // R6.8: with 50% db errors, each GET's outcome depends on GET's counter alone.
    let status = |r: reqwest::Response| r.status().as_u16();
    let plain = start().await;
    plain.put_chaos("db", json!({ "error_rate": 0.5 })).await;
    let mut want = Vec::new();
    for _ in 0..8 {
        want.push(status(plain.get("/users/42").await));
    }
    let mixed = start().await;
    mixed.put_chaos("db", json!({ "error_rate": 0.5 })).await;
    let mut got = Vec::new();
    for _ in 0..8 {
        got.push(status(mixed.get("/users/42").await));
        let head = mixed
            .client
            .head(format!("{}/users/42", mixed.base))
            .send()
            .await
            .unwrap();
        assert!(head.status() == 200 || head.status() == 503);
    }
    assert_eq!(got, want, "HEADs between GETs must not change GET outcomes");
    assert!(want.contains(&200) && want.contains(&503), "{want:?}");
}

#[tokio::test]
async fn percent_encoded_path_param_is_decoded_before_use() {
    let s = start().await;
    // %34%32 is "42"
    let b: Value = s.get("/users/%34%32").await.json().await.unwrap();
    assert_eq!(b["id"], 42);
}

impl Srv {
    pub fn auth(&self, rb: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        rb.bearer_auth(&self.token)
    }
    pub async fn put_chaos(&self, node: &str, body: Value) -> reqwest::Response {
        self.auth(
            self.client
                .put(format!("{}/__kaanal/chaos/{node}", self.base)),
        )
        .json(&body)
        .send()
        .await
        .unwrap()
    }
    pub async fn get_authed(&self, path: &str) -> reqwest::Response {
        self.auth(self.client.get(format!("{}{path}", self.base)))
            .send()
            .await
            .unwrap()
    }
}

#[tokio::test]
async fn chaos_on_db_fails_requests_then_clears() {
    let s = start().await;
    let r = s
        .put_chaos("db", json!({ "error_rate": 1.0, "error_status": 503 }))
        .await;
    assert_eq!(r.status(), 200);
    assert_eq!(r.json::<Value>().await.unwrap()["error_rate"], 1.0);

    let failed = s.get("/users/42").await;
    assert_eq!(failed.status(), 503);
    assert_eq!(
        failed.json::<Value>().await.unwrap(),
        json!({ "error": "chaos", "node": "db" })
    );

    let cleared = s
        .auth(s.client.delete(format!("{}/__kaanal/chaos/db", s.base)))
        .send()
        .await
        .unwrap();
    assert_eq!(cleared.status(), 204);
    assert_eq!(s.get("/users/42").await.status(), 200);
}

#[tokio::test]
async fn chaos_patch_merges_and_lists() {
    let s = start().await;
    s.put_chaos("app", json!({ "latency_ms": 50 })).await;
    s.put_chaos("app", json!({ "down": true })).await;
    let all: Value = s.get_authed("/__kaanal/chaos").await.json().await.unwrap();
    assert_eq!(
        all["app"],
        json!({ "latency_ms": 50, "error_rate": 0.0, "error_status": 503, "down": true })
    );
    let wiped = s
        .auth(s.client.delete(format!("{}/__kaanal/chaos", s.base)))
        .send()
        .await
        .unwrap();
    assert_eq!(wiped.status(), 204);
    let all: Value = s.get_authed("/__kaanal/chaos").await.json().await.unwrap();
    assert_eq!(all, json!({}));
}

#[tokio::test]
async fn latency_chaos_slows_requests() {
    let s = start().await;
    s.put_chaos("db", json!({ "latency_ms": 200 })).await;
    let t = std::time::Instant::now();
    assert_eq!(s.get("/users/1").await.status(), 200);
    assert!(t.elapsed() >= std::time::Duration::from_millis(200));
}

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

#[tokio::test]
async fn unknown_node_is_404_with_message() {
    let s = start().await;
    let r = s.put_chaos("redis", json!({ "down": true })).await;
    assert_eq!(r.status(), 404);
    assert_eq!(
        r.json::<Value>().await.unwrap()["error"],
        "unknown node 'redis'"
    );
}

#[tokio::test]
async fn bad_chaos_values_are_4xx_with_json_errors() {
    let s = start().await;
    for bad in [
        json!({ "error_rate": 1.5 }),
        json!({ "error_status": 418 }),
        json!({ "latency_ms": 300_001 }),
        json!({ "latency_ms": -5 }),
        json!({ "bogus": 1 }),
    ] {
        let r = s.put_chaos("db", bad.clone()).await;
        assert!(r.status().is_client_error(), "{bad}: {}", r.status());
        assert!(
            r.json::<Value>().await.unwrap()["error"].is_string(),
            "{bad}"
        );
    }
    let rate = s.put_chaos("db", json!({ "error_rate": 1.5 })).await;
    assert_eq!(rate.status(), 400);
    let slow = s.put_chaos("db", json!({ "latency_ms": 300_001 })).await;
    assert_eq!(slow.status(), 400);
    // A rejected patch changes nothing.
    let all: Value = s.get_authed("/__kaanal/chaos").await.json().await.unwrap();
    assert_eq!(all, json!({}));
    let cap = s.put_chaos("db", json!({ "latency_ms": 300_000 })).await;
    assert_eq!(cap.status(), 200);
}

#[tokio::test]
async fn topology_lists_nodes_edges_routes() {
    let s = start().await;
    let t: Value = s
        .get_authed("/__kaanal/topology")
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(
        t["nodes"][0],
        json!({ "name": "nginx", "type": "gateway", "flavor": "nginx" })
    );
    assert_eq!(t["edges"][1], json!({ "from": "app", "to": "db" }));
    let routes: Vec<&str> = t["routes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r.as_str().unwrap())
        .collect();
    assert_eq!(
        routes,
        vec![
            "GET /health",
            "GET /users",
            "GET /users/{id}",
            "POST /orders"
        ]
    );
}

#[tokio::test]
async fn events_stream_reports_requests() {
    let s = start().await;
    let mut stream = s
        .client
        .get(format!("{}/__kaanal/events?token={}", s.base, s.token))
        .send()
        .await
        .unwrap();
    assert_eq!(stream.status(), 200);
    s.get("/users/7").await;
    let mut seen = String::new();
    let found = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while let Some(chunk) = stream.chunk().await.unwrap() {
            seen.push_str(&String::from_utf8_lossy(&chunk));
            if seen.contains("event: request") && seen.contains("/users/7") {
                return true;
            }
        }
        false
    })
    .await
    .unwrap_or(false);
    assert!(found, "stream so far: {seen}");
    assert!(
        seen.contains("\"nodes\":[\"nginx\",\"app\",\"db\"]"),
        "{seen}"
    );
    // `ms` is measured wall time, so it is at least the 5 simulated 1ms hops
    let data = seen.lines().find(|l| l.starts_with("data:")).unwrap();
    let ev: Value = serde_json::from_str(data.trim_start_matches("data:").trim()).unwrap();
    assert!(ev["ms"].as_u64().unwrap() >= 5, "{ev}");
}

#[tokio::test]
async fn events_without_token_is_401() {
    let s = start().await;
    assert_eq!(s.get("/__kaanal/events").await.status(), 401);
}

#[tokio::test]
async fn ui_page_cannot_be_framed_and_sends_no_referrer() {
    let s = start().await;
    let r = s.get("/__kaanal/ui").await;
    assert_eq!(r.status(), 200);
    let csp = r.headers()["content-security-policy"].to_str().unwrap();
    assert!(csp.contains("frame-ancestors 'none'"), "{csp}");
    assert_eq!(r.headers()["referrer-policy"], "no-referrer");
}

#[tokio::test]
async fn ui_page_is_served_without_token() {
    let s = start().await;
    let r = s.get("/__kaanal/ui").await;
    assert_eq!(r.status(), 200);
    assert!(
        r.headers()["content-type"]
            .to_str()
            .unwrap()
            .starts_with("text/html")
    );
    let html = r.text().await.unwrap();
    assert!(
        html.contains("Kaanal") && html.contains("EventSource") && html.contains("location.hash")
    );
}

/// Static guard for the page's security rules: no HTML-string sinks, no eval, no inline
/// handlers, no token storage, no external URLs. Request paths and node names are untrusted.
#[tokio::test]
async fn ui_page_avoids_unsafe_dom_apis_and_external_assets() {
    let s = start().await;
    let html = s.get("/__kaanal/ui").await.text().await.unwrap();
    for bad in [
        "innerHTML",
        "outerHTML",
        "insertAdjacentHTML",
        "document.write",
        "eval(",
        "onclick=",
        "localStorage",
        "http://",
        "https://",
    ] {
        assert!(!html.contains(bad), "page contains {bad}");
    }
}

#[tokio::test]
async fn ui_controls_have_accessible_names_and_dark_form_controls() {
    let s = start().await;
    let html = s.get("/__kaanal/ui").await.text().await.unwrap();
    assert!(html.contains("color-scheme: light dark"));
    for needle in ["setAttribute('aria-label'", "role=\"alert\""] {
        assert!(html.contains(needle), "missing {needle}");
    }
}

#[tokio::test]
async fn ui_keeps_stream_status_and_api_errors_on_separate_lines_and_uses_counters() {
    let s = start().await;
    let html = s.get("/__kaanal/ui").await.text().await.unwrap();
    assert!(html.contains("id=\"note\"") && html.contains("role=\"status\""));
    assert!(html.contains("const note = ") && html.contains("const say = "));
    // fixed-size counters, not a growing event list filtered every second
    assert!(html.contains("const buckets = ") && !html.contains("recent.filter"));
    // node names are used as Map keys, never as plain object keys
    assert!(!html.contains("stats[") && html.contains("new Map()"));
}

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

async fn statuses(s: &Srv) -> Vec<u16> {
    s.put_chaos("db", json!({ "error_rate": 0.5 })).await;
    let mut out = Vec::new();
    for _ in 0..16 {
        out.push(s.get("/users/42").await.status().as_u16());
    }
    out
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
