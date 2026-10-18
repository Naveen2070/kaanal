//! Scenarios over HTTP: the control routes, stacking against live chaos, the snapshot rules.

use kaanal_core::Config;
use kaanal_server::ServeOpts;
use reqwest::Method;
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

const BASE: &str = "version: 1
seed: 42
nodes:
  nginx: { type: gateway, proc: 1ms }
  app:   { type: service, spec: openapi.yaml, proc: 1ms }
  db:    { type: database, flavor: postgres, ops: { read: 1ms, write: 1ms, scan: 1ms } }
edges:
  - { from: nginx, to: app, net: 1ms }
  - { from: app, to: db, net: 1ms }
";
const EP: &str = "GET /users/{id}";

struct Sc {
    base: String,
    token: String,
    client: reqwest::Client,
    dir: PathBuf,
}

impl Drop for Sc {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn new_dir(files: &[(&str, &str)]) -> PathBuf {
    static N: AtomicU32 = AtomicU32::new(0);
    let d = std::env::temp_dir().join(format!(
        "kaanal-sh-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    let spec = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/openapi.yaml");
    std::fs::copy(spec, d.join("openapi.yaml")).unwrap();
    for (f, body) in files {
        let p = d.join("scenarios").join(f);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, body).unwrap();
    }
    d
}

async fn start_in(dir: PathBuf, extra: &str) -> Sc {
    start_full(dir, &format!("{BASE}{extra}")).await
}

/// A server for a whole `kaanal.yaml`.
async fn start_full(dir: PathBuf, yaml: &str) -> Sc {
    let cfg = Config::from_yaml(yaml).unwrap();
    let token = kaanal_server::new_token();
    let opts = ServeOpts {
        host: "127.0.0.1".parse().unwrap(),
        port: 0,
        token: token.clone(),
        auth_key: None,
    };
    let s = kaanal_server::serve(&cfg, &dir, opts).await.unwrap();
    Sc {
        base: format!("http://127.0.0.1:{}", s.addr.port()),
        token,
        client: reqwest::Client::new(),
        dir,
    }
}

/// A server whose config carries `extra` (usually a `scenarios:` section).
async fn start_with_scenarios(extra: &str) -> Sc {
    start_in(new_dir(&[]), extra).await
}

impl Sc {
    async fn send(&self, m: Method, path: &str, body: Option<Value>) -> reqwest::Response {
        let mut rb = self
            .client
            .request(m, format!("{}{path}", self.base))
            .bearer_auth(&self.token);
        if let Some(b) = body {
            rb = rb.json(&b);
        }
        rb.send().await.unwrap()
    }
    /// A mock request (no token needed).
    async fn get(&self, path: &str) -> reqwest::Response {
        self.client
            .get(format!("{}{path}", self.base))
            .send()
            .await
            .unwrap()
    }
    async fn ctl(&self, path: &str) -> reqwest::Response {
        self.send(Method::GET, path, None).await
    }
    async fn post(&self, path: &str) -> reqwest::Response {
        self.send(Method::POST, path, None).await
    }
    async fn post_json(&self, path: &str, b: Value) -> reqwest::Response {
        self.send(Method::POST, path, Some(b)).await
    }
    async fn put(&self, path: &str, b: Value) -> reqwest::Response {
        self.send(Method::PUT, path, Some(b)).await
    }
    async fn delete(&self, path: &str) -> reqwest::Response {
        self.send(Method::DELETE, path, None).await
    }
    async fn put_chaos(&self, node: &str, b: Value) -> reqwest::Response {
        self.put(&format!("/__kaanal/chaos/{node}"), b).await
    }
    async fn status(&self, path: &str) -> u16 {
        self.get(path).await.status().as_u16()
    }
    async fn active(&self) -> Value {
        self.ctl("/__kaanal/scenarios/active")
            .await
            .json()
            .await
            .unwrap()
    }
    async fn live(&self) -> Value {
        self.ctl("/__kaanal/chaos").await.json().await.unwrap()
    }
    fn files(&self) -> Vec<String> {
        let mut v: Vec<String> = std::fs::read_dir(self.dir.join("scenarios"))
            .map(|rd| {
                rd.map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
                    .collect()
            })
            .unwrap_or_default();
        v.sort();
        v
    }
}

#[tokio::test]
async fn activate_stacks_and_deactivate_restores() {
    let s = start_with_scenarios(
        r"
scenarios:
  slow-db: { chaos: [ { node: db, latency_ms: 300 } ] }
  db-down: { chaos: [ { node: db, down: true } ] }
",
    )
    .await;
    assert_eq!(s.status("/users/42").await, 200);
    let r = s.post("/__kaanal/scenarios/db-down/activate").await;
    assert_eq!(r.status(), 200);
    let b: Value = r.json().await.unwrap();
    assert_eq!(b["active"], json!(["db-down"]));
    assert_eq!(b["effective"]["nodes"]["db"]["down"], true);
    assert_eq!(s.status("/users/42").await, 503);
    let r = s.post("/__kaanal/scenarios/db-down/deactivate").await;
    assert_eq!(r.status(), 204);
    assert_eq!(s.status("/users/42").await, 200);
    assert_eq!(
        s.post("/__kaanal/scenarios/ghost/activate").await.status(),
        404
    );
    // Not active: still 204.
    assert_eq!(
        s.post("/__kaanal/scenarios/ghost/deactivate")
            .await
            .status(),
        204
    );
}

#[tokio::test]
async fn live_override_calms_a_scenario() {
    let s =
        start_with_scenarios("scenarios:\n  db-down: { chaos: [ { node: db, down: true } ] }\n")
            .await;
    s.post("/__kaanal/scenarios/db-down/activate").await;
    assert_eq!(s.status("/users/42").await, 503);
    // Without override, a live `down: false` cannot lower a scenario's outage.
    s.put_chaos("db", json!({ "down": false })).await;
    assert_eq!(s.status("/users/42").await, 503);
    s.put_chaos("db", json!({ "down": false, "mode": "override" }))
        .await;
    assert_eq!(s.status("/users/42").await, 200);
}

#[tokio::test]
async fn mode_is_sticky_until_merge_is_said() {
    let s = start_with_scenarios("").await;
    s.put_chaos("db", json!({ "down": false, "mode": "override" }))
        .await;
    assert_eq!(s.live().await["nodes"]["db"]["mode"], "override");
    // No `mode` in the body: the stored mode stays.
    s.put_chaos("db", json!({ "latency_ms": 5 })).await;
    assert_eq!(s.live().await["nodes"]["db"]["mode"], "override");
    s.put_chaos("db", json!({ "mode": "merge" })).await;
    assert!(s.live().await["nodes"]["db"].get("mode").is_none());
    let bad = s.put_chaos("db", json!({ "mode": "replace" })).await;
    assert_eq!(bad.status(), 400);
}

#[tokio::test]
async fn delete_chaos_leaves_scenarios_and_delete_scenarios_leaves_live() {
    let s =
        start_with_scenarios("scenarios:\n  db-down: { chaos: [ { node: db, down: true } ] }\n")
            .await;
    s.put_chaos("db", json!({ "latency_ms": 100 })).await;
    s.post("/__kaanal/scenarios/db-down/activate").await;
    assert_eq!(s.delete("/__kaanal/chaos").await.status(), 204);
    assert_eq!(s.active().await["active"], json!(["db-down"]));
    assert_eq!(s.live().await["nodes"], json!({}));
    assert_eq!(s.status("/users/42").await, 503);

    s.put_chaos("db", json!({ "latency_ms": 100 })).await;
    assert_eq!(s.delete("/__kaanal/scenarios").await.status(), 204);
    assert_eq!(s.active().await["active"], json!([]));
    assert_eq!(s.live().await["nodes"]["db"]["latency_ms"], 100);
    assert_eq!(s.status("/users/42").await, 200);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_chaos_puts_all_land() {
    let s = std::sync::Arc::new(start_with_scenarios("").await);
    // 8 keys, two distinct fields each, 16 concurrent PUTs.
    let keys: Vec<Value> = vec![
        json!({ "node": "db" }),
        json!({ "node": "app" }),
        json!({ "node": "nginx" }),
        json!({ "endpoint": "GET /users" }),
        json!({ "endpoint": EP }),
        json!({ "endpoint": "POST /orders" }),
        json!({ "node": "db", "endpoint": EP }),
        json!({ "node": "app", "endpoint": "GET /users" }),
    ];
    let mut tasks = Vec::new();
    for i in 0..16usize {
        let (s, mut body) = (s.clone(), keys[i % 8].clone());
        tasks.push(tokio::spawn(async move {
            let o = body.as_object_mut().unwrap();
            if i < 8 {
                o.insert("latency_ms".into(), json!(10 + i));
            } else {
                o.insert(
                    "error_rate".into(),
                    json!(0.01 * f64::from(u32::try_from(i).unwrap())),
                );
            }
            let r = s.put("/__kaanal/chaos", body).await;
            assert_eq!(r.status(), 200);
        }));
    }
    for t in tasks {
        t.await.unwrap();
    }
    let all = s.live().await;
    let find = |j: usize| -> Value {
        match j {
            0..=2 => all["nodes"][["db", "app", "nginx"][j]].clone(),
            3..=5 => all["endpoints"][["GET /users", EP, "POST /orders"][j - 3]].clone(),
            _ => {
                let (n, e) = [("db", EP), ("app", "GET /users")][j - 6];
                all["node_at"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .find(|v| v["node"] == n && v["endpoint"] == e)
                    .cloned()
                    .unwrap_or(Value::Null)
            }
        }
    };
    for j in 0..8usize {
        let v = find(j);
        assert_eq!(v["latency_ms"], 10 + j, "key {j}: {all}");
        assert_eq!(
            v["error_rate"],
            json!(0.01 * f64::from(u32::try_from(j + 8).unwrap())),
            "key {j}"
        );
    }
    // The compiled view agrees.
    assert_eq!(all["effective"]["nodes"]["nginx"]["latency_ms"], 12);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn put_scenario_racing_activate_ends_consistent() {
    let s = std::sync::Arc::new(start_with_scenarios("").await);
    let def = |i: usize| {
        if i.is_multiple_of(2) {
            json!({ "chaos": [ { "node": "db", "down": true } ] })
        } else {
            json!({ "chaos": [ { "node": "db", "latency_ms": 100 } ] })
        }
    };
    // Two rounds so the last PUT is odd once and even once: both branches are live.
    for n in [20usize, 21] {
        assert_eq!(s.put("/__kaanal/scenarios/x", def(0)).await.status(), 200);
        let putter = {
            let s = s.clone();
            tokio::spawn(async move {
                for i in 1..=n {
                    assert_eq!(s.put("/__kaanal/scenarios/x", def(i)).await.status(), 200);
                }
            })
        };
        let activator = {
            let s = s.clone();
            tokio::spawn(async move {
                for _ in 0..20 {
                    let r = s.post("/__kaanal/scenarios/x/activate").await;
                    assert_eq!(r.status(), 200);
                }
            })
        };
        putter.await.unwrap();
        activator.await.unwrap();
        // Every PUT returned 200, so the last one is def(n): the stack must show exactly it.
        let want_db = if n.is_multiple_of(2) {
            json!({ "latency_ms": 0, "down": true, "faults": [] })
        } else {
            json!({ "latency_ms": 100, "down": false, "faults": [] })
        };
        let a = s.active().await;
        assert_eq!(a["active"], json!(["x"]), "n={n}");
        assert_eq!(
            a["effective"],
            json!({ "nodes": { "db": want_db }, "endpoints": {}, "node_at": [] }),
            "n={n}"
        );
        s.post("/__kaanal/reset").await;
    }
}

#[tokio::test]
async fn read_your_writes() {
    let s = start_with_scenarios("").await;
    for i in 0..50 {
        let down = i % 2 == 0;
        assert_eq!(
            s.put_chaos("db", json!({ "down": down })).await.status(),
            200
        );
        let want = if down { 503 } else { 200 };
        assert_eq!(s.status("/users/42").await, want, "iteration {i}");
    }
}

#[tokio::test]
async fn a_request_keeps_its_snapshot_across_a_swap() {
    let s = std::sync::Arc::new(
        start_with_scenarios("scenarios:\n  db-down: { chaos: [ { node: db, down: true } ] }\n")
            .await,
    );
    // A wide margin: the request must have planned its visit (well under 300 ms, even on a
    // loaded CI box) long before its 2 s delay ends, and the activation must land inside it.
    s.put_chaos("db", json!({ "latency_ms": 2000 })).await;
    let slow = {
        let s = s.clone();
        tokio::spawn(async move { s.status("/users/42").await })
    };
    tokio::time::sleep(Duration::from_millis(300)).await;
    s.post("/__kaanal/scenarios/db-down/activate").await;
    assert_eq!(
        slow.await.unwrap(),
        200,
        "the in-flight request keeps its plan"
    );
    assert_eq!(s.status("/users/42").await, 503);
}

async fn run(s: &Sc) -> Vec<u16> {
    s.put_chaos("db", json!({ "error_rate": 0.5 })).await;
    let mut v = Vec::new();
    for _ in 0..20 {
        v.push(s.status("/users/42").await);
    }
    v
}

#[tokio::test]
async fn reset_clears_everything_and_repeats_the_seeded_pattern() {
    let s =
        start_with_scenarios("scenarios:\n  db-down: { chaos: [ { node: db, down: true } ] }\n")
            .await;
    let first = run(&s).await;
    assert!(first.contains(&200) && first.contains(&503), "{first:?}");
    s.post("/__kaanal/scenarios/db-down/activate").await;
    assert_eq!(s.post("/__kaanal/reset").await.status(), 204);
    assert_eq!(s.active().await["active"], json!([]));
    assert_eq!(s.live().await["nodes"], json!({}));
    assert_eq!(run(&s).await, first);
}

#[tokio::test]
async fn scenario_names_are_checked_everywhere() {
    let s = start_with_scenarios("").await;
    let body = json!({ "chaos": [ { "node": "db", "down": true } ] });
    for bad in ["..%2Fx", "CON", "Payments", "a%2Fb", "save", "active"] {
        let st = s
            .put(&format!("/__kaanal/scenarios/{bad}"), body.clone())
            .await
            .status()
            .as_u16();
        // `save` and `active` are static routes: wrong method or body shape, never a definition.
        assert!([400, 405, 422].contains(&st), "{bad}: {st}");
    }
    for bad in [
        "../x",
        "a/b",
        "Con",
        "save",
        "reset",
        "",
        "x".repeat(65).as_str(),
    ] {
        let r = s
            .post_json(
                "/__kaanal/scenarios/save",
                json!({ "name": bad, "from": "live" }),
            )
            .await;
        assert_eq!(r.status(), 400, "{bad}");
    }
    let list: Value = s.ctl("/__kaanal/scenarios").await.json().await.unwrap();
    assert_eq!(list, json!([]));
    assert_eq!(s.files(), Vec::<String>::new());
}

#[tokio::test]
async fn save_writes_sparse_layers_and_refuses_overwrite() {
    let s = start_with_scenarios(
        r"
scenarios:
  a: { chaos: [ { node: db, latency_ms: 100 }, { endpoint: 'GET /users/{id}', error_rate: 0.5, error_kind: reset } ] }
  b: { chaos: [ { node: db, latency_ms: 50 }, { node: app, down: true } ] }
",
    )
    .await;
    s.put_chaos("db", json!({ "latency_ms": 100 })).await;
    let r = s
        .post_json("/__kaanal/scenarios/save", json!({ "name": "mine" }))
        .await;
    assert_eq!(r.status(), 200);
    let b: Value = r.json().await.unwrap();
    assert_eq!(b["name"], "mine");
    assert!(b["path"].as_str().unwrap().ends_with("mine.yaml"), "{b}");
    let text = std::fs::read_to_string(s.dir.join("scenarios/mine.yaml")).unwrap();
    assert!(text.contains("latency_ms"), "{text}");
    for absent in ["error_kind", "error_status", "error_rate", "down"] {
        assert!(!text.contains(absent), "{absent} in {text}");
    }
    // Exists: 409 without force, 200 with, and the file is replaced.
    let again = s
        .post_json("/__kaanal/scenarios/save", json!({ "name": "mine" }))
        .await;
    assert_eq!(again.status(), 409);
    s.put_chaos("db", json!({ "latency_ms": 7 })).await;
    let forced = s
        .post_json(
            "/__kaanal/scenarios/save",
            json!({ "name": "mine", "force": true }),
        )
        .await;
    assert_eq!(forced.status(), 200);
    let text = std::fs::read_to_string(s.dir.join("scenarios/mine.yaml")).unwrap();
    assert!(text.contains('7') && !text.contains("100"), "{text}");
    // Nothing else is left in the directory (no temp file).
    assert_eq!(s.files(), vec!["mine.yaml".to_string()]);
    // The saved scenario is registered (as a file scenario).
    let one: Value = s
        .ctl("/__kaanal/scenarios/mine")
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(one["source"], "file");
    // An inline name is refused, even with force.
    let inline = s
        .post_json(
            "/__kaanal/scenarios/save",
            json!({ "name": "a", "force": true }),
        )
        .await;
    assert_eq!(inline.status(), 400);

    // from: effective round-trips through a fresh server.
    s.delete("/__kaanal/chaos").await;
    s.post("/__kaanal/scenarios/a/activate").await;
    s.post("/__kaanal/scenarios/b/activate").await;
    // A calmed key (b takes `app` down, the live override calms it) is neither shown nor saved.
    s.put_chaos("app", json!({ "down": false, "mode": "override" }))
        .await;
    let want = s.active().await["effective"].clone();
    assert!(want["nodes"].get("app").is_none(), "{want}");
    let r = s
        .post_json(
            "/__kaanal/scenarios/save",
            json!({ "name": "combo", "from": "effective" }),
        )
        .await;
    assert_eq!(r.status(), 200, "{:?}", r.text().await);
    let dir = new_dir(&[]);
    std::fs::create_dir_all(dir.join("scenarios")).unwrap();
    std::fs::copy(
        s.dir.join("scenarios/combo.yaml"),
        dir.join("scenarios/combo.yaml"),
    )
    .unwrap();
    let fresh = start_in(dir, "").await;
    assert_eq!(
        fresh
            .post("/__kaanal/scenarios/combo/activate")
            .await
            .status(),
        200
    );
    assert_eq!(fresh.active().await["effective"], want);
}

#[tokio::test]
async fn save_from_effective_refuses_banded_keys_and_from_live_refuses_override() {
    let s = start_with_scenarios(
        r"
scenarios:
  p: { chaos: [ { node: db, error_rate: 0.3, error_kind: reset } ] }
  q: { chaos: [ { node: db, error_rate: 0.3 } ] }
",
    )
    .await;
    s.post("/__kaanal/scenarios/p/activate").await;
    s.post("/__kaanal/scenarios/q/activate").await;
    let r = s
        .post_json(
            "/__kaanal/scenarios/save",
            json!({ "name": "m", "from": "effective" }),
        )
        .await;
    assert_eq!(r.status(), 400);
    assert!(r.text().await.unwrap().contains("db"));
    s.put_chaos("app", json!({ "down": false, "mode": "override" }))
        .await;
    let r = s
        .post_json("/__kaanal/scenarios/save", json!({ "name": "m" }))
        .await;
    assert_eq!(r.status(), 400);
    // `from: <scenario>` copies a definition.
    let r = s
        .post_json(
            "/__kaanal/scenarios/save",
            json!({ "name": "p2", "from": { "scenario": "p" } }),
        )
        .await;
    assert_eq!(r.status(), 200);
    assert_eq!(
        s.post_json(
            "/__kaanal/scenarios/save",
            json!({ "name": "z", "from": { "scenario": "nope" } })
        )
        .await
        .status(),
        404
    );
    assert_eq!(s.files(), vec!["p2.yaml".to_string()]);
}

#[tokio::test]
async fn save_from_effective_skips_a_pair_equal_to_its_node_and_refuses_a_different_one() {
    let s = start_with_scenarios("").await;
    s.put_chaos("db", json!({ "down": true })).await;
    let pair = |b: Value| {
        let mut b = b;
        b["node"] = json!("db");
        b["endpoint"] = json!(EP);
        b
    };
    // The pair compiles to exactly what the node already does: nothing to write for it.
    s.put("/__kaanal/chaos", pair(json!({ "down": true })))
        .await;
    let r = s
        .post_json(
            "/__kaanal/scenarios/save",
            json!({ "name": "same", "from": "effective" }),
        )
        .await;
    assert_eq!(r.status(), 200, "{:?}", r.text().await);
    let text = std::fs::read_to_string(s.dir.join("scenarios/same.yaml")).unwrap();
    assert_eq!(text.matches("\"node\"").count(), 1, "{text}");
    assert!(!text.contains("endpoint"), "{text}");
    // A pair that differs cannot be written alone: still refused.
    s.put("/__kaanal/chaos", pair(json!({ "latency_ms": 50 })))
        .await;
    let r = s
        .post_json(
            "/__kaanal/scenarios/save",
            json!({ "name": "diff", "from": "effective" }),
        )
        .await;
    assert_eq!(r.status(), 400);
    assert!(r.text().await.unwrap().contains("compiled together"));
}

#[tokio::test]
async fn save_with_clear_live_does_not_double_count() {
    let s = start_with_scenarios("").await;
    s.put_chaos("db", json!({ "latency_ms": 100 })).await;
    let r = s
        .post_json(
            "/__kaanal/scenarios/save",
            json!({ "name": "mine", "clear_live": true }),
        )
        .await;
    assert_eq!(r.status(), 200);
    assert_eq!(s.live().await["nodes"], json!({}));
    s.post("/__kaanal/scenarios/mine/activate").await;
    assert_eq!(
        s.active().await["effective"]["nodes"]["db"]["latency_ms"],
        100
    );
}

#[tokio::test]
async fn put_scenarios_active_sets_the_stack_atomically() {
    let s = start_with_scenarios(
        "scenarios:\n  a: { chaos: [ { node: db, latency_ms: 1 } ] }\n  b: { chaos: [ { node: app, latency_ms: 1 } ] }\n",
    )
    .await;
    let r = s.put("/__kaanal/scenarios/active", json!(["a", "b"])).await;
    assert_eq!(r.status(), 200);
    assert_eq!(
        r.json::<Value>().await.unwrap()["active"],
        json!(["a", "b"])
    );
    let r = s
        .put("/__kaanal/scenarios/active", json!(["a", "ghost"]))
        .await;
    assert_eq!(r.status(), 404);
    assert_eq!(s.active().await["active"], json!(["a", "b"]));
    let r = s.put("/__kaanal/scenarios/active", json!([])).await;
    assert_eq!(r.status(), 200);
    assert_eq!(s.active().await["active"], json!([]));
}

#[tokio::test]
async fn static_active_routes_win_over_the_name_capture() {
    let s =
        start_with_scenarios("scenarios:\n  a: { chaos: [ { node: db, latency_ms: 1 } ] }\n").await;
    let g = s.ctl("/__kaanal/scenarios/active").await;
    assert_eq!(g.status(), 200);
    assert!(g.json::<Value>().await.unwrap().get("active").is_some());
    let p = s.put("/__kaanal/scenarios/active", json!(["a"])).await;
    assert_eq!(p.status(), 200);
    // `active` is not a definable or deletable name.
    assert_eq!(s.delete("/__kaanal/scenarios/active").await.status(), 405);
}

#[tokio::test]
async fn endpoint_latency_from_two_scenarios_clamps() {
    let s = start_with_scenarios(
        r"
scenarios:
  one: { chaos: [ { endpoint: 'GET /users/{id}', latency_ms: 240000 } ] }
  two: { chaos: [ { endpoint: 'GET /users/{id}', latency_ms: 240000 } ] }
",
    )
    .await;
    assert_eq!(
        s.post("/__kaanal/scenarios/one/activate").await.status(),
        200
    );
    assert_eq!(
        s.post("/__kaanal/scenarios/two/activate").await.status(),
        200
    );
    let a = s.active().await;
    assert_eq!(a["effective"]["endpoints"][EP]["latency_ms"], 300_000);
}

#[tokio::test]
async fn scenario_events_are_emitted() {
    let s =
        start_with_scenarios("scenarios:\n  a: { chaos: [ { node: db, latency_ms: 1 } ] }\n").await;
    let mut stream = s
        .client
        .get(format!("{}/__kaanal/events?token={}", s.base, s.token))
        .send()
        .await
        .unwrap();
    s.post("/__kaanal/scenarios/a/activate").await;
    s.post("/__kaanal/scenarios/a/deactivate").await;
    s.post("/__kaanal/reset").await;
    let mut seen = String::new();
    let ok = tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(chunk) = stream.chunk().await.unwrap() {
            seen.push_str(&String::from_utf8_lossy(&chunk));
            if seen.contains("\"action\":\"reset\"") {
                return true;
            }
        }
        false
    })
    .await
    .unwrap_or(false);
    assert!(ok, "{seen}");
    assert!(seen.contains("event: scenario"), "{seen}");
    assert!(seen.contains("\"action\":\"activate\""), "{seen}");
    assert!(seen.contains("\"action\":\"deactivate\""), "{seen}");
    assert!(seen.contains("\"name\":\"a\""), "{seen}");
}

#[tokio::test]
async fn runtime_scenarios_define_list_show_and_undefine() {
    let s = start_with_scenarios("scenarios:\n  inl: { chaos: [ { node: db, latency_ms: 1 } ] }\n")
        .await;
    let def = json!({ "description": "d", "chaos": [
        { "node": "db", "down": true },
        { "node": "app", "error_kind": "reset" },
    ] });
    let r = s.put("/__kaanal/scenarios/rt", def).await;
    assert_eq!(r.status(), 200);
    let b: Value = r.json().await.unwrap();
    assert_eq!(b["name"], "rt");
    // error_kind without error_rate: a warning, not an error.
    assert_eq!(b["warnings"].as_array().unwrap().len(), 1, "{b}");
    let list: Value = s.ctl("/__kaanal/scenarios").await.json().await.unwrap();
    assert_eq!(
        list,
        json!([
            { "name": "inl", "source": "inline", "description": null, "entries": 1, "active": false },
            { "name": "rt", "source": "runtime", "description": "d", "entries": 2, "active": false },
        ])
    );
    let one: Value = s.ctl("/__kaanal/scenarios/rt").await.json().await.unwrap();
    // Entries come back sorted by node id: app before db.
    assert_eq!(
        one["chaos"][0],
        json!({ "node": "app", "error_kind": "reset" })
    );
    assert_eq!(one["chaos"][1], json!({ "node": "db", "down": true }));
    // Bad definitions change nothing: unknown node 404, bad value 400, no entries 400.
    for (body, code) in [
        (
            json!({ "chaos": [ { "node": "ghost", "down": true } ] }),
            404,
        ),
        (
            json!({ "chaos": [ { "node": "db", "error_rate": 2.0 } ] }),
            400,
        ),
        (json!({ "chaos": [] }), 400),
    ] {
        assert_eq!(s.put("/__kaanal/scenarios/rt", body).await.status(), code);
    }
    // An inline name cannot be redefined or undefined.
    let redo = s
        .put(
            "/__kaanal/scenarios/inl",
            json!({ "chaos": [ { "node": "db", "down": true } ] }),
        )
        .await;
    assert_eq!(redo.status(), 409);
    assert_eq!(s.delete("/__kaanal/scenarios/inl").await.status(), 409);
    assert_eq!(s.delete("/__kaanal/scenarios/ghost").await.status(), 404);
    // Undefining an active runtime scenario drops it from the stack.
    s.post("/__kaanal/scenarios/rt/activate").await;
    assert_eq!(s.status("/users/42").await, 503);
    assert_eq!(s.delete("/__kaanal/scenarios/rt").await.status(), 204);
    assert_eq!(s.active().await["active"], json!([]));
    assert_eq!(s.status("/users/42").await, 200);
}

#[tokio::test]
async fn chaos_responses_carry_effective_and_warnings() {
    let s =
        start_with_scenarios("scenarios:\n  slow: { chaos: [ { node: db, latency_ms: 30 } ] }\n")
            .await;
    s.post("/__kaanal/scenarios/slow/activate").await;
    let r = s
        .put("/__kaanal/chaos", json!({ "node": "db", "latency_ms": 20 }))
        .await;
    let b: Value = r.json().await.unwrap();
    assert_eq!(b["effective"]["nodes"]["db"]["latency_ms"], 50);
    assert_eq!(b["warnings"], json!([]));
    let r = s.put_chaos("app", json!({ "error_kind": "reset" })).await;
    let b: Value = r.json().await.unwrap();
    assert_eq!(b["warnings"].as_array().unwrap().len(), 1, "{b}");
    let w = b["warnings"][0].as_str().unwrap();
    assert!(w.contains("node 'app'"), "{b}");
    // Only the key just written is reported.
    let b: Value = s
        .put_chaos("nginx", json!({ "latency_ms": 1 }))
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(b["warnings"], json!([]));
    assert_eq!(s.live().await["effective"]["nodes"]["db"]["latency_ms"], 50);
}

#[tokio::test]
async fn reset_undefines_runtime_scenarios_but_keeps_inline_and_file_ones() {
    let dir = new_dir(&[("fromfile.yaml", "chaos:\n  - { node: db, latency_ms: 1 }\n")]);
    let s = start_in(
        dir,
        "scenarios:\n  inl: { chaos: [ { node: db, latency_ms: 1 } ] }\n",
    )
    .await;
    s.put(
        "/__kaanal/scenarios/rt",
        json!({ "chaos": [ { "node": "db", "down": true } ] }),
    )
    .await;
    s.post("/__kaanal/scenarios/rt/activate").await;
    s.post("/__kaanal/scenarios/inl/activate").await;
    assert_eq!(s.post("/__kaanal/reset").await.status(), 204);
    let list: Value = s.ctl("/__kaanal/scenarios").await.json().await.unwrap();
    let names: Vec<&str> = list
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["inl", "fromfile"]);
    assert_eq!(s.active().await["active"], json!([]));
    assert_eq!(s.ctl("/__kaanal/scenarios/rt").await.status(), 404);
}

#[tokio::test]
async fn save_from_is_tagged() {
    let s =
        start_with_scenarios("scenarios:\n  live: { chaos: [ { node: db, latency_ms: 3 } ] }\n")
            .await;
    // A scenario may be called `live`; only the tagged form reaches it.
    let r = s
        .post_json(
            "/__kaanal/scenarios/save",
            json!({ "name": "copy", "from": { "scenario": "live" } }),
        )
        .await;
    assert_eq!(r.status(), 200);
    let text = std::fs::read_to_string(s.dir.join("scenarios/copy.yaml")).unwrap();
    assert!(text.contains("\"latency_ms\":3"), "{text}");
    // A bare scenario name is not accepted.
    let r = s
        .post_json(
            "/__kaanal/scenarios/save",
            json!({ "name": "copy2", "from": "someone" }),
        )
        .await;
    assert_eq!(r.status(), 422);
    // Serde's enum map forms are not accepted either: `{"live": null}` and friends are 422.
    for bad in [
        json!({ "live": null }),
        json!({ "effective": null }),
        json!({ "scenario": "live", "extra": 1 }),
        json!({ "scenario": 5 }),
        json!({}),
        json!("scenario"),
    ] {
        let r = s
            .post_json(
                "/__kaanal/scenarios/save",
                json!({ "name": "copy3", "from": bad.clone() }),
            )
            .await;
        assert_eq!(r.status(), 422, "{bad}");
        assert!(r.text().await.unwrap().contains("from must be"), "{bad}");
    }
    assert_eq!(s.files(), vec!["copy.yaml".to_string()]);
}

#[tokio::test]
async fn force_save_does_not_follow_a_planted_tmp_symlink() {
    let s = start_with_scenarios("").await;
    s.put_chaos("db", json!({ "latency_ms": 1 })).await;
    let r = s
        .post_json("/__kaanal/scenarios/save", json!({ "name": "m" }))
        .await;
    assert_eq!(r.status(), 200);
    let victim = s.dir.join("victim.txt");
    std::fs::write(&victim, "keep").unwrap();
    let tmp = s.dir.join("scenarios/.m.tmp");
    #[cfg(unix)]
    let linked = std::os::unix::fs::symlink(&victim, &tmp).is_ok();
    #[cfg(windows)]
    let linked = std::os::windows::fs::symlink_file(&victim, &tmp).is_ok();
    if !linked {
        eprintln!("skipped: cannot create symlinks here");
        return;
    }
    s.put_chaos("db", json!({ "latency_ms": 2 })).await;
    let r = s
        .post_json(
            "/__kaanal/scenarios/save",
            json!({ "name": "m", "force": true }),
        )
        .await;
    assert_eq!(r.status(), 200);
    assert_eq!(std::fs::read_to_string(&victim).unwrap(), "keep");
    let text = std::fs::read_to_string(s.dir.join("scenarios/m.yaml")).unwrap();
    assert!(text.contains('2'), "{text}");
    assert_eq!(s.files(), vec!["m.yaml".to_string()]);
}

const LIMITED: &str = "version: 1
seed: 42
nodes:
  nginx: { type: gateway, rate_limit: { rps: 1, burst: 1 } }
  app: { type: service, spec: openapi.yaml }
edges:
  - { from: nginx, to: app }
";

#[tokio::test]
async fn reset_refills_the_rate_limit_bucket() {
    let s = start_full(new_dir(&[]), LIMITED).await;
    assert_eq!(s.status("/users/42").await, 200);
    assert_eq!(s.status("/users/42").await, 429);
    assert_eq!(s.post("/__kaanal/reset").await.status(), 204);
    assert_eq!(s.status("/users/42").await, 200);
    assert_eq!(s.status("/users/42").await, 429);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn reset_during_in_flight_requests_does_not_hang() {
    // Burst 6 lets all six in; `app` runs one at a time with a queue, so waiters exist at reset.
    let busy = LIMITED
        .replace("burst: 1", "burst: 6")
        .replace("app: {", "app: { concurrency: { limit: 1, queue: 10 },");
    let s = std::sync::Arc::new(start_full(new_dir(&[]), &busy).await);
    s.put_chaos("app", json!({ "latency_ms": 300 })).await;
    let mut tasks = Vec::new();
    for _ in 0..6 {
        let s = s.clone();
        tasks.push(tokio::spawn(async move { s.status("/users/42").await }));
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(s.post("/__kaanal/reset").await.status(), 204);
    let all = tokio::time::timeout(Duration::from_secs(10), async {
        let mut v = Vec::new();
        for t in tasks {
            v.push(t.await.unwrap());
        }
        v
    })
    .await
    .expect("in-flight requests finish after a reset");
    assert!(all.iter().all(|c| *c == 200), "{all:?}");
    // The engine is healthy afterwards: after a reset the bucket is full again.
    s.post("/__kaanal/reset").await;
    assert_eq!(s.status("/users/42").await, 200);
}

#[tokio::test]
async fn put_over_an_active_scenario_emits_an_update_event() {
    let s = start_with_scenarios("").await;
    let def = |ms: u64| json!({ "chaos": [ { "node": "db", "latency_ms": ms } ] });
    s.put("/__kaanal/scenarios/x", def(1)).await;
    s.post("/__kaanal/scenarios/x/activate").await;
    let mut stream = s
        .client
        .get(format!("{}/__kaanal/events?token={}", s.base, s.token))
        .send()
        .await
        .unwrap();
    s.put("/__kaanal/scenarios/x", def(2)).await;
    let mut seen = String::new();
    let ok = tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(chunk) = stream.chunk().await.unwrap() {
            seen.push_str(&String::from_utf8_lossy(&chunk));
            if seen.contains("\"action\":\"update\"") {
                return true;
            }
        }
        false
    })
    .await
    .unwrap_or(false);
    assert!(ok, "{seen}");
    assert_eq!(
        s.active().await["effective"]["nodes"]["db"]["latency_ms"],
        2
    );
}

#[tokio::test]
async fn mode_only_patches() {
    let s =
        start_with_scenarios("scenarios:\n  d: { chaos: [ { node: db, down: true } ] }\n").await;
    // `merge` on a key with no live entry stores nothing.
    let r = s.put_chaos("db", json!({ "mode": "merge" })).await;
    assert_eq!(r.status(), 200);
    assert_eq!(s.live().await["nodes"], json!({}));
    // A bare override calms the key.
    s.post("/__kaanal/scenarios/d/activate").await;
    assert_eq!(s.status("/users/42").await, 503);
    s.put_chaos("db", json!({ "mode": "override" })).await;
    assert_eq!(s.status("/users/42").await, 200);
}

const TTL_SCENARIOS: &str = "scenarios:
  db-down: { chaos: [ { node: db, down: true } ] }
  slow: { chaos: [ { node: db, latency_ms: 1 } ] }
";

impl Sc {
    async fn activate_raw(&self, name: &str, body: &str) -> reqwest::Response {
        self.client
            .post(format!("{}/__kaanal/scenarios/{name}/activate", self.base))
            .bearer_auth(&self.token)
            .header("content-type", "application/json")
            .body(body.to_string())
            .send()
            .await
            .unwrap()
    }
    async fn active_names(&self) -> Value {
        self.active().await["active"].clone()
    }
}

/// Polls `f` for up to 3 s.
async fn until<F: std::future::Future<Output = bool>>(mut f: impl FnMut() -> F) -> bool {
    let end = tokio::time::Instant::now() + Duration::from_secs(3);
    while tokio::time::Instant::now() < end {
        if f().await {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    false
}

#[tokio::test]
async fn ttl_expires_and_chaos_clears() {
    let s = start_with_scenarios(TTL_SCENARIOS).await;
    let mut stream = s
        .client
        .get(format!("{}/__kaanal/events?token={}", s.base, s.token))
        .send()
        .await
        .unwrap();
    let r = s.activate_raw("db-down", r#"{"ttl":"300ms"}"#).await;
    assert_eq!(r.status(), 200);
    let b: Value = r.json().await.unwrap();
    let ms = b["expires_in_ms"]["db-down"].as_u64().unwrap();
    assert!((1..=300).contains(&ms), "{b}");
    assert_eq!(s.status("/users/1").await, 503);
    assert!(until(|| async { s.status("/users/1").await == 200 }).await);
    assert_eq!(s.active_names().await, json!([]));
    assert_eq!(s.active().await["expires_in_ms"], json!({}));
    let mut seen = String::new();
    let ok = tokio::time::timeout(Duration::from_secs(3), async {
        while let Some(chunk) = stream.chunk().await.unwrap() {
            seen.push_str(&String::from_utf8_lossy(&chunk));
            if seen.contains("\"action\":\"expire\"") {
                return true;
            }
        }
        false
    })
    .await
    .unwrap_or(false);
    assert!(ok, "{seen}");
}

#[tokio::test]
async fn reactivate_restarts_or_clears_the_lease() {
    let s = start_with_scenarios(TTL_SCENARIOS).await;
    s.activate_raw("db-down", r#"{"ttl":"300ms"}"#).await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert_eq!(
        s.activate_raw("db-down", r#"{"ttl":"600ms"}"#)
            .await
            .status(),
        200
    );
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(s.active_names().await, json!(["db-down"]));
    assert!(until(|| async { s.active_names().await == json!([]) }).await);

    s.activate_raw("db-down", r#"{"ttl":"300ms"}"#).await;
    let r = s.activate_raw("db-down", "").await;
    let b: Value = r.json().await.unwrap();
    assert_eq!(b["expires_in_ms"], json!({}));
    tokio::time::sleep(Duration::from_secs(1)).await;
    assert_eq!(s.active_names().await, json!(["db-down"]));
}

#[tokio::test]
async fn ttl_bounds_and_shape() {
    let s = start_with_scenarios(TTL_SCENARIOS).await;
    for bad in [
        r#"{"ttl":"99ms"}"#,
        r#"{"ttl":"25h"}"#,
        r#"{"ttl":"abc"}"#,
        r#"{"ttl":5}"#,
        r#"{"x":1}"#,
        "{not json",
    ] {
        let r = s.activate_raw("db-down", bad).await;
        assert_eq!(r.status(), 400, "{bad}");
        assert_eq!(s.active_names().await, json!([]), "{bad}");
    }
    for ok in [
        r#"{"ttl":"100ms"}"#,
        r#"{"ttl":"24h"}"#,
        "{}",
        r#"{"ttl":null}"#,
    ] {
        assert_eq!(s.activate_raw("db-down", ok).await.status(), 200, "{ok}");
    }
}

#[tokio::test]
async fn lease_dropped_with_the_activation() {
    let s = start_with_scenarios(TTL_SCENARIOS).await;
    let lease = r#"{"ttl":"1h"}"#;
    assert_eq!(s.activate_raw("db-down", lease).await.status(), 200);
    s.post("/__kaanal/scenarios/db-down/deactivate").await;
    assert_eq!(s.active().await["expires_in_ms"], json!({}));
    s.activate_raw("db-down", lease).await;
    s.put("/__kaanal/scenarios/active", json!(["slow"])).await;
    assert_eq!(s.active().await["expires_in_ms"], json!({}));
    s.activate_raw("db-down", lease).await;
    s.delete("/__kaanal/scenarios").await;
    assert_eq!(s.active().await["expires_in_ms"], json!({}));
    s.activate_raw("db-down", lease).await;
    s.post("/__kaanal/reset").await;
    assert_eq!(s.active().await["expires_in_ms"], json!({}));

    // A runtime scenario with a short lease, deleted: nothing left to expire.
    s.put(
        "/__kaanal/scenarios/rt",
        json!({ "chaos": [ { "node": "db", "down": true } ] }),
    )
    .await;
    s.activate_raw("rt", r#"{"ttl":"300ms"}"#).await;
    assert_eq!(s.delete("/__kaanal/scenarios/rt").await.status(), 204);
    assert_eq!(s.active().await["expires_in_ms"], json!({}));
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(s.active_names().await, json!([]));
    assert_eq!(s.status("/users/1").await, 200);
}

#[tokio::test]
async fn lease_and_live_write_both_survive() {
    let s = start_with_scenarios(TTL_SCENARIOS).await;
    s.activate_raw("slow", r#"{"ttl":"300ms"}"#).await;
    assert_eq!(
        s.put_chaos("db", json!({ "down": true })).await.status(),
        200
    );
    assert!(until(|| async { s.active_names().await == json!([]) }).await);
    assert_eq!(s.live().await["nodes"]["db"]["down"], true);
    assert_eq!(s.status("/users/1").await, 503);
}

#[tokio::test]
async fn list_shows_expires_in_ms() {
    let s = start_with_scenarios(TTL_SCENARIOS).await;
    s.activate_raw("db-down", r#"{"ttl":"1h"}"#).await;
    let list: Value = s.ctl("/__kaanal/scenarios").await.json().await.unwrap();
    let get = |n: &str| {
        list.as_array()
            .unwrap()
            .iter()
            .find(|x| x["name"] == n)
            .unwrap()
            .clone()
    };
    let ms = get("db-down")["expires_in_ms"].as_u64().unwrap();
    assert!((3_599_000..=3_600_000).contains(&ms), "{ms}");
    assert!(get("slow").get("expires_in_ms").is_none());
}
