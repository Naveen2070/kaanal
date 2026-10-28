//! Unit tests for the control API handlers and guard.

use super::{
    ChaosPatch, activate, clear_node, ct_eq, expire_due, host_allowed, lease_task, lock, lock_gen,
    locked, next_deadline, query_token, scenario_data, set_chaos,
};
use crate::chaos_lease::Unit;
use crate::generation::Gen;
use crate::state::{AppState, BusEvent};
use axum::Json;
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use kaanal_core::{ChaosKey, ChaosLayer, Config};
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;

fn state() -> Arc<AppState> {
    let yaml = "version: 1\nnodes:\n  gw: { type: gateway }\n  db: { type: database, flavor: postgres }\n  app: { type: service }\nedges:\n  - { from: gw, to: app }\n  - { from: app, to: db }\nscenarios:\n  a: { chaos: [ { node: db, latency_ms: 1 } ] }\n  b: { chaos: [ { node: db, latency_ms: 2 } ] }\n";
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
    let st = AppState::build(&Config::from_yaml(yaml).unwrap(), &dir, String::new(), 0)
        .map_err(|e| e.to_string())
        .unwrap();
    Arc::new(st)
}

async fn lease(st: &Arc<AppState>, name: &str, ttl: &str) {
    let body = Bytes::from(format!(r#"{{"ttl":"{ttl}"}}"#));
    let _ = activate(
        State(Arc::clone(st)),
        Gen(st.generation().unwrap()),
        Path(name.into()),
        body,
    )
    .await
    .unwrap();
}

async fn settle() {
    for _ in 0..5 {
        tokio::task::yield_now().await;
    }
}

fn expiries(rx: &mut tokio::sync::broadcast::Receiver<BusEvent>) -> Vec<String> {
    let mut out = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        if let BusEvent::Scenario {
            action: "expire",
            name: Some(n),
            ..
        } = ev
        {
            out.push(n);
        }
    }
    out
}

#[tokio::test(start_paused = true)]
async fn lease_task_expires_on_the_paused_clock() {
    let st = state();
    let mut rx = st.events.subscribe();
    tokio::spawn(lease_task(Arc::clone(&st)));
    lease(&st, "a", "5s").await;
    tokio::time::advance(Duration::from_secs(4)).await;
    settle().await;
    assert_eq!(st.snapshot().stack.active(), ["a"]);
    tokio::time::advance(Duration::from_secs(2)).await;
    settle().await;
    assert_eq!(st.snapshot().stack.active(), [] as [String; 0]);
    assert_eq!(expiries(&mut rx), ["a"]);
}

#[tokio::test(start_paused = true)]
async fn lease_survives_a_poisoned_lock() {
    let st = state();
    let s2 = Arc::clone(&st);
    let _ = std::thread::spawn(move || {
        let _g = s2.control.lock().unwrap();
        panic!("poison");
    })
    .join();
    assert!(st.control.is_poisoned());
    tokio::spawn(lease_task(Arc::clone(&st)));
    lease(&st, "a", "1s").await;
    tokio::time::advance(Duration::from_secs(2)).await;
    settle().await;
    assert_eq!(st.snapshot().stack.active(), [] as [String; 0]);
    assert!(lock(&st).leases.is_empty());
}

#[tokio::test(start_paused = true)]
async fn two_leases_expire_in_order_with_one_event_each() {
    let st = state();
    let mut rx = st.events.subscribe();
    tokio::spawn(lease_task(Arc::clone(&st)));
    lease(&st, "b", "2s").await;
    lease(&st, "a", "1s").await;
    tokio::time::advance(Duration::from_millis(1500)).await;
    settle().await;
    assert_eq!(st.snapshot().stack.active(), ["b"]);
    assert_eq!(expiries(&mut rx), ["a"]);
    tokio::time::advance(Duration::from_secs(1)).await;
    settle().await;
    assert_eq!(st.snapshot().stack.active(), [] as [String; 0]);
    assert_eq!(expiries(&mut rx), ["b"]);
    assert_eq!(expire_due(&st), 0);
}

#[tokio::test(start_paused = true)]
async fn two_leases_due_in_one_call_expire_in_order() {
    let st = state();
    let mut rx = st.events.subscribe();
    lease(&st, "b", "1s").await;
    lease(&st, "a", "1s").await;
    tokio::time::advance(Duration::from_secs(3)).await;
    assert_eq!(expire_due(&st), 2);
    assert_eq!(st.snapshot().stack.active(), [] as [String; 0]);
    assert_eq!(expiries(&mut rx), ["a", "b"]);
}

async fn put(st: &Arc<AppState>, node: &str, body: serde_json::Value) {
    let p: ChaosPatch = serde_json::from_value(body).unwrap();
    let _ = set_chaos(
        State(Arc::clone(st)),
        Gen(st.generation().unwrap()),
        Path(node.into()),
        Ok(Json(p)),
    )
    .await
    .unwrap();
}

fn node(st: &AppState, name: &str) -> ChaosKey {
    ChaosKey::Node(st.generation().unwrap().topo.id(name).unwrap())
}

/// The live entry of `db` in Control and in the published snapshot; they must agree.
fn db_live(st: &AppState) -> Option<ChaosLayer> {
    let k = node(st, "db");
    let snap = st.snapshot().stack.live().layer(&k).cloned();
    assert_eq!(lock(st).stack.live().layer(&k).cloned(), snap);
    snap
}

fn chaos_expiries(rx: &mut tokio::sync::broadcast::Receiver<BusEvent>) -> Vec<serde_json::Value> {
    let mut out = Vec::new();
    while let Ok(ev) = rx.try_recv() {
        if let BusEvent::Scenario {
            action: "expire",
            name,
            chaos_key: Some(k),
            ..
        } = ev
        {
            assert_eq!(name, None);
            out.push(k);
        }
    }
    out
}

#[tokio::test(start_paused = true)]
async fn chaos_ttl_expires_and_clears_only_its_unit() {
    let st = state();
    tokio::spawn(lease_task(Arc::clone(&st)));
    put(&st, "db", json!({ "latency_ms": 200 })).await;
    put(&st, "db", json!({ "down": true, "ttl": "30s" })).await;
    tokio::time::advance(Duration::from_secs(29)).await;
    settle().await;
    assert_eq!(db_live(&st).unwrap().down, Some(true));
    tokio::time::advance(Duration::from_secs(2)).await;
    settle().await;
    let l = db_live(&st).unwrap();
    assert_eq!(l.down, None);
    assert_eq!(l.latency_add, Some(Duration::from_millis(200)));
    let snap = st.snapshot();
    let r = snap
        .chaos
        .visit(st.generation().unwrap().topo.id("db").unwrap(), "GET /x")
        .unwrap();
    assert!(!r.down);
    assert_eq!(r.latency_add, Duration::from_millis(200));
    assert_eq!(snap.chaos_leases.len(), 0);
    assert_eq!(next_deadline(&lock(&st)), None);
}

#[tokio::test(start_paused = true)]
async fn expiry_removes_an_entry_left_empty() {
    let st = state();
    put(
        &st,
        "db",
        json!({ "down": true, "mode": "override", "ttl": "1s" }),
    )
    .await;
    tokio::time::advance(Duration::from_secs(2)).await;
    assert_eq!(expire_due(&st), 1);
    assert_eq!(db_live(&st), None);
    assert!(st.snapshot().chaos.is_empty());
}

#[tokio::test(start_paused = true)]
async fn a_24h_scenario_lease_does_not_delay_a_1s_chaos_lease() {
    let st = state();
    let mut rx = st.events.subscribe();
    tokio::spawn(lease_task(Arc::clone(&st)));
    lease(&st, "a", "24h").await;
    settle().await; // the task now sleeps toward the 24 h deadline
    put(&st, "db", json!({ "down": true, "ttl": "1s" })).await;
    settle().await;
    tokio::time::advance(Duration::from_millis(1001)).await;
    settle().await;
    assert_eq!(db_live(&st).and_then(|l| l.down), None);
    assert_eq!(chaos_expiries(&mut rx), [json!({ "node": "db" })]);
    assert_eq!(st.snapshot().stack.active(), ["a"]);
}

#[tokio::test(start_paused = true)]
async fn a_reput_just_before_the_deadline_wins() {
    let st = state();
    tokio::spawn(lease_task(Arc::clone(&st)));
    put(&st, "db", json!({ "down": true, "ttl": "30s" })).await;
    tokio::time::advance(Duration::from_secs(29)).await;
    settle().await;
    put(&st, "db", json!({ "down": true, "ttl": "30s" })).await;
    tokio::time::advance(Duration::from_secs(2)).await;
    settle().await;
    assert_eq!(db_live(&st).unwrap().down, Some(true));
    // Compared under the lock: a due wake-up finds the replaced deadline and does nothing.
    assert_eq!(expire_due(&st), 0);
    tokio::time::advance(Duration::from_secs(27)).await;
    settle().await;
    assert_eq!(db_live(&st).unwrap().down, Some(true));
    tokio::time::advance(Duration::from_secs(2)).await;
    settle().await;
    assert_eq!(db_live(&st), None);
}

#[tokio::test(start_paused = true)]
async fn plain_put_just_before_the_deadline_makes_it_permanent() {
    let st = state();
    let mut rx = st.events.subscribe();
    tokio::spawn(lease_task(Arc::clone(&st)));
    put(&st, "db", json!({ "down": true, "ttl": "30s" })).await;
    tokio::time::advance(Duration::from_secs(29)).await;
    settle().await;
    put(&st, "db", json!({ "down": true })).await;
    tokio::time::advance(Duration::from_secs(60)).await;
    settle().await;
    assert_eq!(db_live(&st).unwrap().down, Some(true));
    assert_eq!(next_deadline(&lock(&st)), None);
    assert_eq!(chaos_expiries(&mut rx), [] as [serde_json::Value; 0]);
}

#[tokio::test(start_paused = true)]
async fn expiry_after_the_entry_was_deleted_emits_nothing_and_does_not_spin() {
    let st = state();
    let mut rx = st.events.subscribe();
    tokio::spawn(lease_task(Arc::clone(&st)));
    put(&st, "db", json!({ "down": true, "ttl": "1s" })).await;
    let _ = clear_node(
        State(Arc::clone(&st)),
        Gen(st.generation().unwrap()),
        Path("db".into()),
    )
    .await
    .unwrap();
    tokio::time::advance(Duration::from_secs(2)).await;
    settle().await;
    assert_eq!(next_deadline(&lock(&st)), None);
    assert_eq!(expire_due(&st), 0);
    assert_eq!(chaos_expiries(&mut rx), [] as [serde_json::Value; 0]);
    // Even a stale lease (no entry behind it) is dropped, silently.
    lock(&st).chaos_leases.insert(
        (node(&st, "db"), Unit::Down),
        tokio::time::Instant::now() - Duration::from_secs(1),
    );
    assert_eq!(expire_due(&st), 0);
    assert_eq!(next_deadline(&lock(&st)), None);
    assert_eq!(chaos_expiries(&mut rx), [] as [serde_json::Value; 0]);
}

#[tokio::test(start_paused = true)]
async fn expire_event_names_the_chaos_key() {
    let st = state();
    let mut rx = st.events.subscribe();
    put(&st, "db", json!({ "latency_ms": 5, "ttl": "1s" })).await;
    tokio::time::advance(Duration::from_secs(2)).await;
    assert_eq!(expire_due(&st), 1);
    let Ok(BusEvent::Scenario {
        action,
        name,
        chaos_key,
        active,
    }) = rx.try_recv()
    else {
        panic!("no scenario event");
    };
    assert_eq!((action, name.as_deref()), ("expire", None));
    assert_eq!(chaos_key, Some(json!({ "node": "db" })));
    assert_eq!(
        scenario_data(action, name.as_deref(), chaos_key.as_ref(), &active),
        json!({ "action": "expire", "name": null, "chaos_key": { "node": "db" }, "active": [] })
    );
    // An endpoint and a node@endpoint key name their parts the way `PUT /chaos` does.
    // (Stored directly: the test topology has no routes for a keyed PUT.)
    let id = st.generation().unwrap().topo.id("db").unwrap();
    let past = tokio::time::Instant::now() - Duration::from_secs(1);
    {
        let mut c = lock(&st);
        for (i, k) in [
            ChaosKey::Endpoint("GET /x".into()),
            ChaosKey::NodeAt(id, "GET /x".into()),
        ]
        .into_iter()
        .enumerate()
        {
            let down = ChaosLayer {
                down: Some(true),
                ..ChaosLayer::default()
            };
            c.stack.live_mut().set(k.clone(), down);
            let d = past + Duration::from_millis(u64::try_from(i).unwrap());
            c.chaos_leases.insert((k, Unit::Down), d);
        }
    }
    assert_eq!(expire_due(&st), 2);
    assert_eq!(
        chaos_expiries(&mut rx),
        [
            json!({ "endpoint": "GET /x" }),
            json!({ "node": "db", "endpoint": "GET /x" })
        ]
    );
    // A scenario event carries no `chaos_key` at all.
    assert_eq!(
        scenario_data("activate", Some("a"), None, &["a".into()]),
        json!({ "action": "activate", "name": "a", "active": ["a"] })
    );
}

#[tokio::test(start_paused = true)]
async fn two_units_of_one_key_expiring_together_publish_once_and_emit_once() {
    let st = state();
    let mut rx = st.events.subscribe();
    put(
        &st,
        "db",
        json!({ "latency_ms": 5, "down": true, "ttl": "1s" }),
    )
    .await;
    put(&st, "app", json!({ "down": true, "ttl": "1s" })).await;
    tokio::time::advance(Duration::from_secs(2)).await;
    // One commit (one publish) per key.
    assert_eq!(expire_due(&st), 2);
    let mut keys = chaos_expiries(&mut rx);
    keys.sort_by_key(ToString::to_string);
    assert_eq!(keys, [json!({ "node": "app" }), json!({ "node": "db" })]);
    assert_eq!(db_live(&st), None);
    assert_eq!(next_deadline(&lock(&st)), None);
}

#[tokio::test(start_paused = true)]
async fn failed_expiry_drops_the_lease_and_keeps_the_chaos() {
    let st = state();
    let mut rx = st.events.subscribe();
    put(&st, "db", json!({ "down": true, "ttl": "1s" })).await;
    // Inject a failure: an out-of-range entry stored without a publish makes every
    // recompile fail, as a broken invariant would.
    lock(&st).stack.live_mut().set(
        node(&st, "app"),
        ChaosLayer {
            error_rate: Some(2.0),
            ..ChaosLayer::default()
        },
    );
    tokio::time::advance(Duration::from_secs(2)).await;
    assert_eq!(expire_due(&st), 0);
    assert_eq!(db_live(&st).unwrap().down, Some(true));
    assert_eq!(
        next_deadline(&lock(&st)),
        None,
        "a kept past-due lease would spin"
    );
    assert_eq!(chaos_expiries(&mut rx), [] as [serde_json::Value; 0]);
}

#[test]
fn hosts() {
    assert!(host_allowed("localhost:8080", 8080));
    assert!(host_allowed("127.0.0.1:8080", 8080));
    assert!(host_allowed("[::1]:8080", 8080));
    assert!(host_allowed("localhost", 8080));
    assert!(host_allowed("LocalHost:8080", 8080));
    assert!(!host_allowed("localhost:9999", 8080));
    assert!(!host_allowed("evil.example", 8080));
    assert!(!host_allowed("localhost.evil.example:8080", 8080));
    assert!(!host_allowed("evil.example:8080@localhost", 8080));
    assert!(!host_allowed("[::1", 8080));
    assert!(!host_allowed("", 8080));
}

#[test]
fn constant_time_eq() {
    assert!(ct_eq(b"abc", b"abc"));
    assert!(!ct_eq(b"abc", b"abd"));
    assert!(!ct_eq(b"abc", b"ab"));
}

#[test]
fn token_comes_only_from_the_token_parameter() {
    assert_eq!(query_token(Some("token=abc")), Some("abc"));
    assert_eq!(query_token(Some("a=1&token=abc&b=2")), Some("abc"));
    assert_eq!(query_token(Some("xtoken=abc")), None);
    assert_eq!(query_token(Some("token")), None);
    assert_eq!(query_token(None), None);
}

#[tokio::test]
async fn a_due_chaos_lease_is_dropped_even_with_no_generation() {
    let st = state();
    let past = tokio::time::Instant::now() - Duration::from_secs(1);
    lock(&st)
        .chaos_leases
        .insert((ChaosKey::Node(0), Unit::Down), past);
    st.set_generation(None);
    assert_eq!(expire_due(&st), 0, "no key to name, so no event");
    assert!(
        lock(&st).chaos_leases.is_empty(),
        "a past-due lease left behind would make the lease task spin"
    );
}

#[tokio::test]
async fn a_stale_generation_is_refused_under_the_control_lock() {
    let st = state();
    let old = st.generation().unwrap();
    assert!(
        lock_gen(&st, &old).is_ok(),
        "the generation in force is accepted"
    );
    let cfg = Config::from_yaml(
        "version: 1\nnodes:\n  gw: { type: gateway }\n  app: { type: service }\nedges:\n  - { from: gw, to: app }\n",
    )
    .unwrap();
    let (mut next, _) = crate::generation::Generation::build(
        &cfg,
        std::path::Path::new("."),
        None,
        crate::generation::Trust::Owner,
    )
    .unwrap();
    next.id = 2;
    st.install(next);
    let err = lock_gen(&st, &old)
        .err()
        .expect("the old generation is stale");
    assert_eq!(err.0, StatusCode::CONFLICT);
    assert!(
        err.1.0["error"]
            .as_str()
            .unwrap()
            .contains("system changed")
    );
    assert!(lock_gen(&st, &st.generation().unwrap()).is_ok());
}

#[tokio::test]
async fn locked_pairs_the_lock_with_the_generation_in_force_and_never_refuses_a_swap() {
    let st = state();
    let first = st.generation().unwrap();
    assert_eq!(locked(&st).unwrap().1.id, first.id);
    let cfg = Config::from_yaml(
        "version: 1\nnodes:\n  gw: { type: gateway }\n  app: { type: service }\nedges:\n  - { from: gw, to: app }\n",
    )
    .unwrap();
    let (mut next, _) = crate::generation::Generation::build(
        &cfg,
        std::path::Path::new("."),
        None,
        crate::generation::Trust::Owner,
    )
    .unwrap();
    next.id = 2;
    st.install(next);
    // a reader that came after the swap just gets the new one: no 409 for a swap
    assert_eq!(locked(&st).unwrap().1.id, 2);
    st.set_generation(None);
    let err = locked(&st).err().expect("no system");
    assert_eq!(err.0, StatusCode::CONFLICT);
}
