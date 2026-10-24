//! Activation stacks over live chaos, the snapshot rules, races, and reset.

use crate::common::*;
use serde_json::{Value, json};
use std::time::Duration;

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

pub(crate) async fn run(s: &Sc) -> Vec<u16> {
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
