//! `POST /sim/plan` (Step 9c-1): a sizing plan is a run record of kind `plan`, under the same single
//! slot as a run and a sweep. The fixture database is M/M/1 (one slot, every op 10 ms exponential, so
//! mu = 100 req/s a copy): its capacity is c x mu (Erlang C's stability bound, rho = lambda / (c mu) < 1).

use crate::common::*;
use serde_json::{Value, json};
use std::time::Duration;

/// The service rate of one database copy, req/s (10 ms mean per op).
const MU: f64 = 100.0;

/// A gateway, a service with the fixture spec and an M/M/1 database behind it; `peak` is 100 req/s,
/// `big` holds the slot for a while in a debug build.
pub(super) fn config() -> String {
    "version: 1
seed: 42
# a comment the proposed draft must keep
nodes:
  nginx: { type: gateway, proc: 1ms }
  app:   { type: service, spec: openapi.yaml, proc: 1ms }
  db:    { type: database, concurrency: 1, ops: { read: { mean: 10ms }, write: { mean: 10ms }, scan: { mean: 10ms } } }
edges:
  - { from: nginx, to: app, net: 1ms }
  - { from: app, to: db, net: 1ms }
traffic:
  peak:
    rps: 100
    journeys: [ { endpoint: 'GET /users/{id}' } ]
  big:
    rps: 1000
    journeys: [ { endpoint: 'GET /users/{id}' } ]
"
    .to_string()
}

pub(super) async fn plan(s: &Srv, body: Value) -> reqwest::Response {
    s.send(reqwest::Method::POST, "/__kaanal/sim/plan", Some(body))
        .await
}

