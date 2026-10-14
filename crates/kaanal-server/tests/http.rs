use kaanal_core::Config;
use kaanal_core::config::Determinism;
use kaanal_server::ServeOpts;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::Duration;

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

/// One signing key per test binary (spec §7); every server gets a clone.
fn test_key() -> kaanal_server::AuthKey {
    static KEY: std::sync::OnceLock<kaanal_server::AuthKey> = std::sync::OnceLock::new();
    KEY.get_or_init(|| kaanal_server::AuthKey::generate().unwrap())
        .clone()
}

/// Serves `cfg` with spec paths resolved against `dir`.
pub async fn start_in(cfg: &Config, dir: &Path) -> Srv {
    let token = kaanal_server::new_token();
    let opts = ServeOpts {
        host: "127.0.0.1".parse().unwrap(),
        port: 0,
        token: token.clone(),
        auth_key: Some(test_key()),
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
        all["nodes"]["app"],
        json!({ "latency_ms": 50, "error_rate": 0.0, "error_kind": "status", "error_status": 503, "down": true })
    );
    let wiped = s
        .auth(s.client.delete(format!("{}/__kaanal/chaos", s.base)))
        .send()
        .await
        .unwrap();
    assert_eq!(wiped.status(), 204);
    let all: Value = s.get_authed("/__kaanal/chaos").await.json().await.unwrap();
    assert_eq!(all, json!({ "nodes": {}, "endpoints": {}, "node_at": [] }));
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
async fn a_malformed_endpoint_is_400_on_set_and_clear() {
    let s = start().await;
    let r = s
        .auth(
            s.client
                .put(format!("{}/__kaanal/chaos", s.base))
                .json(&json!({ "endpoint": "GET", "down": true })),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 400);
    assert!(r.json::<Value>().await.unwrap()["error"].is_string());
    let r = s
        .auth(
            s.client
                .delete(format!("{}/__kaanal/chaos?endpoint=nope", s.base)),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 400);
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
    assert_eq!(all, json!({ "nodes": {}, "endpoints": {}, "node_at": [] }));
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
async fn events_carry_the_failure_kind_for_non_status_failures() {
    let s = start().await;
    let mut stream = s
        .client
        .get(format!("{}/__kaanal/events?token={}", s.base, s.token))
        .send()
        .await
        .unwrap();
    s.put_chaos(
        "app",
        json!({ "error_rate": 1.0, "error_kind": "malformed" }),
    )
    .await;
    s.get("/users/7").await;
    s.put_chaos("app", json!({ "error_kind": "status" })).await;
    s.get("/users/8").await;
    let mut seen = String::new();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while let Some(chunk) = stream.chunk().await.unwrap() {
            seen.push_str(&String::from_utf8_lossy(&chunk));
            if seen.contains("/users/8") {
                break;
            }
        }
    })
    .await
    .unwrap();
    let evs: Vec<Value> = seen
        .lines()
        .filter_map(|l| l.strip_prefix("data:"))
        .map(|d| serde_json::from_str(d.trim()).unwrap())
        .collect();
    assert_eq!(evs[0]["kind"], "malformed", "{seen}");
    assert_eq!(evs[0]["failed_node"], "app");
    assert!(
        evs[1]["kind"].is_null(),
        "a status failure has no kind: {seen}"
    );
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

#[tokio::test]
async fn serve_without_a_signing_key_fails_closed() {
    let cfg = Config::from_yaml("version: 1\nnodes:\n  login: { type: auth }\n").unwrap();
    let opts = ServeOpts {
        host: "127.0.0.1".parse().unwrap(),
        port: 0,
        token: "t".into(),
        auth_key: None,
    };
    let e = kaanal_server::serve(&cfg, &fixtures(), opts)
        .await
        .err()
        .unwrap();
    assert_eq!(
        e.msg,
        "auth node 'login' needs a signing key (kaanal up provides one)"
    );
}

/// A gateway in front of an auth node (`proc` 50 ms, so latency lower bounds survive the
/// Windows timer) with two users; tests append `endpoints:` entries.
const AUTH: &str = "version: 1
nodes:
  nginx: { type: gateway, proc: 1ms }
  login:
    type: auth
    proc: 50ms
    token_ttl: 2h
    claims: { aud: my-app, tier: free }
    users:
      - { username: alice, password: s3cr3t-pw, roles: [admin], scopes: [read, write], claims: { tier: pro } }
      - { username: bob, password: pw }
edges:
  - { from: nginx, to: login, net: 1ms }
endpoints:
";

const ALICE: &str = r#"{"username":"alice","password":"s3cr3t-pw"}"#;

impl Srv {
    /// `POST /login` with `body` and, if given, a `Content-Type`.
    async fn login(
        &self,
        content_type: Option<&str>,
        body: impl Into<reqwest::Body>,
    ) -> reqwest::Response {
        let mut rb = self.client.post(format!("{}/login", self.base)).body(body);
        if let Some(ct) = content_type {
            rb = rb.header("content-type", ct);
        }
        rb.send().await.unwrap()
    }

    fn port(&self) -> u16 {
        self.base.rsplit(':').next().unwrap().parse().unwrap()
    }
}

/// Verifies `token` against the server's own JWKS with an independent library (X13) and
/// returns its claims.
async fn verify(s: &Srv, token: &str) -> Value {
    let set: jsonwebtoken::jwk::JwkSet =
        s.get("/.well-known/jwks.json").await.json().await.unwrap();
    let kid = jsonwebtoken::decode_header(token).unwrap().kid.unwrap();
    let key = jsonwebtoken::DecodingKey::from_jwk(set.find(&kid).unwrap()).unwrap();
    let mut v = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::RS256);
    v.set_issuer(&[format!("http://localhost:{}", s.port())]);
    v.validate_aud = false;
    jsonwebtoken::decode::<Value>(token, &key, &v)
        .unwrap()
        .claims
}

