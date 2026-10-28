//! `POST /__kaanal/shutdown`: the drain, the budget, and who may ask.

use kaanal_core::Config;
use kaanal_server::{BoundServer, ServeOpts};
use reqwest::Method;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const YAML: &str = "version: 1
seed: 42
nodes:
  nginx: { type: gateway, proc: 1ms }
  app: { type: service, spec: openapi.yaml, proc: 1ms }
  db: { type: database, ops: { read: 1ms, write: 1ms, scan: 1ms } }
edges:
  - { from: nginx, to: app, net: 1ms }
  - { from: app, to: db, net: 1ms }
endpoints:
  \"GET /users/{id}\": { chain: [ db ] }
";

struct Srv {
    base: String,
    token: String,
    client: reqwest::Client,
}

async fn start() -> (Srv, BoundServer) {
    let dir: PathBuf = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let token = kaanal_server::new_token();
    let opts = ServeOpts {
        host: "127.0.0.1".parse().unwrap(),
        port: 0,
        token: token.clone(),
        auth_key: None,
        draft: None,
    };
    let s = kaanal_server::serve(&Config::from_yaml(YAML).unwrap(), &dir, opts)
        .await
        .unwrap();
    let srv = Srv {
        base: format!("http://127.0.0.1:{}", s.addr.port()),
        token,
        client: reqwest::Client::new(),
    };
    (srv, s)
}

impl Srv {
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

    async fn shutdown(&self, body: Option<Value>) -> reqwest::Response {
        self.ctl(Method::POST, "/__kaanal/shutdown", body).await
    }

    async fn healthy(&self) -> bool {
        self.ctl(Method::GET, "/__kaanal/health", None)
            .await
            .status()
            == 200
    }

    async fn chaos(&self, body: Value) {
        let r = self
            .ctl(Method::PUT, "/__kaanal/chaos/db", Some(body))
            .await;
        assert_eq!(r.status(), 200);
    }

    /// A `GET /users/1` running in the background.
    fn slow_get(&self) -> tokio::task::JoinHandle<reqwest::Result<reqwest::Response>> {
        let (c, url) = (self.client.clone(), format!("{}/users/1", self.base));
        tokio::spawn(async move { c.get(url).send().await })
    }
}

/// A hang holds the request far longer than any test here.
fn hang() -> Value {
    json!({ "error_rate": 1.0, "error_kind": "hang", "latency_ms": 30 })
}

/// `wait` ends within `secs`, with `Ok`; returns how long it took.
async fn ends_within(s: BoundServer, secs: u64) -> Duration {
    let t0 = Instant::now();
    tokio::time::timeout(Duration::from_secs(secs), s.wait())
        .await
        .expect("wait did not end")
        .unwrap();
    t0.elapsed()
}

#[tokio::test]
async fn post_shutdown_answers_202_and_the_server_stops() {
    let (s, srv) = start().await;
    let r = s.shutdown(None).await;
    assert_eq!(r.status(), 202);
    assert_eq!(
        r.json::<Value>().await.unwrap(),
        json!({ "shutting_down": true, "timeout_ms": 5000 })
    );
    ends_within(srv, 1).await;

    let (s, srv) = start().await;
    let r = s.shutdown(Some(json!({ "timeout": "2s" }))).await;
    assert_eq!(r.json::<Value>().await.unwrap()["timeout_ms"], 2000);
    ends_within(srv, 1).await;
}

#[tokio::test]
async fn a_second_request_never_extends_the_deadline() {
    let (s, srv) = start().await;
    s.chaos(hang()).await;
    let held = s.slow_get();
    tokio::time::sleep(Duration::from_millis(200)).await;
    s.shutdown(Some(json!({ "timeout": "400ms" }))).await;
    // The listener is gone, so a second HTTP ask cannot arrive; the handle is the second ask.
    let left = srv.shutdown(Duration::from_secs(60));
    assert!(left <= Duration::from_millis(400), "extended: {left:?}");
    let took = ends_within(srv, 3).await;
    assert!(took < Duration::from_secs(2), "{took:?}");
    assert!(held.await.unwrap().is_err());
}

#[tokio::test]
async fn bad_timeouts_are_400_naming_timeout() {
    let (s, _srv) = start().await;
    for body in [
        json!({ "timeout": "abc" }),
        json!({ "timeout": "0s" }),
        json!({ "timeout": "301s" }),
        json!({ "timeout": 5 }),
        json!([]),
    ] {
        let r = s.shutdown(Some(body.clone())).await;
        assert_eq!(r.status(), 400, "{body}");
        let m = r.json::<Value>().await.unwrap()["error"]
            .as_str()
            .unwrap()
            .to_string();
        if body.is_object() {
            assert!(m.contains("timeout"), "{body}: {m}");
        }
    }
    let r = s
        .client
        .post(format!("{}/__kaanal/shutdown", s.base))
        .bearer_auth(&s.token)
        .body("{not json")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 400);
    assert!(r.json::<Value>().await.unwrap()["error"].is_string());
    assert!(
        s.healthy().await,
        "a refused request must not stop the server"
    );
}

