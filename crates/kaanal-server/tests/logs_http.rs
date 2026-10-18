//! Request ids and the event bus: every mock response carries `X-Request-Id`, every request
//! leaves one event. (`/logs` and `/trace` read the ring.)

use kaanal_core::Config;
use kaanal_server::ServeOpts;
use reqwest::Method;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::Duration;

const YAML: &str = "version: 1
seed: 42
nodes:
  nginx: { type: gateway, proc: 1ms }
  app: { type: service, spec: openapi.yaml, proc: 1ms }
  db: { type: database, ops: { read: 1ms, write: 1ms, scan: 1ms } }
  lim: { type: service, proc: 1ms, rate_limit: { rps: 1, burst: 1 } }
  pa: { type: external-api, proc: 2ms }
  pb: { type: external-api, proc: 3ms }
edges:
  - { from: nginx, to: app, net: 1ms }
  - { from: app, to: db, net: 1ms }
  - { from: app, to: lim, net: 1ms }
  - { from: app, to: pa, net: 1ms }
  - { from: app, to: pb, net: 1ms }
endpoints:
  \"GET /users/{id}\": { chain: [ db ] }
  \"POST /orders\": { chain: [ lim ] }
  \"GET /users\": { chain: [ { parallel: [ pa, pb ] } ] }
";

struct Srv {
    base: String,
    token: String,
    client: reqwest::Client,
}

async fn start() -> Srv {
    let dir: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let token = kaanal_server::new_token();
    let opts = ServeOpts {
        host: "127.0.0.1".parse().unwrap(),
        port: 0,
        token: token.clone(),
        auth_key: None,
    };
    let s = kaanal_server::serve(&Config::from_yaml(YAML).unwrap(), &dir, opts)
        .await
        .unwrap();
    Srv {
        base: format!("http://127.0.0.1:{}", s.addr.port()),
        token,
        client: reqwest::Client::new(),
    }
}

impl Srv {
    async fn get(&self, path: &str) -> reqwest::Response {
        self.client
            .get(format!("{}{path}", self.base))
            .send()
            .await
            .unwrap()
    }

    async fn ctl(&self, m: Method, path: &str, body: Option<Value>) -> reqwest::Response {
        let mut rb = self
            .client
            .request(m, format!("{}{path}", self.base))
            .bearer_auth(&self.token);
        if let Some(b) = body {
            rb = rb.json(&b);
        }
        rb.send().await.unwrap()
    }

    async fn reset(&self) {
        assert_eq!(
            self.ctl(Method::POST, "/__kaanal/reset", None)
                .await
                .status(),
            204
        );
    }

    async fn chaos(&self, node: &str, body: Value) {
        let r = self
            .ctl(Method::PUT, &format!("/__kaanal/chaos/{node}"), Some(body))
            .await;
        assert_eq!(r.status(), 200);
    }

    /// Opens the event stream; subscribe before the traffic you want to see.
    async fn events(&self) -> Events {
        let r = self
            .client
            .get(format!(
                "{}/__kaanal/events?token={}",
                self.base, self.token
            ))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 200);
        Events {
            resp: r,
            seen: String::new(),
        }
    }
}

struct Events {
    resp: reqwest::Response,
    seen: String,
}

impl Events {
    /// The next `n` `request` events (scenario events are skipped).
    async fn requests(&mut self, n: usize) -> Vec<Value> {
        let got = |seen: &str| -> Vec<Value> {
            seen.split("\n\n")
                .filter(|b| b.contains("event: request"))
                .filter_map(|b| b.lines().find_map(|l| l.strip_prefix("data:")))
                .filter_map(|d| serde_json::from_str(d.trim()).ok())
                .collect()
        };
        tokio::time::timeout(Duration::from_secs(5), async {
            while got(&self.seen).len() < n {
                let chunk = self.resp.chunk().await.unwrap().expect("stream ended");
                self.seen.push_str(&String::from_utf8_lossy(&chunk));
            }
        })
        .await
        .unwrap_or_else(|_| panic!("wanted {n} events, stream so far: {}", self.seen));
        got(&self.seen)
    }
}

fn id_of(r: &reqwest::Response) -> String {
    r.headers()["x-request-id"].to_str().unwrap().to_string()
}