#[tokio::test]
async fn login_with_json_returns_a_token_that_verifies_against_the_served_jwks() {
    let s = start_yaml(AUTH).await;
    let r = s
        .login(
            Some("application/json; charset=UTF-8"),
            r#"{"username":"alice","password":"s3cr3t-pw","grant_type":"password"}"#,
        )
        .await;
    assert_eq!(r.status(), 200);
    assert_eq!(r.headers()["content-type"], "application/json");
    assert_eq!(trace_of(&r), "nginx.proc=1.0ms,net=1.0ms,login.proc=50.0ms");
    let b: Value = r.json().await.unwrap();
    assert_eq!(b["token_type"], "Bearer");
    assert_eq!(b["expires_in"], 7200);
    assert_eq!(b["scope"], "read write");
    let c = verify(&s, b["access_token"].as_str().unwrap()).await;
    assert_eq!(c["sub"], "alice");
    assert_eq!(c["roles"], json!(["admin"]));
    assert_eq!(c["scope"], "read write");
    assert_eq!(
        (c["aud"].clone(), c["tier"].clone()),
        (json!("my-app"), json!("pro"))
    );
    assert_eq!(
        c["exp"].as_u64().unwrap() - c["iat"].as_u64().unwrap(),
        7200
    );
}

#[tokio::test]
async fn login_with_form_data_works_and_omits_scope_without_scopes() {
    let s = start_yaml(AUTH).await;
    let r = s
        .login(
            Some("Application/X-WWW-Form-Urlencoded"),
            "username=bob&password=pw",
        )
        .await;
    assert_eq!(r.status(), 200);
    let b: Value = r.json().await.unwrap();
    assert!(b.get("scope").is_none(), "{b}");
    let c = verify(&s, b["access_token"].as_str().unwrap()).await;
    assert_eq!(
        (c["sub"].clone(), c["tier"].clone()),
        (json!("bob"), json!("free"))
    );
    assert!(c.get("scope").is_none(), "{c}");
    // Percent escapes are decoded.
    let r = s
        .login(
            Some("application/x-www-form-urlencoded"),
            "username=alice&password=s3cr3t%2Dpw",
        )
        .await;
    assert_eq!(r.status(), 200);
}

#[tokio::test]
async fn wrong_or_unknown_credentials_are_401_after_the_chain_latency() {
    let s = start_yaml(AUTH).await;
    for body in [
        r#"{"username":"alice","password":"nope"}"#,
        r#"{"username":"mallory","password":"s3cr3t-pw"}"#,
    ] {
        let t = std::time::Instant::now();
        let r = s.login(Some("application/json"), body).await;
        assert!(
            t.elapsed() >= Duration::from_millis(50),
            "{:?}",
            t.elapsed()
        );
        assert_eq!(r.status(), 401, "{body}");
        assert_eq!(
            r.json::<Value>().await.unwrap(),
            json!({ "error": "invalid_credentials" })
        );
    }
}

#[tokio::test]
async fn missing_or_text_plain_content_type_is_415() {
    let s = start_yaml(AUTH).await;
    // `fetch(url, { method: "POST", body: JSON.stringify(x) })` sends text/plain.
    for ct in [
        None,
        Some("text/plain;charset=UTF-8"),
        Some("application/jsonx"),
    ] {
        let t = std::time::Instant::now();
        let r = s.login(ct, ALICE).await;
        assert!(
            t.elapsed() >= Duration::from_millis(50),
            "{:?}",
            t.elapsed()
        );
        assert_eq!(r.status(), 415, "{ct:?}");
        assert_eq!(
            r.json::<Value>().await.unwrap(),
            json!({ "error": "unsupported_media_type" })
        );
    }
}