#[tokio::test]
async fn shutdown_needs_the_token() {
    let (s, _srv) = start().await;
    let url = format!("{}/__kaanal/shutdown", s.base);
    let r = s.client.post(&url).send().await.unwrap();
    assert_eq!(r.status(), 401);
    let r = s
        .client
        .post(&url)
        .bearer_auth(&s.token)
        .header("host", "evil.example")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 403);
    let r = s.ctl(Method::GET, "/__kaanal/shutdown", None).await;
    assert_eq!(r.status(), 405);
    // No CORS: a page on another origin gets no permission to call it.
    let r = s
        .client
        .post(&url)
        .header("origin", "http://evil.example")
        .send()
        .await
        .unwrap();
    assert!(r.headers().get("access-control-allow-origin").is_none());
    assert!(s.healthy().await, "none of these may stop the server");
}

#[tokio::test]
async fn in_flight_request_finishes_inside_the_budget() {
    let (s, srv) = start().await;
    s.chaos(json!({ "latency_ms": 800 })).await;
    let inflight = s.slow_get();
    // The request is on the wire well before the shutdown (it needs 800 ms to finish).
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(s.shutdown(None).await.status(), 202);
    let r = inflight.await.unwrap().unwrap();
    assert_eq!(r.status(), 200);
    ends_within(srv, 3).await;
}

#[tokio::test]
async fn in_flight_request_is_cut_at_the_budget() {
    let (s, srv) = start().await;
    s.chaos(hang()).await;
    let held = s.slow_get();
    tokio::time::sleep(Duration::from_millis(200)).await;
    let r = s.shutdown(Some(json!({ "timeout": "200ms" }))).await;
    assert_eq!(
        r.status(),
        202,
        "the answer to the shutdown call itself arrives"
    );
    let took = ends_within(srv, 2).await;
    assert!(
        took >= Duration::from_millis(100),
        "not before the budget: {took:?}"
    );
    let got = tokio::time::timeout(Duration::from_secs(2), held)
        .await
        .expect("the client was left hanging")
        .unwrap();
    assert!(got.is_err(), "the connection ended with no response");
}

#[tokio::test]
async fn new_connections_are_refused_while_draining() {
    let (s, _srv) = start().await;
    s.chaos(hang()).await;
    let _held = s.slow_get();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(s.shutdown(None).await.status(), 202);
    // The held request keeps the drain open for 5 s; a fresh connection still gets nobody.
    let addr = s.base.trim_start_matches("http://").to_string();
    let deadline = Instant::now() + Duration::from_secs(2);
    loop {
        if tokio::net::TcpStream::connect(&addr).await.is_err() {
            break;
        }
        assert!(Instant::now() < deadline, "still accepting while draining");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

impl Srv {
    /// Opens the event stream (the token in the query, as a browser would).
    async fn events(&self) -> reqwest::Response {
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
        r
    }
}

/// Reads an event stream to its end; returns everything it sent.
async fn read_to_end(mut r: reqwest::Response) -> String {
    let mut seen = String::new();
    tokio::time::timeout(Duration::from_secs(2), async {
        while let Some(chunk) = r.chunk().await.unwrap() {
            seen.push_str(&String::from_utf8_lossy(&chunk));
        }
    })
    .await
    .expect("the events stream did not end");
    seen
}

#[tokio::test]
async fn events_stream_ends_when_shutdown_is_requested() {
    let (s, srv) = start().await;
    let ev = s.events().await;
    assert_eq!(
        s.shutdown(Some(json!({ "timeout": "60s" }))).await.status(),
        202
    );
    let seen = read_to_end(ev).await;
    assert!(seen.contains("event: shutdown"), "{seen}");
    let data = seen.lines().find_map(|l| l.strip_prefix("data:")).unwrap();
    let ms = serde_json::from_str::<Value>(data.trim()).unwrap()["timeout_ms"]
        .as_u64()
        .unwrap();
    assert!(ms > 0 && ms <= 60_000, "{ms}");
    let took = ends_within(srv, 3).await;
    assert!(
        took < Duration::from_secs(3),
        "the drain waited for the stream: {took:?}"
    );
}

#[tokio::test]
async fn other_connections_still_drain_with_an_events_stream_open() {
    let (s, srv) = start().await;
    s.chaos(json!({ "latency_ms": 800 })).await;
    let ev = s.events().await;
    let inflight = s.slow_get();
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(s.shutdown(None).await.status(), 202);
    assert_eq!(inflight.await.unwrap().unwrap().status(), 200);
    assert!(read_to_end(ev).await.contains("event: shutdown"));
    ends_within(srv, 3).await;
}

#[tokio::test]
async fn a_whitespace_body_is_no_body() {
    let (s, srv) = start().await;
    let r = s
        .client
        .post(format!("{}/__kaanal/shutdown", s.base))
        .bearer_auth(&s.token)
        .body(
            " 
	",
        )
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 202);
    assert_eq!(r.json::<Value>().await.unwrap()["timeout_ms"], 5000);
    ends_within(srv, 1).await;
}