/// Polls record `id` until it is no longer running.
pub(super) async fn finished(s: &Srv, id: &str) -> Value {
    for _ in 0..4800 {
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
    panic!("record {id} did not finish");
}

/// Posts a plan, asserts the 202, and waits for its record.
pub(super) async fn planned(s: &Srv, body: Value) -> Value {
    let r = plan(s, body).await;
    assert_eq!(r.status(), 202);
    let v: Value = r.json().await.unwrap();
    finished(s, v["id"].as_str().unwrap()).await
}

/// 2.5 x `peak`: 250 req/s on a database of 100 req/s a copy.
pub(super) fn target() -> Value {
    json!({ "profile": "peak", "seed": 7, "target": { "multiple": 2.5 } })
}

#[tokio::test]
async fn post_plan_returns_202_and_the_plan_is_found_in_the_record() {
    let s = start(&config()).await;
    let r = plan(&s, target()).await;
    assert_eq!(r.status(), 202);
    let loc = r.headers()["location"].to_str().unwrap().to_string();
    let v: Value = r.json().await.unwrap();
    let id = v["id"].as_str().unwrap();
    assert_eq!(v["state"], "running");
    assert_eq!(loc, format!("/__kaanal/sim/runs/{id}"));
    let done = finished(&s, id).await;
    assert_eq!(done["state"], "done", "{done}");
    assert_eq!(done["kind"], "plan");
    assert_eq!(done["profile"], "peak");
    let p = &done["plan"];
    assert_eq!(p["verdict"]["kind"], "holds", "{p}");
    assert!(
        p["label"].as_str().unwrap().contains("not a prediction"),
        "{p}"
    );
    assert_eq!(p["request"]["target"], json!({ "multiple": 2.5 }));
    assert_eq!(p["request"]["req_s"], 250.0);
    assert!(done["proposed_draft"].is_string(), "{done}");
    assert_eq!(
        done["plan_steps"], p["steps"],
        "the progress is kept once done (here it is the plan): {done}"
    );
    let rep = s.get(&format!("/__kaanal/sim/runs/{id}/report")).await;
    assert_eq!(
        rep.status(),
        200,
        "a done plan has a report (plan_report.rs)"
    );
    let log = s.get(&format!("/__kaanal/sim/runs/{id}/log")).await;
    assert_eq!(log.status(), 409, "a plan has no log");
    let list: Value = s.get("/__kaanal/sim/runs").await.json().await.unwrap();
    let row = &list["runs"][0];
    assert_eq!(row["kind"], "plan");
    for k in ["plan", "proposed_draft", "plan_steps"] {
        assert!(row.get(k).is_none(), "the list strips {k}: {row}");
    }
}

/// The oracle: at 250 req/s a copy of mu = 100 needs c > 2.5 for a steady state, and at c = 3 the p99
/// is far under the 900 ms margin (Erlang C, see kaanal-sim's `plan_walk` tests), so the plan is exactly
/// three copies; the capacity estimate is c x mu before (100) and after (300), within 5 percent.
#[tokio::test]
async fn the_plan_holds_the_target_and_its_proposed_draft_builds_and_holds() {
    let s = start(&config()).await;
    let done = planned(&s, target()).await;
    assert_eq!(done["state"], "done", "{done}");
    let p = &done["plan"];
    let steps = p["steps"].as_array().unwrap();
    assert!(!steps.is_empty(), "{p}");
    for st in steps {
        assert_eq!(st["mv"]["node"], "db", "{st}");
        assert_eq!(st["mv"]["kind"]["knob"], "copies", "{st}");
    }
    assert_eq!(steps.last().unwrap()["mv"]["kind"]["to"], 3, "{p}");
    let near = |got: &Value, want: f64| {
        let g = got.as_f64().unwrap();
        assert!((g - want).abs() / want < 0.05, "{g} vs {want}");
    };
    near(&p["start"]["capacity_req_s"], MU);
    near(&p["end"]["capacity_req_s"], 3.0 * MU);
    assert_eq!(p["verify"]["holds"], true, "{p}");
    assert!(p["verify"].get("cost_note").is_none(), "a holding run: {p}");
    assert_eq!(
        p["verify"]["p99_ms"], p["end"]["p99_ms"],
        "the re-check is the walk's end"
    );
    assert!(p["verify"]["cost"].is_object(), "{p}");
    assert!(p["measured_knee"]["kind"].is_string(), "{p}");
    // the proposed draft keeps the developer's comment and gives the database three copies
    let text = done["proposed_draft"].as_str().unwrap();
    assert!(
        text.contains("# a comment the proposed draft must keep"),
        "{text}"
    );
    assert!(text.contains("replicas: 3"), "{text}");
    // and a server started on it already holds the target: nothing to change
    let s2 = start(text).await;
    let again = planned(&s2, target()).await;
    assert_eq!(again["plan"]["verdict"]["kind"], "already_holds", "{again}");
    assert_eq!(again["proposed_draft"].as_str(), Some(text));
}

#[tokio::test]
async fn an_unsizable_bottleneck_is_a_done_plan_with_a_cannot_hold_verdict() {
    let s = start(&config()).await;
    let mut body = target();
    body["keep"] = json!(["db"]);
    let done = planned(&s, body).await;
    assert_eq!(done["state"], "done", "{done}");
    let v = &done["plan"]["verdict"];
    assert_eq!(v["kind"], "cannot_hold", "{v}");
    assert_eq!(v["node"], "db", "{v}");
    assert!(v["reason"].as_str().unwrap().contains("kept"), "{v}");
    assert_eq!(done["plan"]["steps"], json!([]));
    assert_eq!(done["plan"]["verify"]["holds"], false, "{done}");
    let note = done["plan"]["verify"]["cost_note"].as_str().unwrap();
    assert!(note.contains("understated"), "a saturated run: {note}");
    // a pinned database also gets no family section of its own
    let fam = &done["plan"]["families"][0];
    assert_eq!(fam["kind"], "skipped", "{fam}");
    assert!(fam["reason"].as_str().unwrap().contains("kept"), "{fam}");
}

#[tokio::test]
async fn bad_bodies_are_400() {
    let s = start(&config()).await;
    for bad in [
        json!({}),
        json!({ "target": {} }),
        json!({ "target": { "multiple": 2, "req_s": 100 } }),
        json!({ "target": { "multiple": 0.05 } }),
        json!({ "target": { "multiple": 65 } }),
        json!({ "target": { "req_s": 0 } }),
        json!({ "target": { "req_s": -1 } }),
        json!({ "target": { "req_s": 2e6 } }),
        json!({ "target": { "multiple": 2 }, "max_steps": 101 }),
        json!({ "target": { "multiple": 2 }, "max_steps": 0 }),
        json!({ "target": { "multiple": 2 }, "keep": ["nope"] }),
        json!({ "target": { "multiple": 2 }, "budget_per_hour": -1 }),
        json!({ "target": { "multiple": 2 }, "p99_ms": 0 }),
        json!({ "target": { "multiple": 2 }, "error_pct": 101 }),
        json!({ "target": { "multiple": 2 }, "duration_s": 10 }),
        json!({ "target": { "multiple": 2, "x": 1 } }),
    ] {
        let mut body = bad.clone();
        body["profile"] = json!("peak");
        let r = plan(&s, body.clone()).await;
        assert_eq!(r.status(), 400, "{body}");
    }
    let runs: Value = s.get("/__kaanal/sim/runs").await.json().await.unwrap();
    assert_eq!(runs["runs"], json!([]), "a refused request starts nothing");
}

#[tokio::test]
async fn unknown_profile_and_scenario_are_404() {
    let s = start(&config()).await;
    let t = json!({ "multiple": 2 });
    assert_eq!(
        plan(&s, json!({ "profile": "nope", "target": t }))
            .await
            .status(),
        404
    );
    let r = plan(
        &s,
        json!({ "profile": "peak", "scenario": "nope", "target": t }),
    )
    .await;
    assert_eq!(r.status(), 404);
}

#[tokio::test]
async fn busy_is_409_with_the_id() {
    let s = start(&config()).await;
    let v: Value = s
        .send(
            reqwest::Method::POST,
            "/__kaanal/sim/run",
            Some(json!({ "profile": "big", "duration_s": 300, "warmup_s": 1 })),
        )
        .await
        .json()
        .await
        .unwrap();
    let id = v["id"].as_str().expect("202 with an id").to_string();
    let r = plan(&s, target()).await;
    assert_eq!(r.status(), 409);
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["id"], id.as_str());
    assert_eq!(finished(&s, &id).await["state"], "done");
}