#[tokio::test]
async fn malformed_duplicate_or_non_string_fields_are_400_without_echo() {
    let s = start_yaml(AUTH).await;
    let cases = [
        (
            "application/json",
            r#"{"username":"alice","password":"s3cr3t-pw","password":"s3cr3t-pw"}"#,
        ),
        ("application/json", r#"{"username":"alice","password":123}"#),
        ("application/json", r#"{"username":"alice"}"#),
        (
            "application/json",
            r#"{"username":"alice","password":"s3cr3t-pw""#,
        ),
        ("application/json", "[]"),
        (
            "application/x-www-form-urlencoded",
            "username=alice&password=s3cr3t-pw&password=s3cr3t-pw",
        ),
        ("application/x-www-form-urlencoded", "username=alice"),
    ];
    for (ct, body) in cases {
        let r = s.login(Some(ct), body).await;
        assert_eq!(r.status(), 400, "{body}");
        // Exactly this text: parser messages (which can quote the password) never leak.
        assert_eq!(
            r.text().await.unwrap(),
            r#"{"error":"invalid_request"}"#,
            "{body}"
        );
    }
}

#[tokio::test]
async fn oversized_login_body_is_413() {
    let s = start_yaml(AUTH).await;
    let with_pad = |n: usize| {
        format!(
            r#"{{"username":"alice","password":"s3cr3t-pw","pad":"{}"}}"#,
            "x".repeat(n)
        )
    };
    let at_cap = with_pad(16 * 1024 - with_pad(0).len());
    assert_eq!(at_cap.len(), 16 * 1024);
    assert_eq!(
        s.login(Some("application/json"), at_cap).await.status(),
        200
    );
    let r = s.login(Some("application/json"), with_pad(16 * 1024)).await;
    assert_eq!(r.status(), 413);
    assert_eq!(
        r.json::<Value>().await.unwrap(),
        json!({ "error": "request_too_large" })
    );
}

#[tokio::test]
async fn every_login_response_is_no_store() {
    let s = start_yaml(AUTH).await;
    let cases = [
        (Some("application/json"), ALICE.to_string(), 200),
        (
            Some("application/json"),
            r#"{"username":"alice","password":"x"}"#.to_string(),
            401,
        ),
        (Some("application/json"), "{".to_string(), 400),
        (Some("application/json"), "x".repeat(16 * 1024 + 1), 413),
        (None, ALICE.to_string(), 415),
    ];
    for (ct, body, status) in cases {
        let r = s.login(ct, body).await;
        assert_eq!(r.status(), status);
        assert_eq!(r.headers()["cache-control"], "no-store", "{status}");
        assert_eq!(r.headers()["pragma"], "no-cache", "{status}");
    }
    // The JWKS is a public document: no such header.
    let jwks = s.get("/.well-known/jwks.json").await;
    assert!(jwks.headers().get("cache-control").is_none());
}

#[tokio::test]
async fn chain_failure_beats_body_errors_and_goes_through_error_map() {
    let s = start_yaml(AUTH).await;
    s.put_chaos("login", json!({ "down": true })).await;
    for (ct, body) in [
        (Some("application/json"), ALICE.to_string()),
        (Some("application/json"), "x".repeat(16 * 1024 + 1)),
        (None, "x".to_string()),
        (Some("application/json"), "{".to_string()),
    ] {
        let r = s.login(ct, body).await;
        assert_eq!(r.status(), 503);
        assert_eq!(trace_of(&r), "nginx.proc=1.0ms,net=1.0ms,login.error=503");
        assert_eq!(
            r.json::<Value>().await.unwrap(),
            json!({ "error": "chaos", "node": "login" })
        );
    }
    assert_eq!(s.get("/.well-known/jwks.json").await.status(), 503);
    // An `error_map` on the built-in route's entry shapes the failure (X9).
    let s = start_yaml(&format!(
        "{AUTH}  \"POST /login\": {{ error_map: {{ login: {{ status: 502, body: {{ e: 1 }} }} }} }}\n"
    ))
    .await;
    s.put_chaos("login", json!({ "error_rate": 1.0, "error_status": 500 }))
        .await;
    let r = s.login(Some("application/json"), ALICE).await;
    assert_eq!(r.status(), 502);
    assert_eq!(r.json::<Value>().await.unwrap(), json!({ "e": 1 }));
}

#[tokio::test]
async fn auth_latency_chaos_slows_login_and_jwks() {
    let s = start_yaml(AUTH).await;
    s.put_chaos("login", json!({ "latency_ms": 200 })).await;
    let t = std::time::Instant::now();
    assert_eq!(s.login(Some("application/json"), ALICE).await.status(), 200);
    assert!(
        t.elapsed() >= Duration::from_millis(250),
        "{:?}",
        t.elapsed()
    );
    let t = std::time::Instant::now();
    assert_eq!(s.get("/.well-known/jwks.json").await.status(), 200);
    assert!(
        t.elapsed() >= Duration::from_millis(250),
        "{:?}",
        t.elapsed()
    );
}

#[tokio::test]
async fn override_replaces_the_login_responder() {
    let s = start_yaml(&format!(
        "{AUTH}  \"POST /login\": {{ override: {{ body: {{ access_token: fixed }} }} }}\n"
    ))
    .await;
    // No Content-Type and no credentials: the override answers anyway (it is never read).
    let r = s.login(None, "").await;
    assert_eq!(r.status(), 200);
    assert_eq!(r.headers()["cache-control"], "no-store");
    assert_eq!(
        r.json::<Value>().await.unwrap(),
        json!({ "access_token": "fixed" })
    );
}

#[tokio::test]
async fn login_and_jwks_send_cors_and_answer_preflight() {
    let s = start_yaml(AUTH).await;
    let pre = s
        .client
        .request(reqwest::Method::OPTIONS, format!("{}/login", s.base))
        .header("origin", "http://localhost:3000")
        .header("access-control-request-method", "POST")
        .header("access-control-request-headers", "content-type")
        .send()
        .await
        .unwrap();
    assert_eq!(pre.status(), 204);
    assert_eq!(pre.headers()["access-control-allow-origin"], "*");
    let r = s.login(Some("application/json"), ALICE).await;
    assert_eq!(r.headers()["access-control-allow-origin"], "*");
    let jwks = s.get("/.well-known/jwks.json").await;
    assert_eq!(jwks.headers()["access-control-allow-origin"], "*");
}

#[tokio::test]
async fn passwords_never_reach_trace_events_or_control_api() {
    let s = start_yaml(AUTH).await;
    let mut stream = s
        .client
        .get(format!("{}/__kaanal/events?token={}", s.base, s.token))
        .send()
        .await
        .unwrap();
    let ok = s.login(Some("application/json"), ALICE).await;
    let wrong = s
        .login(
            Some("application/json"),
            r#"{"username":"alice","password":"zz-wrong-zz"}"#,
        )
        .await;
    let mut seen = String::new();
    tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(chunk) = stream.chunk().await.unwrap() {
            seen.push_str(&String::from_utf8_lossy(&chunk));
            if seen.matches("\"path\":\"/login\"").count() >= 2 {
                break;
            }
        }
    })
    .await
    .unwrap();
    assert!(seen.contains("\"status\":401"), "{seen}");
    let topo = s
        .get_authed("/__kaanal/topology")
        .await
        .text()
        .await
        .unwrap();
    let chaos = s.get_authed("/__kaanal/chaos").await.text().await.unwrap();
    for text in [trace_of(&ok), trace_of(&wrong), seen, topo, chaos] {
        assert!(
            !text.contains("s3cr3t") && !text.contains("zz-wrong"),
            "{text}"
        );
    }
}