/// `^r[0-9a-f]{4}-[0-9]{4,}$`
fn is_id(s: &str) -> bool {
    let ok = |c: char| c.is_ascii_digit() || ('a'..='f').contains(&c);
    s.strip_prefix('r')
        .and_then(|r| r.split_once('-'))
        .is_some_and(|(b, n)| {
            b.len() == 4
                && b.chars().all(ok)
                && n.len() >= 4
                && n.chars().all(|c| c.is_ascii_digit())
        })
}

fn number(id: &str) -> u64 {
    id.split_once('-').unwrap().1.parse().unwrap()
}

#[tokio::test]
async fn every_mock_response_has_a_request_id() {
    let s = start().await;
    let mut got = Vec::new();
    let mut note = |what: &str, r: &reqwest::Response| got.push((what.to_string(), id_of(r)));

    let r = s.get("/users/1").await;
    assert_eq!(r.status(), 200);
    note("200", &r);
    let r = s.get("/users").await;
    assert_eq!(r.status(), 200, "the parallel route");
    note("parallel", &r);
    assert!(
        r.headers()["access-control-expose-headers"]
            .to_str()
            .unwrap()
            .contains("x-request-id")
    );

    s.chaos("db", json!({ "error_rate": 1.0, "error_status": 503 }))
        .await;
    let r = s.get("/users/1").await;
    assert_eq!(r.status(), 503);
    note("chaos 503", &r);
    s.reset().await;

    let pin = json!({ "endpoint": "GET /users/{id}", "state": "error" });
    s.ctl(Method::PUT, "/__kaanal/pins", Some(pin)).await;
    let r = s.get("/users/1").await;
    assert_eq!(r.status(), 500);
    note("pinned", &r);
    s.reset().await;

    s.chaos("app", json!({ "error_rate": 1.0, "error_kind": "reset" }))
        .await;
    let r = s.get("/users/1").await;
    note("wire reset", &r);
    s.reset().await;

    let r = s.get("/nope").await;
    assert_eq!(r.status(), 404);
    note("404", &r);
    let r = s
        .client
        .delete(format!("{}/users/1", s.base))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 405);
    note("405", &r);
    let r = s
        .client
        .get(format!("{}/users/1", s.base))
        .header("x-kaanal-scenario", "no-such-scenario")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 400);
    note("bad scope", &r);

    let post = || s.client.post(format!("{}/orders", s.base)).send();
    assert_eq!(post().await.unwrap().status(), 201);
    let r = post().await.unwrap();
    assert_eq!(r.status(), 429);
    note("429", &r);

    for (what, id) in &got {
        assert!(is_id(id), "{what}: {id}");
    }
    let mut nums: Vec<_> = got.iter().map(|g| number(&g.1)).collect();
    let n = nums.len();
    nums.dedup();
    assert_eq!(nums.len(), n, "ids are distinct");
}

#[tokio::test]
async fn control_responses_and_preflights_have_no_request_id() {
    let s = start().await;
    let r = s.ctl(Method::GET, "/__kaanal/health", None).await;
    assert!(r.headers().get("x-request-id").is_none());
    let r = s.ctl(Method::GET, "/__kaanal/no-such-route", None).await;
    assert_eq!(r.status(), 404);
    assert!(r.headers().get("x-request-id").is_none());
    let r = s
        .client
        .request(Method::OPTIONS, format!("{}/users/1", s.base))
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 204);
    assert!(r.headers().get("x-request-id").is_none());
    // The preflight took no number: the first real request is the first id.
    assert_eq!(number(&id_of(&s.get("/users/1").await)), 1);
}

#[tokio::test]
async fn ids_increase_and_survive_reset() {
    let s = start().await;
    let a = id_of(&s.get("/users/1").await);
    let b = id_of(&s.get("/users/1").await);
    s.reset().await;
    let c = id_of(&s.get("/users/1").await);
    assert_eq!(a.split('-').next(), c.split('-').next(), "one boot nonce");
    assert_eq!((number(&a), number(&b), number(&c)), (1, 2, 3));
}

