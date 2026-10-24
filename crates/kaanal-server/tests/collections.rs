//! `collection:` at runtime: the store itself, then the request path (the chain still runs, and an
//! operation is applied only when the request succeeds).

use std::path::PathBuf;
use std::time::{Duration, Instant};

use kaanal_core::{CollectionOp, Config};
use kaanal_server::{Collections, MAX_ITEM_BYTES, MAX_ITEMS};
use reqwest::Method;
use serde_json::{Value, json};

// ---- the store -----------------------------------------------------------------------------

fn add(c: &Collections, name: &str, item: Value) -> (u16, Value) {
    c.apply(name, CollectionOp::Add, None, Some(item))
}

fn list(c: &Collections, name: &str) -> Value {
    let (status, body) = c.apply(name, CollectionOp::List, None, None);
    assert_eq!(status, 200);
    body
}

#[test]
fn add_list_get_remove_roundtrip() {
    let c = Collections::new();
    assert_eq!(list(&c, "notes"), json!([]));
    assert_eq!(
        add(&c, "notes", json!({ "text": "a" })),
        (201, json!({ "text": "a", "id": 1 }))
    );
    add(&c, "notes", json!({ "text": "b" }));
    assert_eq!(
        list(&c, "notes"),
        json!([{ "text": "a", "id": 1 }, { "text": "b", "id": 2 }])
    );
    let get = |id| c.apply("notes", CollectionOp::Get, Some(id), None);
    assert_eq!(get("2"), (200, json!({ "text": "b", "id": 2 })));
    let missing = (404, json!({ "error": "not found" }));
    for id in ["3", "0", "-1", "x", "", "99999999999999999999999999"] {
        assert_eq!(get(id), missing, "{id}");
    }
    let remove = |id| c.apply("notes", CollectionOp::Remove, Some(id), None);
    assert_eq!(remove("1"), (204, Value::Null));
    assert_eq!(remove("1"), missing);
    assert_eq!(get("1"), missing);
    assert_eq!(list(&c, "notes"), json!([{ "text": "b", "id": 2 }]));
    // collections are separate
    assert_eq!(list(&c, "other"), json!([]));
}

#[test]
fn ids_are_sequential_and_ignore_a_client_id() {
    let c = Collections::new();
    assert_eq!(add(&c, "notes", json!({ "id": 99 })).1, json!({ "id": 1 }));
    assert_eq!(add(&c, "notes", json!({ "id": "x" })).1["id"], 2);
    // a removed id is never handed out again
    c.apply("notes", CollectionOp::Remove, Some("2"), None);
    assert_eq!(add(&c, "notes", json!({})).1["id"], 3);
    // each collection counts on its own
    assert_eq!(add(&c, "tags", json!({})).1["id"], 1);
}

#[tokio::test]
async fn bad_oversize_and_full_collection_change_nothing() {
    let c = Collections::new();
    let bad = (400, json!({ "error": "item must be a JSON object" }));
    for item in [json!([1]), json!("x"), json!(1), Value::Null] {
        assert_eq!(add(&c, "notes", item.clone()), bad, "{item}");
    }
    assert_eq!(c.apply("notes", CollectionOp::Add, None, None), bad);
    let big = json!({ "x": "y".repeat(MAX_ITEM_BYTES) });
    assert_eq!(
        add(&c, "notes", big),
        (413, json!({ "error": "item too large" }))
    );
    assert_eq!(list(&c, "notes"), json!([]));
    for _ in 0..MAX_ITEMS {
        assert_eq!(add(&c, "notes", json!({})).0, 201);
    }
    assert_eq!(
        add(&c, "notes", json!({})),
        (409, json!({ "error": "collection full" }))
    );
    assert_eq!(list(&c, "notes").as_array().unwrap().len(), MAX_ITEMS);
    // the failures took no id
    c.apply("notes", CollectionOp::Remove, Some("1"), None);
    assert_eq!(add(&c, "notes", json!({})).1["id"], MAX_ITEMS + 1);

    // over the wire: a bad body is answered after the chain, and nothing is stored
    let s = start("bad").await;
    let deep = format!("{}{}", "[".repeat(10_000), "]".repeat(10_000));
    let huge = format!("{{\"x\":\"{}\"}}", "y".repeat(MAX_ITEM_BYTES));
    for (body, status) in [
        ("not json".to_string(), 400),
        ("[1,2]".to_string(), 400),
        ("{\"a\":".to_string(), 400),
        (deep, 400),
        (huge, 413),
    ] {
        let r = s.post_raw(body.into_bytes()).await;
        assert_eq!(r.status(), status);
        assert!(r.headers().get("x-kaanal-trace").is_some(), "the chain ran");
    }
    // The server stops reading at the cap and answers; with ten times the cap still unsent, the
    // close can reach the client as a reset before it reads the 413 (Windows), so either is fine.
    let r = s
        .client
        .post(format!("{}/notes", s.base))
        .header("content-type", "application/json")
        .body("x".repeat(10 * MAX_ITEM_BYTES))
        .send()
        .await;
    if let Ok(r) = r {
        assert_eq!(r.status(), 413);
    }
    let r = s.post_raw(vec![b'{', 0xff, b'}']).await;
    assert_eq!(r.status(), 400);
    assert_eq!(s.notes().await, json!([]));
}