#[tokio::test]
async fn jwks_head_trace_and_route_listing() {
    let s = start_yaml(AUTH).await;
    let get = s.get("/.well-known/jwks.json").await;
    assert_eq!(get.status(), 200);
    assert_eq!(get.headers()["content-type"], "application/json");
    assert_eq!(
        trace_of(&get),
        "nginx.proc=1.0ms,net=1.0ms,login.proc=50.0ms"
    );
    let jwks: Value = get.json().await.unwrap();
    assert_eq!(jwks["keys"][0]["alg"], "RS256");
    let head = s
        .client
        .head(format!("{}/.well-known/jwks.json", s.base))
        .send()
        .await
        .unwrap();
    assert_eq!(head.status(), 200);
    assert!(head.bytes().await.unwrap().is_empty());
    let t: Value = s
        .get_authed("/__kaanal/topology")
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(
        t["routes"],
        json!(["GET /.well-known/jwks.json", "POST /login"])
    );
}

#[tokio::test]
async fn an_auth_only_config_serves_login() {
    // X11 / success criterion 7: a frontend and a login mock, nothing else.
    let s = start_yaml(
        "version: 1\nnodes:\n  login: { type: auth, proc: 1ms, users: [ { username: a, password: b } ] }\n",
    )
    .await;
    let r = s
        .login(
            Some("application/json"),
            r#"{"username":"a","password":"b"}"#,
        )
        .await;
    assert_eq!(r.status(), 200);
    assert_eq!(trace_of(&r), "login.proc=1.0ms");
    let token = r.json::<Value>().await.unwrap()["access_token"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(verify(&s, &token).await["sub"], "a");
}

#[tokio::test]
async fn get_on_login_is_405() {
    let s = start_yaml(AUTH).await;
    assert_eq!(s.get("/login").await.status(), 405);
}

/// Gateway → fixture service → db whose pool is `pool`; every db op is 100 ms so waits are visible.
fn pooled(pool: &str) -> String {
    format!(
        "version: 1\nnodes:\n  nginx: {{ type: gateway }}\n  app: {{ type: service, spec: openapi.yaml }}\n  db: {{ type: database, ops: {{ read: 300ms, write: 300ms, scan: 300ms }}, {pool} }}\nedges:\n  - {{ from: nginx, to: app }}\n  - {{ from: app, to: db }}\nendpoints:\n"
    )
}

async fn burst(
    s: &Srv,
    path: &'static str,
    n: usize,
) -> Vec<(u16, Duration, String, Option<String>)> {
    let mut tasks = Vec::new();
    for _ in 0..n {
        let (client, url) = (s.client.clone(), format!("{}{path}", s.base));
        tasks.push(tokio::spawn(async move {
            let t = std::time::Instant::now();
            let r = client.get(url).send().await.unwrap();
            let retry = r
                .headers()
                .get("retry-after")
                .map(|v| v.to_str().unwrap().to_string());
            let trace = r.headers()["x-kaanal-trace"].to_str().unwrap().to_string();
            (r.status().as_u16(), t.elapsed(), trace, retry)
        }));
    }
    let mut out = Vec::new();
    for t in tasks {
        out.push(t.await.unwrap());
    }
    out
}

#[tokio::test]
async fn a_full_pool_queues_then_overflows() {
    let s = start_yaml(&pooled("pool: { size: 1, queue: 1 }")).await;
    let mut got = burst(&s, "/users/1", 3).await;
    got.sort_by_key(|g| g.1);
    let statuses: Vec<u16> = got.iter().map(|g| g.0).collect();
    assert_eq!(
        statuses.iter().filter(|s| **s == 200).count(),
        2,
        "{statuses:?}"
    );
    assert_eq!(
        statuses.iter().filter(|s| **s == 503).count(),
        1,
        "{statuses:?}"
    );
    let ok: Vec<_> = got.iter().filter(|g| g.0 == 200).collect();
    assert!(
        ok[1].1 >= Duration::from_millis(200),
        "the queued request waited a hop: {:?}",
        ok[1].1
    );
    assert!(ok[1].2.contains("db.wait="), "{}", ok[1].2);
    assert!(
        !ok[0].2.contains(".wait="),
        "the first request did not wait: {}",
        ok[0].2
    );
}

#[tokio::test]
async fn zero_load_trace_has_no_wait_pieces() {
    let s = start_yaml(&pooled("concurrency: 4")).await;
    let r = s.get("/users/1").await;
    assert_eq!(r.status(), 200);
    assert!(!trace_of(&r).contains("wait"), "{}", trace_of(&r));
}

#[tokio::test]
async fn rate_limit_429_carries_retry_after_and_error_map_can_drop_it() {
    let yaml = |entry: &str| {
        format!(
            "version: 1\nnodes:\n  nginx: {{ type: gateway, rate_limit: {{ rps: 1, burst: 1 }} }}\n  app: {{ type: service, spec: openapi.yaml }}\nedges:\n  - {{ from: nginx, to: app }}\nendpoints:\n{entry}"
        )
    };
    let s = start_yaml(&yaml("")).await;
    assert_eq!(s.get("/users/1").await.status(), 200);
    let r = s.get("/users/1").await;
    assert_eq!(r.status(), 429);
    assert_eq!(r.headers()["retry-after"], "1");
    assert!(
        trace_of(&r).ends_with(",nginx.error=429") || trace_of(&r) == "nginx.error=429",
        "{}",
        trace_of(&r)
    );

    let s = start_yaml(&yaml(
        "  \"GET /users/{id}\": { error_map: { \"429\": 503 } }\n",
    ))
    .await;
    s.get("/users/1").await;
    let r = s.get("/users/1").await;
    assert_eq!(r.status(), 503, "error_map turned the 429 into a 503");
    assert!(
        r.headers().get("retry-after").is_none(),
        "no Retry-After unless the final status is 429"
    );
}

#[tokio::test]
async fn a_chaos_down_node_beats_the_rate_limit() {
    let s = start_yaml("version: 1\nnodes:\n  nginx: { type: gateway, rate_limit: { rps: 1, burst: 1 } }\n  app: { type: service, spec: openapi.yaml }\nedges:\n  - { from: nginx, to: app }\n").await;
    s.put_chaos("nginx", json!({ "down": true })).await;
    for _ in 0..3 {
        assert_eq!(
            s.get("/users/1").await.status(),
            503,
            "a chaos failure takes no token, so it never becomes 429"
        );
    }
}

#[tokio::test]
async fn a_dropped_client_does_not_leak_a_slot() {
    let s = start_yaml(&pooled("pool: { size: 1, queue: 5 }")).await;
    // Start requests and abandon them after 30 ms; the engine keeps them until they end.
    for _ in 0..3 {
        let c = reqwest::Client::builder()
            .timeout(Duration::from_millis(30))
            .build()
            .unwrap();
        let _ = c.get(format!("{}/users/1", s.base)).send().await;
    }
    tokio::time::sleep(Duration::from_millis(1200)).await; // 3 abandoned hops of 300 ms drain
    let r = s.get("/users/1").await;
    assert_eq!(r.status(), 200, "the slot came back");
    assert!(
        !trace_of(&r).contains(".wait="),
        "and nobody is still queued: {}",
        trace_of(&r)
    );
}

#[tokio::test]
async fn many_parallel_requests_all_complete_through_one_slot() {
    // 40 requests x 2 ms through one slot with an unbounded queue: all complete, none overflow.
    let s = start_yaml("version: 1\nnodes:\n  nginx: { type: gateway }\n  app: { type: service, spec: openapi.yaml, concurrency: 1, proc: 2ms }\nedges:\n  - { from: nginx, to: app }\n").await;
    let got = burst(&s, "/users/1", 40).await;
    assert!(got.iter().all(|g| g.0 == 200));
}

#[tokio::test]
async fn failing_branch_answers_before_the_running_sibling_ends() {
    let yaml = "version: 1\nnodes:\n  app: { type: service, spec: openapi.yaml }\n  slow: { type: external-api, proc: 400ms }\n  fast: { type: external-api, proc: 60ms }\n  bad: { type: external-api }\nedges:\n  - { from: app, to: slow }\n  - { from: app, to: fast }\n  - { from: app, to: bad }\nendpoints:\n  \"GET /users/{id}\": { chain: [ { parallel: [ slow, [ fast, bad ] ] } ] }\n";
    let s = start_yaml(yaml).await;
    s.put_chaos("bad", json!({ "down": true })).await;
    let t = std::time::Instant::now();
    let r = s.get("/users/1").await;
    assert_eq!(r.status(), 503);
    assert!(
        t.elapsed() < Duration::from_millis(350),
        "answered at the failure (~60 ms), not when `slow` ends: {:?}",
        t.elapsed()
    );
}

#[tokio::test]
async fn cors_exposes_retry_after_to_browsers() {
    let s = start_yaml("version: 1\nnodes:\n  nginx: { type: gateway, rate_limit: { rps: 1, burst: 1 } }\n  app: { type: service, spec: openapi.yaml }\nedges:\n  - { from: nginx, to: app }\n").await;
    s.get("/users/1").await;
    let r = s.get("/users/1").await;
    assert_eq!(r.status(), 429);
    let expose = r.headers()["access-control-expose-headers"]
        .to_str()
        .unwrap()
        .to_ascii_lowercase();
    assert!(
        expose.contains("retry-after") && expose.contains("x-kaanal-trace"),
        "{expose}"
    );
}

/// Sum of the `=Xms` pieces of a trace without brackets: the simulated latency in ms.
fn trace_total_ms(trace: &str) -> f64 {
    trace
        .split(',')
        .filter_map(|p| {
            p.rsplit_once('=')?
                .1
                .strip_suffix("ms")?
                .parse::<f64>()
                .ok()
        })
        .sum()
}

/// `pct` in 0..=100.
fn pct(sorted: &[f64], pct: usize) -> f64 {
    sorted[(sorted.len() - 1) * pct / 100]
}

/// Open-loop load (a request every 1/RPS s whether or not the last one answered) against a
/// pooled database at about 75% utilisation, so some requests queue. Checks that every hop
/// keeps its exact simulated duration, that the trace never claims more time than the client
/// really waited, and how far the real latency drifts above the trace (HTTP + timer lag).
///
/// `cargo test -p kaanal-server --release --test http -- --ignored --nocapture load_`
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "load test; run with --release"]
async fn load_pooled_node_keeps_latency_and_trace_accurate() {
    const RPS: u64 = 3000;
    const SECS: u64 = 4;
    // db: 8 slots (concurrency: unbounded queue) x 2 ms = 4000 requests/s; 3000/s offered = 75% busy.
    let s = start_yaml("version: 1\nnodes:\n  nginx: { type: gateway }\n  app: { type: service, spec: openapi.yaml }\n  db: { type: database, ops: { read: 2ms, write: 2ms, scan: 2ms }, concurrency: 8 }\nedges:\n  - { from: nginx, to: app }\n  - { from: app, to: db }\nendpoints:\n").await;
    let client = reqwest::Client::builder()
        .pool_max_idle_per_host(256)
        .build()
        .unwrap();
    let url = format!("{}/users/1", s.base);
    let mut tick = tokio::time::interval(Duration::from_nanos(1_000_000_000 / RPS));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Burst);
    let mut tasks = Vec::new();
    for _ in 0..RPS * SECS {
        tick.tick().await;
        let (client, url) = (client.clone(), url.clone());
        tasks.push(tokio::spawn(async move {
            let t = std::time::Instant::now();
            let r = client.get(url).send().await.unwrap();
            let status = r.status().as_u16();
            let trace = r.headers()["x-kaanal-trace"].to_str().unwrap().to_string();
            (status, t.elapsed().as_secs_f64() * 1000.0, trace)
        }));
    }
    let (mut lag, mut waits, mut waited) = (Vec::new(), Vec::new(), 0usize);
    for t in tasks {
        let (status, real, trace) = t.await.unwrap();
        assert_eq!(status, 200, "unbounded queue: nothing may fail ({trace})");
        assert!(
            trace.contains("db.read=2.0ms"),
            "every db hop keeps its exact 2 ms: {trace}"
        );
        let sim = trace_total_ms(&trace);
        assert!(
            real + 1.0 >= sim,
            "the trace claims {sim:.1} ms but the client waited {real:.1} ms: {trace}"
        );
        lag.push(real - sim);
        if let Some(w) = trace.split(',').find(|p| p.starts_with("db.wait=")) {
            waited += 1;
            waits.push(w["db.wait=".len()..w.len() - 2].parse::<f64>().unwrap());
        }
    }
    lag.sort_by(f64::total_cmp);
    waits.sort_by(f64::total_cmp);
    eprintln!(
        "{} requests, {waited} queued; real - trace ms: p50 {:.1} p99 {:.1} max {:.1}; db.wait ms: p50 {:.1} p99 {:.1}",
        lag.len(),
        pct(&lag, 50),
        pct(&lag, 99),
        lag[lag.len() - 1],
        waits.first().map_or(0.0, |_| pct(&waits, 50)),
        waits.first().map_or(0.0, |_| pct(&waits, 99)),
    );
    assert!(waited > 0, "at 75% load some requests must queue");
    assert!(
        pct(&lag, 50) < 25.0,
        "median lag above the trace: {:.1} ms",
        pct(&lag, 50)
    );
    assert!(
        pct(&lag, 99) < 150.0,
        "p99 lag above the trace: {:.1} ms",
        pct(&lag, 99)
    );
}

