//! `POST /sim/run` with `source: "draft"`: simulate the draft without Apply.

use crate::common::*;
use serde_json::{Value, json};
use std::time::Duration;

const PEAK: fn() -> Value =
    || json!({ "rps": 100, "journeys": [ { "endpoint": "GET /users/{id}" } ] });

async fn post(s: &Srv, body: Value) -> reqwest::Response {
    s.send(reqwest::Method::POST, "/__kaanal/sim/run", Some(body))
        .await
}

async fn finished(s: &Srv, id: &str) -> Value {
    for _ in 0..1200 {
        let v: Value = s
            .get(&format!("/__kaanal/sim/runs/{id}"))
            .await
            .json()
            .await
            .unwrap();
        if v["state"] != "running" {
            return v;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("run {id} did not finish");
}

/// Runs `extra` merged over an inline PEAK profile on the draft; returns the finished record.
async fn draft_run(s: &Srv, extra: Value) -> Value {
    let mut body = json!({ "profile": PEAK(), "duration_s": 10, "source": "draft" });
    body.as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    let r = post(s, body).await;
    assert_eq!(r.status(), 202, "{}", r.text().await.unwrap());
    let loc = r.headers()["location"].to_str().unwrap().to_string();
    let v: Value = r.json().await.unwrap();
    assert_eq!(
        loc,
        format!("/__kaanal/sim/runs/{}", v["id"].as_str().unwrap())
    );
    finished(s, v["id"].as_str().unwrap()).await
}

fn with_cache() -> String {
    format!(
        "{}  - {{ from: app, to: cache, net: 1ms }}\n",
        system("1ms").replace("  db:", "  cache: { type: service, proc: 1ms }\n  db:")
    )
}

#[tokio::test]
async fn a_draft_run_simulates_the_draft_and_leaves_the_applied_system() {
    let s = start(&system("1ms")).await;
    // a mock hit before the run
    assert_eq!(
        s.client
            .get(format!("{}/users/1", s.base))
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    s.put("/__kaanal/draft", json!({ "text": with_cache() }))
        .await;
    let done = draft_run(&s, json!({})).await;
    assert_eq!(done["state"], "done", "{done}");
    let names: Vec<&str> = done["summary"]["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["name"].as_str().unwrap())
        .collect();
    assert!(
        names.contains(&"cache") && names.contains(&"db"),
        "{names:?}"
    );
    let topo: Value = s.get("/__kaanal/topology").await.json().await.unwrap();
    assert_eq!(topo["nodes"].as_array().unwrap().len(), 3, "{topo}");
    let d: Value = s.get("/__kaanal/draft").await.json().await.unwrap();
    assert_eq!(d["generation"], 1, "no Apply happened");
    assert_eq!(d["dirty"], true);
    // the mocks keep serving the applied system
    assert_eq!(
        s.client
            .get(format!("{}/users/1", s.base))
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
}

#[tokio::test]
async fn a_draft_that_does_not_build_is_a_400_with_a_diagnostic() {
    let s = start(&system("1ms")).await;
    for text in ["version: 1\nnodes: {}\n", "nodes: ["] {
        s.put("/__kaanal/draft", json!({ "text": text })).await;
        let r = post(
            &s,
            json!({ "profile": PEAK(), "duration_s": 5, "source": "draft" }),
        )
        .await;
        assert_eq!(r.status(), 400, "{text}");
        let v: Value = r.json().await.unwrap();
        assert!(v["diagnostic"]["message"].is_string(), "{v}");
        assert!(v["error"].is_string(), "{v}");
    }
    // the slot was never taken
    let applied = post(
        &s,
        json!({ "profile": PEAK(), "duration_s": 5, "source": "applied" }),
    )
    .await;
    assert_eq!(applied.status(), 202);
}

#[tokio::test]
async fn an_unknown_source_is_a_400() {
    let s = start(&system("1ms")).await;
    let r = post(&s, json!({ "profile": PEAK(), "source": "nope" })).await;
    assert_eq!(r.status(), 400);
    assert!(
        r.text()
            .await
            .unwrap()
            .contains("source must be applied or draft")
    );
}

#[tokio::test]
async fn a_draft_run_carries_live_chaos_by_name() {
    let s = start(&system("1ms")).await;
    let r = s
        .put("/__kaanal/chaos/db", json!({ "latency_ms": 50 }))
        .await;
    assert_eq!(r.status(), 200);
    // the draft keeps `db`
    s.put(
        "/__kaanal/draft",
        json!({ "text": system("1ms").replace("proc: 1ms }\n  db", "proc: 2ms }\n  db") }),
    )
    .await;
    let kept = draft_run(&s, json!({})).await;
    assert_eq!(kept["state"], "done", "{kept}");
    let p50 = kept["summary"]["latency"]["p50_ms"].as_f64().unwrap();
    assert!(p50 > 40.0, "chaos carried: p50 {p50}");
    // the draft renames `db` to `db2`: the chaos has nothing to attach to
    let renamed = system("1ms")
        .replace("db:    {", "db2:   {")
        .replace("to: db,", "to: db2,");
    assert_ne!(renamed, system("1ms"));
    s.put("/__kaanal/draft", json!({ "text": renamed })).await;
    let gone = draft_run(&s, json!({})).await;
    assert_eq!(gone["state"], "done", "{gone}");
    let p50 = gone["summary"]["latency"]["p50_ms"].as_f64().unwrap();
    assert!(p50 < 20.0, "chaos dropped: p50 {p50}");
    // a scenario the applied system does not define is a 404
    let r = post(
        &s,
        json!({ "profile": PEAK(), "source": "draft", "scenario": "nope" }),
    )
    .await;
    assert_eq!(r.status(), 404);
}

#[tokio::test]
async fn the_run_record_says_it_was_a_draft_run_and_which_revision() {
    let s = start(&system("1ms")).await;
    let put: Value = s
        .put("/__kaanal/draft", json!({ "text": with_cache() }))
        .await
        .json()
        .await
        .unwrap();
    let rev = put["rev"].as_u64().expect("rev in the view");
    let get: Value = s.get("/__kaanal/draft").await.json().await.unwrap();
    assert_eq!(get["rev"], rev);
    let done = draft_run(&s, json!({})).await;
    assert_eq!(done["source"], "draft");
    assert_eq!(done["draft_rev"], rev);
    s.put("/__kaanal/draft", json!({ "text": system("1ms") }))
        .await;
    let again: Value = s
        .get(&format!(
            "/__kaanal/sim/runs/{}",
            done["id"].as_str().unwrap()
        ))
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(again["draft_rev"], rev, "a later edit does not change it");
    let applied = post(&s, json!({ "profile": PEAK(), "duration_s": 5 })).await;
    let applied = finished(
        &s,
        applied.json::<Value>().await.unwrap()["id"]
            .as_str()
            .unwrap(),
    )
    .await;
    assert_eq!(applied["source"], "applied");
    assert_eq!(applied["draft_rev"], Value::Null);
}

#[tokio::test]
async fn a_second_draft_run_while_one_runs_is_a_409() {
    let s = start(&system("1ms")).await;
    let big = json!({ "rps": 1000, "journeys": [ { "endpoint": "GET /users/{id}" } ] });
    let first: Value = post(
        &s,
        json!({ "profile": big, "duration_s": 300, "warmup_s": 1, "source": "draft" }),
    )
    .await
    .json()
    .await
    .unwrap();
    let id = first["id"].as_str().expect("202 with an id");
    let r = post(
        &s,
        json!({ "profile": PEAK(), "duration_s": 5, "source": "draft" }),
    )
    .await;
    assert_eq!(r.status(), 409);
    assert_eq!(r.json::<Value>().await.unwrap()["id"], id);
    assert_eq!(finished(&s, id).await["state"], "done");
}

/// Stress, not deterministic: an Apply landing between a draft run's reads of the control state
/// and the generation used to pair ids of one generation with the topology of another (a wrong
/// chaos, or a panic and a 500). Whatever the interleaving, the answer is never a 5xx.
#[tokio::test]
async fn a_draft_run_racing_an_apply_is_never_a_500() {
    let s = start(&system("1ms")).await;
    s.put("/__kaanal/chaos/db", json!({ "latency_ms": 5 }))
        .await;
    let shapes = [
        with_cache(),
        system("1ms"),
        system("1ms")
            .replace("db:    {", "db2:   {")
            .replace("to: db,", "to: db2,"),
    ];
    for i in 0..30 {
        s.put("/__kaanal/draft", json!({ "text": shapes[i % 3] }))
            .await;
        let apply = s.send(reqwest::Method::POST, "/__kaanal/draft/apply", None);
        let run = post(
            &s,
            json!({ "profile": PEAK(), "duration_s": 1, "source": "draft" }),
        );
        let (a, r) = tokio::join!(apply, run);
        assert!(a.status().as_u16() < 500, "apply {}", a.status());
        let st = r.status().as_u16();
        assert!(st < 500, "draft run answered {st}");
        if st == 202 {
            let id = r.json::<Value>().await.unwrap()["id"]
                .as_str()
                .unwrap()
                .to_string();
            assert_ne!(finished(&s, &id).await["state"], "running");
        }
    }
}

/// The first thing a user does after "Start blank": type a system, press Run. No Apply has
/// happened (generation 0), so the run's inputs are the draft alone, scenarios included.
#[tokio::test]
async fn a_draft_run_on_a_blank_start_needs_no_apply() {
    let s = start_blank().await;
    let text = format!(
        "{}scenarios:\n  slow-db: {{ chaos: [ {{ node: db, latency_ms: 200 }} ] }}\n",
        kaanal_server::STUDIO_DEMO_YAML
    );
    s.put("/__kaanal/draft", json!({ "text": text })).await;
    let done = draft_run(&s, json!({ "profile": "peak" })).await;
    assert_eq!(done["state"], "done", "{done}");
    assert_eq!(done["source"], "draft");
    assert_eq!(done["generation"], 0, "no Apply happened");
    assert_ne!(done["summary"]["nodes"].as_array().unwrap().len(), 0);
    let p50 = |v: &Value| v["summary"]["latency"]["p50_ms"].as_f64().unwrap();
    // a scenario that only the draft's own text defines resolves
    let slow = draft_run(&s, json!({ "profile": "peak", "scenario": "slow-db" })).await;
    assert_eq!(slow["state"], "done", "{slow}");
    assert!(p50(&slow) > p50(&done), "{} vs {}", p50(&slow), p50(&done));
    // a scenario that only the draft's own text defines resolves
    let slow = draft_run(&s, json!({ "profile": "peak", "scenario": "slow-db" })).await;
    assert_eq!(slow["state"], "done", "{slow}");
    assert!(p50(&slow) > p50(&done), "{} vs {}", p50(&slow), p50(&done));
    // an unknown one is a 404
    let r = post(
        &s,
        json!({ "profile": "peak", "source": "draft", "scenario": "nope" }),
    )
    .await;
    assert_eq!(r.status(), 404);
}

#[tokio::test]
async fn a_draft_run_has_a_log_of_the_draft_system() {
    let s = start(&system("1ms")).await;
    s.put("/__kaanal/draft", json!({ "text": with_cache() }))
        .await;
    let done = draft_run(&s, json!({})).await;
    let id = done["id"].as_str().unwrap();
    let r = s
        .get(&format!("/__kaanal/sim/runs/{id}/log?node=cache&limit=5"))
        .await;
    assert_eq!(r.status(), 200);
    let v: Value = r.json().await.unwrap();
    // `cache` exists only in the draft: its lines prove the log is the draft run's
    let lines = v["lines"].as_array().unwrap();
    assert!(lines.iter().all(|l| l["node"] == "cache"), "{v}");
}
