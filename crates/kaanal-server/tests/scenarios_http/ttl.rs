//! Activation and chaos `ttl` leases: expiry, re-activation, bounds, `down` handling.

use crate::common::*;
use reqwest::Method;
use serde_json::{Value, json};
use std::time::Duration;

pub(crate) const TTL_SCENARIOS: &str = "scenarios:
  db-down: { chaos: [ { node: db, down: true } ] }
  slow: { chaos: [ { node: db, latency_ms: 1 } ] }
";

impl Sc {
    pub(crate) async fn activate_raw(&self, name: &str, body: &str) -> reqwest::Response {
        self.client
            .post(format!("{}/__kaanal/scenarios/{name}/activate", self.base))
            .bearer_auth(&self.token)
            .header("content-type", "application/json")
            .body(body.to_string())
            .send()
            .await
            .unwrap()
    }
    pub(crate) async fn active_names(&self) -> Value {
        self.active().await["active"].clone()
    }
}

/// Polls `f` for up to 3 s.
pub(crate) async fn until<F: std::future::Future<Output = bool>>(mut f: impl FnMut() -> F) -> bool {
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

#[tokio::test]
async fn put_down_true_then_false_removes_the_entry() {
    let s = start_with_scenarios("").await;
    s.put_chaos("db", json!({ "down": true })).await;
    assert_eq!(s.live().await["nodes"]["db"]["down"], true);
    s.put_chaos("db", json!({ "down": false })).await;
    assert_eq!(s.live().await["nodes"], json!({}));
    // The keyed form shares `apply_patch`.
    let k = json!({ "node": "db", "endpoint": EP });
    let put = |down| {
        let mut b = k.clone();
        b["down"] = json!(down);
        s.put("/__kaanal/chaos", b)
    };
    assert_eq!(put(true).await.status(), 200);
    assert_eq!(s.live().await["node_at"].as_array().unwrap().len(), 1);
    assert_eq!(put(false).await.status(), 200);
    assert_eq!(s.live().await["node_at"], json!([]));
}

#[tokio::test]
async fn down_false_keeps_other_fields() {
    let s = start_with_scenarios("").await;
    s.put_chaos("db", json!({ "latency_ms": 200, "down": true }))
        .await;
    s.put_chaos("db", json!({ "down": false })).await;
    let db = &s.live().await["nodes"]["db"];
    assert_eq!(db["latency_ms"], 200);
    assert_eq!(db["down"], false);
}

#[tokio::test]
async fn down_false_does_not_undo_a_scenarios_down() {
    let s =
        start_with_scenarios("scenarios:\n  db-down: { chaos: [ { node: db, down: true } ] }\n")
            .await;
    s.post("/__kaanal/scenarios/db-down/activate").await;
    s.put_chaos("db", json!({ "down": true })).await;
    let r: Value = s
        .put_chaos("db", json!({ "down": false }))
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(s.live().await["nodes"], json!({}));
    assert_eq!(r["effective"]["nodes"]["db"]["down"], true);
    assert_eq!(s.status("/users/42").await, 503);
}

/// The leased units of the db node's live entry, as `GET /chaos` shows them.
pub(crate) async fn db_expires(s: &Sc) -> Value {
    s.live().await["nodes"]["db"]["expires_in_ms"].clone()
}

#[tokio::test]
async fn put_with_ttl_leases_only_the_units_it_sets() {
    let s = start_with_scenarios("").await;
    let r: Value = s
        .put_chaos("db", json!({ "latency_ms": 200 }))
        .await
        .json()
        .await
        .unwrap();
    assert!(r.get("expires_in_ms").is_none(), "{r}");
    let r = s
        .put_chaos("db", json!({ "down": true, "ttl": "30s" }))
        .await;
    assert_eq!(r.status(), 200);
    let r: Value = r.json().await.unwrap();
    let ms = r["expires_in_ms"]["down"].as_u64().unwrap();
    assert!((29_000..=30_000).contains(&ms), "{r}");
    let exp = db_expires(&s).await;
    assert_eq!(exp.as_object().unwrap().len(), 1, "{exp}");
    assert!((29_000..=30_000).contains(&exp["down"].as_u64().unwrap()));
    assert_eq!(s.live().await["nodes"]["db"]["latency_ms"], 200);
}

#[tokio::test]
async fn plain_put_makes_the_unit_permanent() {
    let s = start_with_scenarios("").await;
    s.put_chaos("db", json!({ "down": true, "ttl": "30s" }))
        .await;
    assert!(db_expires(&s).await["down"].is_u64());
    let r: Value = s
        .put_chaos("db", json!({ "down": true }))
        .await
        .json()
        .await
        .unwrap();
    assert!(r.get("expires_in_ms").is_none(), "{r}");
    assert!(db_expires(&s).await.is_null());
    assert_eq!(s.live().await["nodes"]["db"]["down"], true);
}

#[tokio::test]
async fn reput_with_a_new_ttl_replaces_the_deadline() {
    let s = start_with_scenarios("").await;
    s.put_chaos("db", json!({ "latency_ms": 5, "ttl": "30s" }))
        .await;
    s.put_chaos("db", json!({ "latency_ms": 6, "ttl": "1h" }))
        .await;
    let ms = db_expires(&s).await["latency"].as_u64().unwrap();
    assert!((3_599_000..=3_600_000).contains(&ms), "{ms}");
    s.put_chaos("db", json!({ "latency_ms": 7, "ttl": "1s" }))
        .await;
    let ms = db_expires(&s).await["latency"].as_u64().unwrap();
    assert!(ms <= 1_000, "{ms}");
}

#[tokio::test]
async fn bad_ttl_is_400_and_changes_nothing() {
    let s = start_with_scenarios("").await;
    s.put_chaos("db", json!({ "latency_ms": 10 })).await;
    let before = s.live().await;
    for bad in ["0ms", "25h", "abc"] {
        let r = s
            .put_chaos("db", json!({ "latency_ms": 99, "down": true, "ttl": bad }))
            .await;
        assert_eq!(r.status(), 400, "{bad}");
        let b: Value = r.json().await.unwrap();
        assert!(b["error"].as_str().unwrap().contains("ttl"), "{b}");
        let k = s
            .put(
                "/__kaanal/chaos",
                json!({ "node": "app", "down": true, "ttl": bad }),
            )
            .await;
        assert_eq!(k.status(), 400, "{bad}");
    }
    assert_eq!(s.live().await, before);
}

#[tokio::test]
async fn keyed_put_with_ttl_leases_its_key() {
    let s = start_with_scenarios("").await;
    let r = s
        .put(
            "/__kaanal/chaos",
            json!({ "node": "db", "endpoint": EP, "error_rate": 0.5, "mode": "override", "ttl": "1m" }),
        )
        .await;
    assert_eq!(r.status(), 200);
    let r: Value = r.json().await.unwrap();
    let exp = r["expires_in_ms"].as_object().unwrap();
    let mut units: Vec<&str> = exp.keys().map(String::as_str).collect();
    units.sort_unstable();
    assert_eq!(units, ["error", "mode"]);
    let entry = &s.live().await["node_at"][0];
    assert_eq!(entry["node"], "db");
    assert!(entry["expires_in_ms"]["error"].as_u64().unwrap() <= 60_000);
    // The node's own entry is another key: unleased.
    s.put_chaos("db", json!({ "latency_ms": 1 })).await;
    assert!(db_expires(&s).await.is_null());
}

#[tokio::test]
async fn delete_node_and_reset_drop_leases() {
    // The later PUT sets another unit, so only `commit`'s prune can drop the stale `down` lease.
    let s = start_with_scenarios("").await;
    for (m, path) in [
        (Method::DELETE, "/__kaanal/chaos/db"),
        (Method::DELETE, "/__kaanal/chaos?node=db"),
        (Method::DELETE, "/__kaanal/chaos"),
        (Method::POST, "/__kaanal/reset"),
    ] {
        s.put_chaos("db", json!({ "down": true, "ttl": "1h" }))
            .await;
        assert!(db_expires(&s).await["down"].is_u64(), "{path}");
        assert_eq!(s.send(m, path, None).await.status().as_u16(), 204, "{path}");
        s.put_chaos("db", json!({ "latency_ms": 3 })).await;
        let exp = db_expires(&s).await;
        assert!(exp.is_null(), "{path}: {exp}");
        s.delete("/__kaanal/chaos").await;
    }
}

#[tokio::test]
async fn save_with_clear_live_drops_leases() {
    let s = start_with_scenarios("").await;
    s.put_chaos("db", json!({ "down": true, "ttl": "1h" }))
        .await;
    let r = s
        .post_json(
            "/__kaanal/scenarios/save",
            json!({ "name": "cl", "clear_live": true }),
        )
        .await;
    assert_eq!(r.status(), 200);
    s.put_chaos("db", json!({ "latency_ms": 3 })).await;
    let exp = db_expires(&s).await;
    assert!(exp.is_null(), "{exp}");
}

#[tokio::test]
async fn down_false_with_a_ttl_leaves_no_down_lease() {
    // `down: false` with a ttl would lease `down`; the unset field must not keep it.
    let s = start_with_scenarios("").await;
    s.put_chaos(
        "db",
        json!({ "latency_ms": 200, "down": true, "ttl": "1h" }),
    )
    .await;
    let r: Value = s
        .put_chaos("db", json!({ "down": false, "ttl": "30s" }))
        .await
        .json()
        .await
        .unwrap();
    assert!(r["expires_in_ms"]["latency"].is_u64(), "{r}");
    assert!(r["expires_in_ms"].get("down").is_none(), "{r}");
    let exp = db_expires(&s).await;
    assert!(exp["latency"].is_u64(), "{exp}");
    assert!(exp.get("down").is_none(), "{exp}");
}

#[tokio::test]
async fn down_false_removes_the_down_lease() {
    let s = start_with_scenarios("").await;
    s.put_chaos(
        "db",
        json!({ "latency_ms": 200, "down": true, "ttl": "1h" }),
    )
    .await;
    s.put_chaos("db", json!({ "down": false })).await;
    let exp = db_expires(&s).await;
    assert!(exp["latency"].is_u64(), "{exp}");
    assert!(exp.get("down").is_none(), "{exp}");
    // An entry with only a leased `down`, emptied by `down: false` with a ttl: a later entry on
    // the key (another unit) inherits nothing.
    s.delete("/__kaanal/chaos").await;
    s.put_chaos("db", json!({ "down": true, "ttl": "1h" }))
        .await;
    s.put_chaos("db", json!({ "down": false, "ttl": "30s" }))
        .await;
    assert_eq!(s.live().await["nodes"], json!({}));
    s.put_chaos("db", json!({ "latency_ms": 3 })).await;
    let exp = db_expires(&s).await;
    assert!(exp.is_null(), "{exp}");
}