#[tokio::test]
async fn each_request_makes_exactly_one_event() {
    let s = start().await;
    let mut ev = s.events().await;
    let mut ids = Vec::new();
    for path in ["/users/1", "/users", "/nope", "/users/2", "/users/3"] {
        ids.push(id_of(&s.get(path).await));
    }
    let r = s
        .client
        .get(format!("{}/users/1", s.base))
        .header("x-kaanal-scenario", "nope")
        .send()
        .await
        .unwrap();
    ids.push(id_of(&r));
    // A preflight and a control call make none.
    s.client
        .request(Method::OPTIONS, format!("{}/users/1", s.base))
        .send()
        .await
        .unwrap();
    s.ctl(Method::GET, "/__kaanal/health", None).await;
    ids.push(id_of(&s.get("/users/4").await));

    let got = ev.requests(7).await;
    let seen: Vec<_> = got.iter().map(|e| e["id"].as_str().unwrap()).collect();
    assert_eq!(seen, ids, "events and responses agree, one each, in order");
    assert_eq!(got.len(), 7, "no extra events");
}

#[tokio::test]
async fn client_request_id_is_recorded_and_a_bad_one_ignored() {
    let s = start().await;
    let mut ev = s.events().await;
    let send = |v: String| {
        s.client
            .get(format!("{}/users/1", s.base))
            .header("x-request-id", v)
            .send()
    };
    send("web-7f.a_b-1".into()).await.unwrap();
    send("a".repeat(65)).await.unwrap();
    send("has space".into()).await.unwrap();
    send("a".repeat(64)).await.unwrap();
    // A header with a control byte cannot be built by reqwest; raw bytes are not valid UTF-8 text.
    let r = s
        .client
        .get(format!("{}/users/1", s.base))
        .header(
            "x-request-id",
            reqwest::header::HeaderValue::from_bytes(b"caf\xe9").unwrap(),
        )
        .send()
        .await
        .unwrap();
    assert!(is_id(&id_of(&r)), "our id is still ours");
    let got = ev.requests(5).await;
    let client: Vec<_> = got.iter().map(|e| e["client_id"].as_str()).collect();
    let long = "a".repeat(64);
    assert_eq!(
        client,
        [Some("web-7f.a_b-1"), None, None, Some(long.as_str()), None]
    );
}

#[tokio::test]
async fn sse_event_carries_id_and_seq() {
    let s = start().await;
    let mut ev = s.events().await;
    let id = id_of(&s.get("/users/1").await);
    let nope = id_of(&s.get("/nope").await);
    let got = ev.requests(2).await;
    assert_eq!(got[0]["id"], id.as_str());
    assert_eq!(got[0]["seq"], number(&id));
    assert!(got[0]["rejected"].is_null());
    assert_eq!(got[0]["client_id"], Value::Null);
    assert_eq!(got[1]["id"], nope.as_str());
    assert_eq!(got[1]["rejected"], "unmatched");
    assert_eq!(got[1]["status"], 404);
}

#[tokio::test]
async fn bad_scope_is_observed_but_still_not_counted() {
    // Amends R1.41 T4: a bad scope now leaves an event, but it still takes no counter value or
    // seeded draw (the same seeded sequence is checked in `scoping_http.rs`).
    let s = start().await;
    let mut ev = s.events().await;
    s.chaos("db", json!({ "error_rate": 0.5 })).await;
    let mut plain = Vec::new();
    for _ in 0..12 {
        plain.push(s.get("/users/1").await.status().as_u16());
    }
    s.reset().await;
    s.chaos("db", json!({ "error_rate": 0.5 })).await;
    let mut mixed = Vec::new();
    for _ in 0..12 {
        let r = s
            .client
            .get(format!("{}/users/1", s.base))
            .header("x-kaanal-scenario", "nope")
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 400);
        mixed.push(s.get("/users/1").await.status().as_u16());
    }
    assert_eq!(plain, mixed);
    let got = ev.requests(36).await;
    let bad = got.iter().filter(|e| e["rejected"] == "bad-scope").count();
    assert_eq!(bad, 12);
}

// ---- the log routes ----

impl Srv {
    async fn logs(&self, q: &str) -> Value {
        let r = self
            .ctl(Method::GET, &format!("/__kaanal/logs{q}"), None)
            .await;
        assert_eq!(r.status(), 200, "{q}");
        r.json().await.unwrap()
    }