#[tokio::test]
async fn no_system_is_409() {
    let s = start_blank().await;
    assert_eq!(plan(&s, target()).await.status(), 409);
}

#[tokio::test]
async fn a_plan_does_not_change_the_draft_or_the_applied_design() {
    let s = start(&config()).await;
    let before: Value = s.get("/__kaanal/draft").await.json().await.unwrap();
    let done = planned(&s, target()).await;
    assert_eq!(done["state"], "done", "{done}");
    let after: Value = s.get("/__kaanal/draft").await.json().await.unwrap();
    assert_eq!(after["text"], before["text"]);
    assert_eq!(after["text"].as_str(), Some(config().as_str()));
    // the system in force is the same generation: a run records it
    let v: Value = s
        .send(
            reqwest::Method::POST,
            "/__kaanal/sim/run",
            Some(json!({ "profile": "peak", "duration_s": 1 })),
        )
        .await
        .json()
        .await
        .unwrap();
    let run = finished(&s, v["id"].as_str().unwrap()).await;
    assert_eq!(run["generation"], done["generation"]);
    assert_eq!(run["generation"], 1);
}

#[tokio::test]
async fn two_plans_with_one_seed_are_identical() {
    let s = start(&config()).await;
    let a = planned(&s, target()).await;
    let b = planned(&s, target()).await;
    assert_eq!(a["state"], "done", "{a}");
    assert_eq!(a["plan"], b["plan"]);
    assert_eq!(a["proposed_draft"], b["proposed_draft"]);
}

#[tokio::test]
async fn the_record_carries_seed_fingerprint_and_chaos_digest() {
    let s = start(&config()).await;
    let done = planned(&s, target()).await;
    assert_eq!(done["seed"], 7, "{done}");
    let hex = |v: &Value| {
        let t = v.as_str().unwrap();
        t.len() == 12 && t.bytes().all(|c| c.is_ascii_hexdigit())
    };
    assert!(hex(&done["fingerprint"]), "{done}");
    assert!(hex(&done["chaos_digest"]), "{done}");
    assert!(
        done["live_chaos"].is_object(),
        "no scenario: the live chaos is recorded"
    );
    assert!(done.get("max_factor").is_none(), "a sweep's field: {done}");
}

/// The database-family what-if: a `postgres` database with no ops of its own is tried as each
/// non-relational flavor (`neo4j` only on request), each with its own walk; the low load holds as is.
#[tokio::test]
async fn the_database_family_section_walks_each_alternative() {
    let cfg = config().replace(
        "db:    { type: database, concurrency: 1, ops: { read: { mean: 10ms }, write: { mean: 10ms }, scan: { mean: 10ms } } }",
        "db:    { type: database, flavor: postgres }",
    );
    let s = start(&cfg).await;
    let done = planned(
        &s,
        json!({ "profile": "peak", "target": { "multiple": 0.5 } }),
    )
    .await;
    assert_eq!(done["state"], "done", "{done}");
    let fams = done["plan"]["families"].as_array().unwrap();
    let flavors: Vec<&str> = fams.iter().map(|f| f["flavor"].as_str().unwrap()).collect();
    assert_eq!(flavors, ["mongodb", "dynamodb", "cassandra"], "{fams:?}");
    for f in fams {
        assert_eq!(f["kind"], "answer", "{f}");
        assert_eq!(f["node"], "db");
        assert!(f["plan"]["verdict"]["kind"].is_string(), "{f}");
        let notes = f["notes"].to_string();
        assert!(notes.contains("preset estimates"), "{f}");
    }
}

/// The record and the text a plan patches must be one design. Here the server runs `config()` while
/// its applied text (the draft source) says something else, as after a live edit whose structure
/// waits for an Apply: a retry cannot help, so the route answers 409 and says to Apply, and no job
/// starts.
#[tokio::test]
async fn an_applied_text_that_is_not_the_design_in_force_is_409_saying_apply_it() {
    let other = config().replace("concurrency: 1", "concurrency: 2");
    let s = start_with(&config(), Some(&other)).await;
    let r = plan(&s, target()).await;
    assert_eq!(r.status(), 409);
    let v: Value = r.json().await.unwrap();
    let e = v["error"].as_str().unwrap();
    assert!(
        e.contains("differs from the applied text") && e.contains("Apply it"),
        "{e}"
    );
    let runs: Value = s.get("/__kaanal/sim/runs").await.json().await.unwrap();
    assert_eq!(runs["runs"], json!([]), "a refused request starts nothing");
}