#[tokio::test]
async fn parallel_failure_tie_goes_to_the_first_branch_live() {
    // Both branches fail at t = 0 (default zero costs); the first branch reaches its failure
    // through a zero-duration hop. The first branch wins, as the docs promise.
    let yaml = "version: 1\nnodes:\n  app: { type: service, spec: openapi.yaml }\n  z: { type: external-api }\n  s: { type: external-api }\n  a: { type: external-api }\nedges:\n  - { from: app, to: z }\n  - { from: app, to: s }\n  - { from: app, to: a }\nendpoints:\n  \"GET /users/{id}\": { chain: [ { parallel: [ [ z, s ], [ a ] ] } ] }\n";
    let s = start_yaml(yaml).await;
    s.put_chaos("s", json!({ "down": true })).await;
    s.put_chaos("a", json!({ "error_rate": 1.0, "error_status": 500 }))
        .await;
    let r = s.get("/users/1").await;
    assert_eq!(r.status(), 503, "{}", trace_of(&r));
    assert!(trace_of(&r).ends_with("s.error=503"), "{}", trace_of(&r));
}

async fn raw_get(addr_base: &str, path: &str) -> (String, bool) {
    // returns (bytes read as lossy text, whether the read ended in an error rather than a clean EOF)
    // std sockets in spawn_blocking: tokio's `io-util` feature is not enabled in this crate (Task 0).
    use std::io::{Read, Write};
    let host = addr_base.trim_start_matches("http://").to_string();
    let path = path.to_string();
    tokio::task::spawn_blocking(move || {
        let mut s = std::net::TcpStream::connect(&host).unwrap();
        s.set_read_timeout(Some(std::time::Duration::from_secs(5)))
            .unwrap();
        s.write_all(
            format!("GET {path} HTTP/1.1\r\nHost: {host}\r\nConnection: keep-alive\r\n\r\n")
                .as_bytes(),
        )
        .unwrap();
        let mut buf = Vec::new();
        let r = s.read_to_end(&mut buf);
        (String::from_utf8_lossy(&buf).into_owned(), r.is_err())
    })
    .await
    .unwrap()
}