    /// `n` mock requests, up to 32 at a time (arrival order is not completion order).
    async fn burst(&self, n: usize) {
        let mut set = tokio::task::JoinSet::new();
        for _ in 0..n {
            if set.len() >= 32 {
                set.join_next().await.unwrap().unwrap();
            }
            let (c, url) = (self.client.clone(), format!("{}/users/1", self.base));
            set.spawn(async move { c.get(url).send().await.unwrap() });
        }
        while set.join_next().await.is_some() {}
    }

    async fn trace(&self, key: &str) -> reqwest::Response {
        self.ctl(Method::GET, &format!("/__kaanal/trace/{key}"), None)
            .await
    }
}

fn ids(page: &Value) -> Vec<String> {
    page["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["id"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn logs_default_limit_and_order() {
    let s = start().await;
    for _ in 0..150 {
        s.get("/users/1").await;
    }
    let p = s.logs("").await;
    let dones: Vec<u64> = p["entries"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["done"].as_u64().unwrap())
        .collect();
    assert_eq!(
        dones,
        (51..=150).collect::<Vec<_>>(),
        "the last 100, oldest first"
    );
    assert!(p["oldest"].as_str().unwrap().ends_with("0001"));
    assert!(p["newest"].as_str().unwrap().ends_with("0150"));
    assert_eq!(p["cap"], 1000);
    assert_eq!(
        s.logs("?limit=5").await["entries"]
            .as_array()
            .unwrap()
            .len(),
        5
    );
}

#[tokio::test]
async fn logs_filters() {
    let s = start().await;
    s.get("/users/1").await;
    s.get("/nope").await;
    s.chaos("db", json!({ "error_rate": 1.0, "error_status": 503 }))
        .await;
    s.get("/users/2").await;
    s.ctl(Method::DELETE, "/__kaanal/chaos/db", None).await;
    let r = s
        .client
        .get(format!("{}/users/3", s.base))
        .header("x-kaanal-scenario", "")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let n = |p: &Value| p["entries"].as_array().unwrap().len();
    assert_eq!(n(&s.logs("?endpoint=GET%20/users/%7Bid%7D").await), 3);
    assert_eq!(
        n(&s.logs("?endpoint=get%20/users/%7Bid%7D").await),
        3,
        "method case normalised"
    );
    assert_eq!(n(&s.logs("?status=404").await), 1);
    assert_eq!(n(&s.logs("?status=5xx").await), 1);
    assert_eq!(n(&s.logs("?status=404,5xx").await), 2);
    assert_eq!(n(&s.logs("?failed=true").await), 1);
    assert_eq!(
        n(&s.logs("?scope=").await),
        1,
        "the empty-header scope, exact"
    );
    assert_eq!(n(&s.logs("?scope=nope").await), 0);
    assert_eq!(n(&s.logs("?since=1h").await), 4);
    assert_eq!(
        n(&s.logs("?since=2999-01-01T00:00:00Z").await),
        0,
        "a future time"
    );
    assert_eq!(n(&s.logs("?since=2000-01-01T00:00:00Z").await), 4);
    let all = s.logs("").await;
    let second = all["entries"][1]["done"].as_u64().unwrap();
    assert_eq!(n(&s.logs(&format!("?after={second}")).await), 2);
    tokio::time::sleep(Duration::from_millis(60)).await;
    s.get("/users/4").await;
    assert_eq!(
        n(&s.logs("?since=40ms").await),
        1,
        "older entries are excluded"
    );
}

#[tokio::test]
async fn logs_bad_input_is_400_and_names_the_parameter() {
    let s = start().await;
    for (q, name) in [
        ("?bogus=1", "bogus"),
        ("?limit=0", "limit"),
        ("?limit=1001", "limit"),
        ("?limit=abc", "limit"),
        ("?limit=", "limit"),
        ("?limit=%2B5", "limit"),
        ("?limit=%205", "limit"),
        ("?limit=99999999999999999999999", "limit"),
        ("?limit=1&limit=2", "limit"),
        ("?status=abc", "status"),
        ("?since=yesterday", "since"),
        ("?failed=maybe", "failed"),
        ("?endpoint=", "endpoint"),
    ] {
        let r = s
            .ctl(Method::GET, &format!("/__kaanal/logs{q}"), None)
            .await;
        assert_eq!(r.status(), 400, "{q}");
        let m = r.json::<Value>().await.unwrap()["error"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(m.contains(name), "{q}: {m}");
    }
    let p = s.logs("?limit=1000").await;
    assert_eq!(p["cap"], 1000);
    assert_eq!(p["entries"], json!([]), "min(1000, an empty ring)");
}

#[tokio::test]
async fn ring_stays_at_1000_and_reports_evicted() {
    let s = start().await;
    let first = id_of(&s.get("/users/1").await);
    s.burst(1199).await;
    let p = s.logs("?limit=1000").await;
    assert_eq!(
        p["entries"].as_array().unwrap().len(),
        1000,
        "min(1000, ring)"
    );
    assert_eq!(p["evicted"], 200);
    // Concurrent arrival: which ids are oldest or newest is not fixed, but the first request
    // finished among the first 200 and so is gone.
    let r = s.trace(&first).await;
    assert_eq!(r.status(), 404);
    assert_eq!(
        r.json::<Value>().await.unwrap(),
        json!({ "error": "evicted", "detail": "older than the last 1000 requests, or still in flight" })
    );
}

#[tokio::test]
async fn trace_by_id_last_and_client_id() {
    let s = start().await;
    let resp = s
        .client
        .get(format!("{}/users/1", s.base))
        .header("x-request-id", "web-7")
        .send()
        .await
        .unwrap();
    let first = id_of(&resp);
    let second = id_of(&s.get("/users/2").await);
    let got: Value = s.trace(&first).await.json().await.unwrap();
    assert_eq!(
        (got["id"].as_str().unwrap(), got["path"].as_str().unwrap()),
        (first.as_str(), "/users/1")
    );
    assert!(got["hops"].is_array() && got["trace"].is_string());
    let got: Value = s.trace("last").await.json().await.unwrap();
    assert_eq!(got["id"], second.as_str());
    let got: Value = s.trace("web-7").await.json().await.unwrap();
    assert_eq!(got["id"], first.as_str());
    let got: Value = s.trace("web%2D7").await.json().await.unwrap();
    assert_eq!(got["id"], first.as_str(), "percent-decoded");
}

#[tokio::test]
async fn trace_evicted_vs_unknown_vs_malformed() {
    let s = start().await;
    let boot = id_of(&s.get("/users/1").await)[..5].to_string();
    for k in [
        format!("{boot}-9999"),
        "rffff-0001".into(),
        "no-such-client".into(),
    ] {
        let r = s.trace(&k).await;
        assert_eq!(r.status(), 404, "{k}");
        assert_eq!(
            r.json::<Value>().await.unwrap(),
            json!({ "error": "unknown" })
        );
    }
    assert_eq!(s.trace("has%20space").await.status(), 400);
    let r = s.trace("%FF").await;
    assert_eq!(r.status(), 400, "not UTF-8");
    assert!(r.json::<Value>().await.unwrap()["error"].is_string());
    s.reset().await;
    assert_eq!(s.trace("last").await.status(), 404, "empty ring");
}

#[tokio::test]
async fn trace_of_an_out_of_order_slow_request_found_after_it_finishes() {
    let s = start().await;
    s.chaos("pa", json!({ "latency_ms": 400 })).await;
    let slow = tokio::spawn({
        let (c, url) = (s.client.clone(), format!("{}/users", s.base));
        async move { c.get(url).send().await.unwrap() }
    });
    // Fast requests until one has an id past the first two numbers: by then the slow request
    // has surely arrived and (400ms of chaos) is still in flight. Bounded, no fixed sleep.
    let fast = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let id = id_of(&s.get("/users/1").await);
            if number(&id) >= 3 {
                return id;
            }
        }
    })
    .await
    .unwrap();
    let e: Value = s.trace("last").await.json().await.unwrap();
    assert_eq!(e["id"], fast.as_str(), "the slow one is still in flight");
    let slow_id = id_of(&slow.await.unwrap());
    assert!(number(&slow_id) < number(&fast), "started first");
    let e: Value = s.trace(&slow_id).await.json().await.unwrap();
    assert_eq!(e["id"], slow_id.as_str());
    let last: Value = s.trace("last").await.json().await.unwrap();
    assert_eq!(last["id"], slow_id.as_str(), "last means last to finish");
}

