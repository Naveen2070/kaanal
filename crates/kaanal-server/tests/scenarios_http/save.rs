//! `scenarios/save`: sparse layers, overwrite refusal, `from` sources, leased-field warnings.

use crate::common::*;
use serde_json::{Value, json};

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

#[tokio::test]
async fn save_from_live_warns_that_leased_fields_become_permanent() {
    let s = start_with_scenarios("").await;
    s.put_chaos("db", json!({ "latency_ms": 100 })).await;
    let r: Value = s
        .post_json("/__kaanal/scenarios/save", json!({ "name": "plain" }))
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(r["warnings"], json!([]), "{r}");
    // Fields, not units: `down` 1, `error_rate` + `error_kind` 2, `mode` none (a file has no mode).
    s.put_chaos(
        "db",
        json!({ "down": true, "error_rate": 0.1, "error_kind": "reset", "mode": "merge", "ttl": "1h" }),
    )
    .await;
    let r: Value = s
        .post_json("/__kaanal/scenarios/save", json!({ "name": "timed" }))
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(
        r["warnings"],
        json!(["3 fields have a ttl; the file has none"]),
        "{r}"
    );
    let text = std::fs::read_to_string(s.dir.join("scenarios/timed.yaml")).unwrap();
    assert!(text.contains("down") && !text.contains("ttl"), "{text}");
}

#[tokio::test]
async fn save_from_effective_warns_only_when_it_includes_leased_chaos() {
    let s = start_with_scenarios("").await;
    s.put_chaos("db", json!({ "latency_ms": 100 })).await;
    let r: Value = s
        .post_json(
            "/__kaanal/scenarios/save",
            json!({ "name": "eff-plain", "from": "effective" }),
        )
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(r["warnings"], json!([]), "{r}");
    s.put_chaos("db", json!({ "down": true, "ttl": "1h" }))
        .await;
    let r: Value = s
        .post_json(
            "/__kaanal/scenarios/save",
            json!({ "name": "eff-timed", "from": "effective" }),
        )
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(
        r["warnings"],
        json!(["1 field has a ttl; the file has none"]),
        "{r}"
    );
}

#[tokio::test]
async fn ttl_without_a_field_to_lease_is_400() {
    let s = start_with_scenarios("").await;
    s.put_chaos("db", json!({ "latency_ms": 10 })).await;
    let before = s.live().await;
    for body in [
        json!({ "ttl": "1m" }),
        json!({ "mode": "merge", "ttl": "1m" }),
    ] {
        let r = s.put_chaos("db", body.clone()).await;
        assert_eq!(r.status(), 400, "{body}");
        let b: Value = r.json().await.unwrap();
        assert_eq!(b["error"], "ttl needs a field to lease", "{b}");
        let mut keyed = body.clone();
        keyed["node"] = json!("db");
        let r = s.put("/__kaanal/chaos", keyed).await;
        assert_eq!(r.status(), 400, "{body}");
    }
    assert_eq!(s.live().await, before);
    // A leased field (or an override mode) alongside the ttl is fine.
    for body in [
        json!({ "latency_ms": 5, "ttl": "1m" }),
        json!({ "mode": "override", "ttl": "1m" }),
    ] {
        assert_eq!(s.put_chaos("db", body).await.status(), 200);
    }
}

#[tokio::test]
async fn a_saved_replica_entry_round_trips() {
    let yaml = BASE.replace(
        "db:    { type: database,",
        "db:    { type: database, replicas: 3,",
    );
    let s = start_full(new_dir(&[]), &yaml).await;
    let r = s
        .put(
            "/__kaanal/chaos",
            json!({ "node": "db", "replica": 1, "down": true }),
        )
        .await;
    assert_eq!(r.status(), 200);
    let r = s
        .post_json("/__kaanal/scenarios/save", json!({ "name": "r1" }))
        .await;
    assert_eq!(r.status(), 200);
    let text = std::fs::read_to_string(s.dir.join("scenarios/r1.yaml")).unwrap();
    assert!(
        text.contains(r#""replica":1"#) && text.contains(r#""down":true"#),
        "{text}"
    );
    // the saved file loads: a fresh server with the file in place resolves its replica entry
    let dir = new_dir(&[("r1.yaml", &text)]);
    let t = start_full(dir, &yaml).await;
    let active = t.post("/__kaanal/scenarios/r1/activate").await;
    assert_eq!(active.status(), 200, "{}", active.text().await.unwrap());
    let eff = t.active().await;
    assert_eq!(eff["effective"]["replicas"][0]["replica"], 1);
}

#[tokio::test]
async fn scenario_save_round_trips_an_edge_entry() {
    let s = start_full(new_dir(&[]), BASE).await;
    let body = json!({ "edge": { "from": "app", "to": "db" }, "latency_ms": 20, "jitter_ms": 4, "error_rate": 0.1 });
    assert_eq!(s.put("/__kaanal/chaos", body).await.status(), 200);
    let r = s
        .post_json("/__kaanal/scenarios/save", json!({ "name": "e1" }))
        .await;
    assert_eq!(r.status(), 200);
    let text = std::fs::read_to_string(s.dir.join("scenarios/e1.yaml")).unwrap();
    assert!(
        text.contains(r#""edge":{"from":"app","to":"db"}"#) && text.contains(r#""jitter_ms":4"#),
        "{text}"
    );
    // the saved file loads and resolves to the same edge entry
    let t = start_full(new_dir(&[("e1.yaml", &text)]), BASE).await;
    assert_eq!(
        t.post("/__kaanal/scenarios/e1/activate").await.status(),
        200
    );
    let e = &t.active().await["effective"]["edges"][0];
    assert_eq!(e["edge"]["to"], "db");
    assert_eq!(e["jitter_ms"], 4);
    assert_eq!(e["latency_ms"], 20);
}