/// The `GW_APP_DB` topology with extra node fields appended, e.g. `yaml(", timeout: 200ms", "", "")`.
/// Tests append `endpoints:` entries to the result if they need them.
fn yaml(gw: &str, app: &str, db: &str) -> String {
    format!(
        "version: 1\nnodes:\n  nginx: {{ type: gateway, proc: 1ms{gw} }}\n  app: {{ type: service, spec: openapi.yaml, proc: 1ms{app} }}\n  db: {{ type: database, ops: {{ read: 1ms, write: 1ms, scan: 1ms }}{db} }}\nedges:\n  - {{ from: nginx, to: app, net: 1ms }}\n  - {{ from: app, to: db, net: 1ms }}\nendpoints:\n  \"GET /health\": {{ override: {{ body: {{ ok: true }} }} }}\n"
    )
}

#[tokio::test]
async fn timeout_returns_504_near_the_deadline_with_kind_in_the_body() {
    let s = start_yaml(&yaml(", timeout: 200ms", "", "")).await;
    s.put_chaos("db", json!({ "latency_ms": 2000 })).await;
    let t0 = std::time::Instant::now();
    let r = s.get("/users/42").await;
    let took = t0.elapsed();
    assert_eq!(r.status(), 504);
    assert!(
        took >= Duration::from_millis(190) && took < Duration::from_millis(900),
        "{took:?}"
    );
    let body: Value = r.json().await.unwrap();
    assert_eq!(
        body,
        json!({ "error": "chaos", "node": "nginx", "kind": "timeout" })
    );
}

#[tokio::test]
async fn status_failure_body_is_unchanged() {
    // existing contract: no "kind" field for kind status
    let s = start().await;
    s.put_chaos("db", json!({ "error_rate": 1.0, "error_status": 503 }))
        .await;
    let b: Value = s.get("/users/42").await.json().await.unwrap();
    assert_eq!(b, json!({ "error": "chaos", "node": "db" }));
}

