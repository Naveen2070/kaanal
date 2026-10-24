//! The scenario API: the active stack, runtime scenarios, events, chaos responses, reset and rate-limit refill.

use crate::common::*;
use serde_json::{Value, json};
use std::time::Duration;

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
            { "name": "inl", "source": "inline", "description": null, "entries": 1, "pins": [], "active": false },
            { "name": "rt", "source": "runtime", "description": "d", "entries": 2, "pins": [], "active": false },
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

pub(crate) const LIMITED: &str = "version: 1
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