#[tokio::test]
async fn fifty_concurrent_adds_get_distinct_ids() {
    let s = start("fifty").await;
    let tasks: Vec<_> = (0..50)
        .map(|i| {
            let (client, base) = (s.client.clone(), s.base.clone());
            tokio::spawn(async move {
                let r = client
                    .post(format!("{base}/notes"))
                    .json(&json!({ "n": i }))
                    .send()
                    .await
                    .unwrap();
                assert_eq!(r.status(), 201);
                r.json::<Value>().await.unwrap()["id"].as_u64().unwrap()
            })
        })
        .collect();
    let mut ids = Vec::new();
    for t in tasks {
        ids.push(t.await.unwrap());
    }
    ids.sort_unstable();
    assert_eq!(ids, (1..=50).collect::<Vec<_>>());
    assert_eq!(s.notes().await.as_array().unwrap().len(), 50);
}

#[tokio::test]
async fn reset_clears_every_collection() {
    let c = Collections::new();
    add(&c, "notes", json!({}));
    add(&c, "tags", json!({}));
    c.clear();
    assert_eq!(
        (list(&c, "notes"), list(&c, "tags")),
        (json!([]), json!([]))
    );
    assert_eq!(add(&c, "notes", json!({})).1["id"], 1, "ids start over");

    let s = start("reset").await;
    s.post(json!({ "a": 1 })).await;
    s.post(json!({ "a": 2 })).await;
    let r = s.ctl(Method::POST, "/__kaanal/reset", None).await;
    assert!(r.status().is_success());
    assert_eq!(s.notes().await, json!([]));
    assert_eq!(s.post(json!({})).await.1["id"], 1);
}

#[tokio::test]
async fn a_collection_get_still_goes_through_the_chain() {
    let s = start("chain").await;
    s.chaos("app", json!({ "latency_ms": 300 })).await;
    let t = Instant::now();
    let r = s.get("/notes").await;
    assert!(
        t.elapsed() >= Duration::from_millis(290),
        "{:?}",
        t.elapsed()
    );
    assert_eq!(r.status(), 200);
    let trace = r.headers()["x-kaanal-trace"].to_str().unwrap().to_string();
    assert!(trace.contains("db"), "{trace}");
    assert_eq!(r.json::<Value>().await.unwrap(), json!([]));
}

#[tokio::test]
async fn a_chaos_failed_post_does_not_persist() {
    let s = start("chaos").await;
    s.chaos("db", json!({ "error_rate": 1.0, "error_status": 503 }))
        .await;
    assert_eq!(s.post(json!({ "a": 1 })).await.0, 503);
    s.calm("db").await;
    assert_eq!(s.notes().await, json!([]));
    assert_eq!(s.post(json!({})).await.1["id"], 1);
}

#[tokio::test]
async fn a_wire_failed_post_does_not_persist_or_apply_twice() {
    let s = start("wire").await;
    s.chaos(
        "app",
        json!({ "error_rate": 1.0, "error_kind": "malformed" }),
    )
    .await;
    let r = s
        .client
        .post(format!("{}/notes", s.base))
        .json(&json!({ "a": 1 }))
        .send()
        .await
        .unwrap();
    // the would-be answer, cut short: the status of an add, a body that is not JSON
    assert_eq!(r.status(), 201);
    let text = r.text().await.unwrap();
    assert!(
        text.starts_with('{') && serde_json::from_str::<Value>(&text).is_err(),
        "{text}"
    );
    // a read through the same failure is a preview too, and harmless
    s.get("/notes").await;
    s.calm("app").await;
    assert_eq!(s.notes().await, json!([]));
    // the preview took no id
    assert_eq!(s.post(json!({})).await.1["id"], 1);
    assert_eq!(s.notes().await.as_array().unwrap().len(), 1);
}