#[tokio::test]
async fn routes_need_the_token() {
    let s = start().await;
    for p in ["/__kaanal/logs", "/__kaanal/trace/last"] {
        // `s.get` sends no token; `s.ctl` (and the explicit bearer below) does.
        assert_eq!(s.get(p).await.status(), 401, "{p}");
        let r = s
            .client
            .get(format!("{}{p}", s.base))
            .bearer_auth(&s.token)
            .header("host", "evil.example")
            .send()
            .await
            .unwrap();
        assert_eq!(r.status(), 403, "{p}");
    }
}

#[tokio::test]
async fn logs_entries_have_no_bodies_or_query() {
    let s = start().await;
    s.client
        .get(format!("{}/users/1?token=secret-query-789", s.base))
        .header("authorization", "Bearer secret-header-456")
        .header("x-custom", "secret-header-456")
        .header("x-request-id", "client-7")
        .body("{\"password\":\"secret-body-123\"}")
        .send()
        .await
        .unwrap();
    let seen = s.logs("").await.to_string() + &s.trace("last").await.text().await.unwrap();
    for secret in ["secret-body-123", "secret-header-456", "secret-query-789"] {
        assert!(!seen.contains(secret), "{secret} leaked: {seen}");
    }
    assert!(seen.contains("client-7") && seen.contains("\"path\":\"/users/1\""));
}