/// A file the loader read (the spec) changed on disk after the design in force was built: the
/// applied text now builds to another design until the next Apply. 409, not a failed job.
#[tokio::test]
async fn a_spec_changed_on_disk_since_the_apply_is_409() {
    let s = start(&config()).await;
    let spec = s.dir.join("openapi.yaml");
    let text = std::fs::read_to_string(&spec).unwrap();
    // a new route: the spec the loader would read now is another design
    let changed = text.replace(
        "  /orders:",
        "  /ping:\n    get:\n      responses:\n        \"200\": { description: ok }\n  /orders:",
    );
    assert_ne!(changed, text);
    std::fs::write(&spec, changed).unwrap();
    let r = plan(&s, target()).await;
    assert_eq!(r.status(), 409);
    let v: Value = r.json().await.unwrap();
    assert!(v["error"].as_str().unwrap().contains("Apply it"), "{v}");
}

#[tokio::test]
async fn no_applied_text_is_409() {
    let s = start_with(&config(), None).await;
    let r = plan(&s, target()).await;
    assert_eq!(r.status(), 409);
    let v: Value = r.json().await.unwrap();
    assert!(v["error"].as_str().unwrap().contains("applied text"), "{v}");
}

/// An Azure SQL class does not run `MongoDB` (kaanal-core's family rules): that flavor is a skipped
/// section with the reason, the others are walked.
#[tokio::test]
async fn a_dropped_flavor_is_a_skipped_section_with_its_reason() {
    let cfg = config().replace(
        "db:    { type: database, concurrency: 1, ops: { read: { mean: 10ms }, write: { mean: 10ms }, scan: { mean: 10ms } } }",
        "db:    { type: database, flavor: sqlserver, hosting: managed, instance: azure/db.gp_gen5_4 }",
    );
    let s = start(&cfg).await;
    let done = planned(
        &s,
        json!({ "profile": "peak", "target": { "multiple": 0.5 } }),
    )
    .await;
    assert_eq!(done["state"], "done", "{done}");
    let fams = done["plan"]["families"].as_array().unwrap();
    let mongo = fams
        .iter()
        .find(|f| f["flavor"] == "mongodb")
        .unwrap_or_else(|| panic!("{fams:?}"));
    assert_eq!(mongo["kind"], "skipped", "{mongo}");
    assert!(
        mongo["reason"].as_str().unwrap().contains("Azure SQL"),
        "{mongo}"
    );
    assert!(fams.iter().any(|f| f["kind"] == "answer"), "{fams:?}");
}

/// No node saturates (unlimited concurrency, fixed times) and the p99 limit sits under the fixed path
/// latency: nothing sizable is the cause, so the verdict names no node and the reason names the limit.
#[tokio::test]
async fn a_limit_crossed_with_no_saturated_node_cannot_hold_and_names_no_node() {
    let s = start(&format!(
        "{}traffic:\n  peak:\n    rps: 100\n    journeys: [ {{ endpoint: 'GET /users/{{id}}' }} ]\n",
        system("1ms")
    ))
    .await;
    let done = planned(
        &s,
        json!({ "profile": "peak", "target": { "multiple": 1 }, "p99_ms": 1 }),
    )
    .await;
    assert_eq!(done["state"], "done", "{done}");
    let v = &done["plan"]["verdict"];
    assert_eq!(v["kind"], "cannot_hold", "{v}");
    assert!(v["node"].is_null(), "{v}");
    assert!(
        v["reason"].as_str().unwrap().contains("no node saturates"),
        "{v}"
    );
}

/// Busy is answered before the applied text is built: a plan while a run holds the slot is the
/// busy 409 with the holder's id, even when the applied text would also be refused.
#[tokio::test]
async fn busy_is_answered_before_the_applied_text_is_built() {
    let other = config().replace("concurrency: 1", "concurrency: 2");
    let s = start_with(&config(), Some(&other)).await;
    let v: Value = s
        .send(
            reqwest::Method::POST,
            "/__kaanal/sim/run",
            Some(json!({ "profile": "big", "duration_s": 300, "warmup_s": 1 })),
        )
        .await
        .json()
        .await
        .unwrap();
    let id = v["id"].as_str().expect("202 with an id").to_string();
    let r = plan(&s, target()).await;
    assert_eq!(r.status(), 409);
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["id"], id.as_str(), "{v}");
    assert_eq!(finished(&s, &id).await["state"], "done");
}