#[tokio::test]
async fn a_pinned_error_post_does_not_persist() {
    let s = start("pin-error").await;
    // the pin really answered: error is its 500, empty the route's 201 with a derived body
    for (state, status) in [("error", 500), ("empty", 201)] {
        s.pin(json!({ "endpoint": "POST /notes", "state": state }))
            .await;
        assert_eq!(s.post(json!({ "a": 1 })).await.0, status, "{state}");
    }
    s.ctl(Method::DELETE, "/__kaanal/pins", None).await;
    assert_eq!(s.notes().await, json!([]));
    assert_eq!(s.post(json!({})).await.1["id"], 1);
}

#[tokio::test]
async fn a_wire_failed_delete_does_not_report_success() {
    let s = start("wire-delete").await;
    s.post(json!({ "a": 1 })).await;
    for kind in ["malformed", "reset"] {
        s.chaos("app", json!({ "error_rate": 1.0, "error_kind": kind }))
            .await;
        // a 204 has no body to break, so it fails as a chaos error, never a clean 204
        let r = s.req(Method::DELETE, "/notes/1").await;
        assert_eq!(r.status(), 502, "{kind}");
        assert_eq!(
            r.json::<Value>().await.unwrap(),
            json!({ "error": "chaos", "node": "app", "kind": kind })
        );
        // a HEAD has no body either
        let r = s.req(Method::HEAD, "/notes/1").await;
        assert_eq!(r.status(), 502, "{kind}");
        s.calm("app").await;
        assert_eq!(s.get("/notes/1").await.status(), 200, "{kind}: still there");
    }
}

#[tokio::test]
async fn a_failed_or_pinned_delete_removes_nothing() {
    let s = start("delete-kept").await;
    s.post(json!({ "a": 1 })).await;
    s.chaos("db", json!({ "error_rate": 1.0, "error_status": 503 }))
        .await;
    assert_eq!(s.req(Method::DELETE, "/notes/1").await.status(), 503);
    s.calm("db").await;
    // a pinned empty answers the route's 204, yet removes nothing
    for (state, status) in [("error", 500), ("empty", 204)] {
        s.pin(json!({ "endpoint": "DELETE /notes/{id}", "state": state }))
            .await;
        let r = s.req(Method::DELETE, "/notes/1").await;
        assert_eq!(r.status(), status, "{state}");
    }
    s.ctl(Method::DELETE, "/__kaanal/pins", None).await;
    assert_eq!(s.notes().await, json!([{ "a": 1, "id": 1 }]));
    assert_eq!(s.req(Method::DELETE, "/notes/1").await.status(), 204);
    assert_eq!(s.notes().await, json!([]));
}

#[test]
fn ids_must_be_canonical() {
    let c = Collections::new();
    for _ in 0..7 {
        add(&c, "notes", json!({}));
    }
    let get = |id| c.apply("notes", CollectionOp::Get, Some(id), None).0;
    for id in ["007", "+7", " 7", "7 ", "07"] {
        assert_eq!(get(id), 404, "{id:?}");
        assert_eq!(
            c.apply("notes", CollectionOp::Remove, Some(id), None).0,
            404
        );
    }
    assert_eq!(get("7"), 200);
}

#[tokio::test]
async fn a_head_of_a_missing_id_is_404_with_no_body() {
    let s = start("head-missing").await;
    let r = s.req(Method::HEAD, "/notes/1").await;
    assert_eq!(r.status(), 404);
    assert!(r.bytes().await.unwrap().is_empty());
}

#[tokio::test]
async fn a_pinned_loading_post_persists_once() {
    let s = start("pin-loading").await;
    s.pin(json!({ "endpoint": "POST /notes", "state": "loading", "delay": "10ms" }))
        .await;
    assert_eq!(
        s.post(json!({ "a": 1 })).await,
        (201, json!({ "a": 1, "id": 1 }))
    );
    s.ctl(Method::DELETE, "/__kaanal/pins", None).await;
    assert_eq!(s.notes().await, json!([{ "a": 1, "id": 1 }]));
}

#[tokio::test]
async fn get_of_an_empty_collection_is_an_empty_array() {
    let s = start("empty").await;
    assert_eq!(s.notes().await, json!([]));
    let r = s.req(Method::HEAD, "/notes").await;
    assert_eq!(r.status(), 200);
    assert!(r.bytes().await.unwrap().is_empty());
    for p in [
        "/notes/1",
        "/notes/99999999999999999999999999",
        "/notes/%FF",
    ] {
        let r = s.get(p).await;
        assert_eq!(r.status(), 404, "{p}");
        assert_eq!(
            r.json::<Value>().await.unwrap(),
            json!({ "error": "not found" })
        );
    }
    assert_eq!(s.req(Method::DELETE, "/notes/1").await.status(), 404);
    // a HEAD by id reads and never writes
    s.post(json!({ "a": 1 })).await;
    assert_eq!(s.req(Method::HEAD, "/notes/1").await.status(), 200);
    assert_eq!(s.get("/notes/1").await.status(), 200);
    assert_eq!(s.req(Method::DELETE, "/notes/1").await.status(), 204);
    assert_eq!(s.notes().await, json!([]));
}