#[tokio::test]
async fn hang_with_a_short_node_timeout_answers_504() {
    let s = start_yaml(&yaml("", "", ", timeout: 300ms")).await;
    s.put_chaos("db", json!({ "error_rate": 1.0, "error_kind": "hang" }))
        .await;
    let t0 = std::time::Instant::now();
    let r = s.get("/users/42").await;
    assert_eq!(r.status(), 504);
    assert!(t0.elapsed() >= Duration::from_millis(290));
}

#[tokio::test]
async fn deep_reset_and_malformed_become_502() {
    let s = start().await;
    for k in ["reset", "malformed"] {
        s.put_chaos("db", json!({ "error_rate": 1.0, "error_kind": k }))
            .await;
        let r = s.get("/users/42").await;
        assert_eq!(r.status(), 502, "{k}");
        assert_eq!(r.json::<Value>().await.unwrap()["kind"], k);
    }
}

#[tokio::test]
async fn reset_on_the_owner_behind_a_gateway_aborts_the_connection() {
    // Z7: the gateway is transparent, the owner is a wire node
    let s = start().await;
    s.put_chaos("app", json!({ "error_rate": 1.0, "error_kind": "reset" }))
        .await;
    let (text, _) = raw_get(&s.base, "/users/42").await;
    assert!(
        text.starts_with("HTTP/1.1 200"),
        "Task 0: the head must arrive before the abort: {text:?}"
    );
    let declared: usize = text
        .lines()
        .find_map(|l| l.strip_prefix("content-length: "))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let body = text.split("\r\n\r\n").nth(1).unwrap_or("");
    assert!(
        !body.is_empty() && body.len() < declared,
        "partial body, then EOF: {text:?}"
    );
    let r = s
        .client
        .get(format!("{}/users/42", s.base))
        .send()
        .await
        .expect("the head arrives");
    assert_eq!(r.status(), 200);
    assert!(r.bytes().await.is_err(), "the body read must abort");
}

#[tokio::test]
async fn malformed_on_a_wire_node_is_valid_http_with_invalid_json() {
    let s = start().await;
    s.put_chaos(
        "app",
        json!({ "error_rate": 1.0, "error_kind": "malformed" }),
    )
    .await;
    let r = s.get("/users/42").await;
    assert_eq!(r.status(), 200);
    let declared = r.content_length();
    let text = r.text().await.unwrap();
    assert!(serde_json::from_str::<Value>(&text).is_err(), "{text}");
    assert_eq!(
        declared,
        Some(text.len() as u64),
        "Content-Length matches the cut body"
    );
    assert_ne!(text, "");
}

#[tokio::test]
async fn malformed_on_head_answers_the_plain_response() {
    let s = start().await;
    let clean = s.get("/users/42").await.content_length();
    s.put_chaos(
        "app",
        json!({ "error_rate": 1.0, "error_kind": "malformed" }),
    )
    .await;
    let r = s
        .client
        .head(format!("{}/users/42", s.base))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200, "nothing to cut");
    let declared: Option<u64> = r.headers()["content-length"].to_str().unwrap().parse().ok();
    assert_eq!(
        declared, clean,
        "HEAD declares the clean GET length, not a cut one"
    );
}

#[tokio::test]
async fn error_map_kind_key_overrides_the_wire_behaviour() {
    let src = format!(
        "{}  \"GET /users/{{id}}\": {{ error_map: {{ reset: {{ status: 418 }} }} }}\n",
        yaml("", "", "")
    );
    let s = start_yaml(&src).await;
    s.put_chaos("app", json!({ "error_rate": 1.0, "error_kind": "reset" }))
        .await;
    assert_eq!(s.get("/users/42").await.status(), 418);
}

#[tokio::test]
async fn class_key_does_not_swallow_a_wire_reset() {
    let src = format!(
        "{}  \"GET /users/{{id}}\": {{ error_map: {{ \"5xx\": {{ status: 599 }} }} }}\n",
        yaml("", "", "")
    );
    let s = start_yaml(&src).await;
    s.put_chaos("app", json!({ "error_rate": 1.0, "error_kind": "reset" }))
        .await;
    let r = s.client.get(format!("{}/users/42", s.base)).send().await;
    assert!(
        r.is_err() || r.unwrap().status() != 599,
        "the 5xx key must not turn a reset into JSON"
    );
}