#[tokio::test]
async fn reset_empties_logs_but_ids_continue() {
    let s = start().await;
    let before = id_of(&s.get("/users/1").await);
    s.reset().await;
    let page = s.logs("").await;
    assert_eq!(page["entries"], json!([]));
    assert_eq!(
        (&page["oldest"], &page["newest"]),
        (&Value::Null, &Value::Null)
    );
    let after = id_of(&s.get("/users/1").await);
    assert_eq!(number(&after), number(&before) + 1);
    assert_eq!(ids(&s.logs("").await), [after]);
    let resp = s.trace(&before).await;
    assert_eq!(resp.status(), 404);
    assert_eq!(resp.json::<Value>().await.unwrap()["error"], "evicted");
}

/// A real socket closed mid-request: hyper drops the handler, and the log shows exactly one
/// `client-gone` 499 entry with its endpoint and client id, and one event.
#[tokio::test]
async fn a_closed_socket_leaves_exactly_one_client_gone_entry() {
    use std::io::Write as _;
    let s = start().await;
    // a hang holds the request for a minute, far longer than this test
    s.chaos(
        "db",
        json!({ "error_rate": 1.0, "error_kind": "hang", "latency_ms": 30 }),
    )
    .await;
    let mut ev = s.events().await;
    let addr = s.base.trim_start_matches("http://").to_string();
    let mut sock = std::net::TcpStream::connect(&addr).unwrap();
    let head = format!("GET /users/1 HTTP/1.1\r\nHost: {addr}\r\nX-Request-Id: tcp-gone\r\n\r\n");
    sock.write_all(head.as_bytes()).unwrap();
    // the request is in flight (hung), then the client goes away without reading
    tokio::time::sleep(Duration::from_millis(300)).await;
    drop(sock);
    // poll the log for the entry, bounded
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let entry = loop {
        let page: Value = s
            .ctl(Method::GET, "/__kaanal/logs?status=499", None)
            .await
            .json()
            .await
            .unwrap();
        if let Some(e) = page["entries"].as_array().and_then(|a| a.first()) {
            break e.clone();
        }
        assert!(
            std::time::Instant::now() < deadline,
            "no client-gone entry: the handler was not dropped when the socket closed"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    assert_eq!(entry["rejected"], "client-gone");
    assert_eq!(entry["client_id"], "tcp-gone");
    assert_eq!(entry["endpoint"], "GET /users/{id}");
    let events = ev.requests(1).await;
    assert_eq!(events[0]["rejected"], "client-gone");
    // still exactly one entry for that request, and no second event
    tokio::time::sleep(Duration::from_millis(500)).await;
    let page: Value = s
        .ctl(Method::GET, "/__kaanal/logs", None)
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(page["entries"].as_array().unwrap().len(), 1, "{page}");
}