// ---- a live server -------------------------------------------------------------------------

const SPEC: &str = r##"openapi: 3.0.3
info: { title: Notes, version: "1.0" }
paths:
  /notes:
    get:
      responses:
        "200":
          description: Notes
          content:
            application/json:
              schema: { type: array, items: { $ref: "#/components/schemas/Note" } }
    post:
      responses:
        "201":
          description: Added
          content:
            application/json:
              schema: { $ref: "#/components/schemas/Note" }
  /notes/{id}:
    get:
      parameters: [ { name: id, in: path, required: true, schema: { type: integer } } ]
      responses:
        "200":
          description: One note
          content:
            application/json:
              schema: { $ref: "#/components/schemas/Note" }
    delete:
      parameters: [ { name: id, in: path, required: true, schema: { type: integer } } ]
      responses:
        "204": { description: Removed }
components:
  schemas:
    Note:
      type: object
      properties:
        id: { type: integer }
        text: { type: string }
"##;

/// A gateway, the notes service and a database; every notes route is the `notes` collection and
/// visits the database.
const CONFIG: &str = "version: 1
seed: 42
nodes:
  nginx: { type: gateway, proc: 1ms }
  app:   { type: service, spec: openapi.yaml, proc: 1ms }
  db:    { type: database, ops: { read: 1ms, write: 1ms, scan: 1ms } }
edges:
  - { from: nginx, to: app, net: 1ms }
  - { from: app, to: db, net: 1ms }
endpoints:
  \"GET /notes\":         { collection: notes, chain: [ db ] }
  \"POST /notes\":        { collection: notes, chain: [ db ] }
  \"GET /notes/{id}\":    { collection: notes, chain: [ db ] }
  \"DELETE /notes/{id}\": { collection: notes, chain: [ db ] }
";

struct Srv {
    base: String,
    token: String,
    client: reqwest::Client,
}

async fn start(name: &str) -> Srv {
    let dir: PathBuf =
        std::env::temp_dir().join(format!("kaanal-coll-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("openapi.yaml"), SPEC).unwrap();
    let token = kaanal_server::new_token();
    let opts = kaanal_server::ServeOpts {
        host: "127.0.0.1".parse().unwrap(),
        port: 0,
        token: token.clone(),
        auth_key: None,
    };
    let s = kaanal_server::serve(&Config::from_yaml(CONFIG).unwrap(), &dir, opts)
        .await
        .unwrap();
    Srv {
        base: format!("http://127.0.0.1:{}", s.addr.port()),
        token,
        client: reqwest::Client::new(),
    }
}

impl Srv {
    async fn req(&self, m: Method, path: &str) -> reqwest::Response {
        self.client
            .request(m, format!("{}{path}", self.base))
            .send()
            .await
            .unwrap()
    }

    async fn get(&self, path: &str) -> reqwest::Response {
        self.req(Method::GET, path).await
    }

    /// `GET /notes` as JSON; it must succeed.
    async fn notes(&self) -> Value {
        let r = self.get("/notes").await;
        assert_eq!(r.status(), 200);
        r.json().await.unwrap()
    }

    /// `POST /notes` with a JSON body: the status and the body (`null` when it is not JSON).
    async fn post(&self, item: Value) -> (u16, Value) {
        let r = self
            .client
            .post(format!("{}/notes", self.base))
            .json(&item)
            .send()
            .await
            .unwrap();
        let status = r.status().as_u16();
        (status, r.json().await.unwrap_or(Value::Null))
    }

    async fn post_raw(&self, body: Vec<u8>) -> reqwest::Response {
        self.client
            .post(format!("{}/notes", self.base))
            .header("content-type", "application/json")
            .body(body)
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

    async fn chaos(&self, node: &str, body: Value) {
        let r = self
            .ctl(Method::PUT, &format!("/__kaanal/chaos/{node}"), Some(body))
            .await;
        assert_eq!(r.status(), 200, "{node}");
    }

    async fn calm(&self, node: &str) {
        let r = self
            .ctl(Method::DELETE, &format!("/__kaanal/chaos/{node}"), None)
            .await;
        assert!(r.status().is_success(), "{node}");
    }

    async fn pin(&self, pin: Value) {
        let r = self
            .ctl(Method::PUT, "/__kaanal/pins", Some(pin.clone()))
            .await;
        assert!(r.status().is_success(), "{pin}");
    }
}