#[tokio::test]
async fn endpoint_and_node_at_chaos_through_the_control_api() {
    let s = start().await;
    let put = |body: Value| {
        s.auth(
            s.client
                .put(format!("{}/__kaanal/chaos", s.base))
                .json(&body),
        )
        .send()
    };
    let r = put(json!({ "endpoint": "GET /users/{id}", "error_rate": 1.0, "error_status": 502 }))
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    assert_eq!(s.get("/users/42").await.status(), 502);
    assert_eq!(
        s.get("/health").await.status(),
        200,
        "other endpoints untouched"
    );
    s.put_chaos("db", json!({ "latency_ms": 5 })).await;
    // exact-key delete
    let d = s
        .auth(s.client.delete(format!(
            "{}/__kaanal/chaos?endpoint=GET%20/users/%7Bid%7D",
            s.base
        )))
        .send()
        .await
        .unwrap();
    assert_eq!(d.status(), 204);
    assert_eq!(s.get("/users/42").await.status(), 200);
    let all: Value = s.get_authed("/__kaanal/chaos").await.json().await.unwrap();
    assert_eq!(
        all["nodes"]["db"]["latency_ms"], 5,
        "the exact-key delete leaves node chaos alone"
    );
    assert!(all["endpoints"].as_object().unwrap().is_empty());
    // a typo in the query is a 400 and wipes nothing
    let typo = s
        .auth(
            s.client
                .delete(format!("{}/__kaanal/chaos?endpoin=GET%20/x", s.base)),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(typo.status(), 400);
    assert!(typo.json::<Value>().await.unwrap()["error"].is_string());
    let all: Value = s.get_authed("/__kaanal/chaos").await.json().await.unwrap();
    assert_eq!(all["nodes"]["db"]["latency_ms"], 5);
    // node@endpoint
    let r = put(json!({ "node": "db", "endpoint": "get /users/{id}", "error_rate": 1.0 }))
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let all: Value = s.get_authed("/__kaanal/chaos").await.json().await.unwrap();
    assert_eq!(all["node_at"][0]["node"], "db");
    assert_eq!(all["node_at"][0]["endpoint"], "GET /users/{id}");
    assert_eq!(s.get("/users/42").await.status(), 503);
    let bad = put(json!({ "node": "db", "error_kind": "nope" }))
        .await
        .unwrap();
    assert_eq!(bad.status(), 400);
    let bad = put(json!({ "node": "db", "error_kind": "reset", "error_status": 200 }))
        .await
        .unwrap();
    assert_eq!(bad.status(), 400, "status is range-checked for every kind");
    let bad = put(json!({ "endpoint": "GET /nope", "down": true }))
        .await
        .unwrap();
    assert_eq!(bad.status(), 404);
    let none = put(json!({ "down": true })).await.unwrap();
    assert_eq!(none.status(), 400, "needs node or endpoint");
}

#[tokio::test]
async fn get_chaos_is_nested() {
    let s = start().await;
    s.put_chaos("db", json!({ "latency_ms": 5 })).await;
    let v: Value = s
        .auth(s.client.get(format!("{}/__kaanal/chaos", s.base)))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(v["nodes"]["db"]["latency_ms"], 5);
    assert_eq!(v["nodes"]["db"]["error_kind"], "status");
    assert!(v["endpoints"].is_object() && v["node_at"].is_array());
}

#[tokio::test]
async fn trace_names_reset_and_malformed_and_no_body_routes_get_the_plain_response() {
    let src = format!(
        "{}  \"GET /nocontent\": {{ override: {{ status: 204 }} }}\n  \"GET /five\": {{ override: {{ body: 5 }} }}\n",
        yaml("", "", "")
    );
    let s = start_yaml(&src).await;
    s.put_chaos(
        "app",
        json!({ "error_rate": 1.0, "error_kind": "malformed" }),
    )
    .await;
    let r = s.get("/users/42").await;
    assert!(
        trace_of(&r).ends_with("app.error=malformed"),
        "{}",
        trace_of(&r)
    );
    s.put_chaos("app", json!({ "error_kind": "reset" })).await;
    let (text, _) = raw_get(&s.base, "/users/42").await;
    assert!(text.to_lowercase().contains("app.error=reset"), "{text}");
    // a 204 has no body to break, even with reset at the gateway
    s.put_chaos("nginx", json!({ "error_rate": 1.0, "error_kind": "reset" }))
        .await;
    let r = s.get("/nocontent").await;
    assert_eq!(r.status(), 204);
    assert!(
        trace_of(&r).ends_with("nginx.error=reset"),
        "{}",
        trace_of(&r)
    );
    // a one-byte body: the declared length must still exceed what was sent
    let (text, _) = raw_get(&s.base, "/five").await;
    let declared: usize = text
        .lines()
        .find_map(|l| l.strip_prefix("content-length: "))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let body = text.split("\r\n\r\n").nth(1).unwrap_or("");
    assert!(!body.is_empty() && body.len() < declared, "{text:?}");
}

#[tokio::test]
async fn error_map_keyed_by_node_name_overrides_a_wire_reset() {
    let src = format!(
        "{}  \"GET /users/{{id}}\": {{ error_map: {{ app: {{ status: 418 }} }} }}\n",
        yaml("", "", "")
    );
    let s = start_yaml(&src).await;
    s.put_chaos("app", json!({ "error_rate": 1.0, "error_kind": "reset" }))
        .await;
    assert_eq!(s.get("/users/42").await.status(), 418);
}

#[tokio::test]
async fn endpoint_level_reset_belongs_to_the_gateway_and_aborts() {
    let s = start().await;
    let put = s
        .auth(s.client.put(format!("{}/__kaanal/chaos", s.base)))
        .json(&json!({ "endpoint": "GET /users/{id}", "error_rate": 1.0, "error_kind": "reset" }));
    assert_eq!(put.send().await.unwrap().status(), 200);
    let r = s
        .client
        .get(format!("{}/users/42", s.base))
        .send()
        .await
        .expect("the head arrives");
    assert!(
        r.headers()["x-kaanal-trace"]
            .to_str()
            .unwrap()
            .contains("nginx.error=reset")
    );
    assert!(r.bytes().await.is_err(), "the body read must abort");
}

#[tokio::test]
async fn a_gateway_timeout_goes_through_error_map_like_any_failure() {
    // Decided (B2a Z7 follow-up): a timeout at a wire node is not a wire break; `504` and `5xx` keys
    // remap it (nginx `error_page 504`), and without a key it stays a plain 504 with the kind in the body.
    let src = format!(
        "{}  \"GET /users/{{id}}\": {{ error_map: {{ \"504\": {{ status: 502, body: {{ mapped: true }} }} }} }}\n",
        yaml(", timeout: 200ms", "", "")
    );
    let s = start_yaml(&src).await;
    s.put_chaos("db", json!({ "latency_ms": 2000 })).await;
    let r = s.get("/users/42").await;
    assert_eq!(r.status(), 502);
    assert_eq!(r.json::<Value>().await.unwrap(), json!({ "mapped": true }));
}

#[tokio::test]
async fn malformed_on_an_auth_responder_cuts_the_real_reply() {
    let s = start_yaml(AUTH).await;
    s.put_chaos(
        "login",
        json!({ "error_rate": 1.0, "error_kind": "malformed" }),
    )
    .await;
    let r = s.get("/.well-known/jwks.json").await;
    assert_eq!(r.status(), 200);
    let text = r.text().await.unwrap();
    assert!(text.starts_with("{\"keys\""), "{text}");
    assert!(serde_json::from_str::<Value>(&text).is_err());
}
