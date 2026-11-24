//! `POST /sim/plan` with `cheapest: true` (Step 9c-2): the cheapest search on the design in force,
//! one what-if section per other provider, the search's record, and the new fields' bounds. The
//! fixtures are `plan.rs`'s; `priced()` puts the service and the database on `aws/m5.large` so the
//! plan has prices and a provider to move from.

use super::plan::{config, finished, plan, planned, target};
use crate::common::*;
use serde_json::{Value, json};

/// What every provider section says (spec anti-goal: a comparison that hides what it does not model).
const CAVEAT: &str =
    "assumes wires, egress, data location and application are unchanged; on-demand price estimates";
/// What every cheapest answer says (spec anti-goal: never "cheapest" without the limit).
const CHEAPEST: &str = "cheapest found within the limit, not proven cheapest";

/// `config()` with the service and the database on a priced AWS class.
pub(super) fn priced() -> String {
    config()
        .replace(
            "app:   { type: service, spec: openapi.yaml, proc: 1ms }",
            "app:   { type: service, spec: openapi.yaml, proc: 1ms, instance: aws/m5.large }",
        )
        .replace(
            "db:    { type: database, concurrency: 1,",
            "db:    { type: database, concurrency: 1, instance: aws/m5.large,",
        )
}

/// `target()` as a cheapest search with `extra` merged in (a small run cap keeps a debug build quick).
pub(super) fn cheapest(extra: &Value) -> Value {
    let mut b = target();
    b["cheapest"] = json!(true);
    b["run_cap"] = json!(30);
    for (k, v) in extra.as_object().unwrap() {
        b[k] = v.clone();
    }
    b
}

#[tokio::test]
async fn a_cheapest_plan_is_found_in_the_record_with_its_search_info() {
    let s = start(&config()).await;
    let done = planned(&s, cheapest(&json!({ "providers": ["none"] }))).await;
    assert_eq!(done["state"], "done", "{done}");
    let p = &done["plan"];
    assert!(
        p["label"].as_str().unwrap().contains("not a prediction"),
        "{p}"
    );
    assert_eq!(p["cheapest_note"], CHEAPEST, "{p}");
    assert_eq!(p["verdict"]["kind"], "holds", "{p}");
    let info = &p["search"];
    let stop = info["stop"].as_str().unwrap();
    assert!(
        ["exhausted", "run_cap", "time_limit"].contains(&stop),
        "{info}"
    );
    let runs = info["runs"].as_u64().unwrap();
    assert!((1..=30).contains(&runs), "{info}");
    assert_eq!(info["run_cap"], 30, "{info}");
    assert_eq!(info["limit_ms"], 150_000, "the default limit: {info}");
    assert!(info["elapsed_ms"].is_u64(), "{info}");
    assert_eq!(
        p["request"]["cheapest"],
        json!({ "time_limit_s": 150, "run_cap": 30, "providers": [] }),
        "{p}"
    );
    assert_ne!(
        done["plan_steps"],
        json!([]),
        "the main search's progress: {done}"
    );
}

/// The service runs eight copies it does not need (the database is the bottleneck): the walk leaves
/// them, the search takes them off, so the cheapest plan is strictly cheaper and says so.
#[tokio::test]
async fn the_cheapest_plan_is_not_dearer_than_the_walk_for_the_same_request() {
    let cfg = priced().replacen(
        "instance: aws/m5.large }",
        "instance: aws/m5.large, replicas: 8 }",
        1,
    );
    assert!(cfg.contains("replicas: 8"), "{cfg}");
    let s = start(&cfg).await;
    let walked = planned(&s, target()).await;
    let searched = planned(&s, cheapest(&json!({ "providers": ["none"] }))).await;
    assert_eq!(walked["plan"]["verdict"]["kind"], "holds", "{walked}");
    assert_eq!(searched["plan"]["verdict"]["kind"], "holds", "{searched}");
    let price = |v: &Value| v["plan"]["price_after"].as_u64().unwrap();
    assert!(
        price(&searched) < price(&walked),
        "{} vs {}",
        price(&searched),
        price(&walked)
    );
    let steps = searched["plan"]["steps"].to_string();
    assert!(steps.contains("cheaper by"), "{steps}");
}

/// Review Focus 3, both kinds in one record: a managed SQL Server on RDS has an Azure SQL class
/// (an answer) and no managed class on GCP (skipped, the node named).
#[tokio::test]
async fn a_skipped_provider_does_not_stop_the_others() {
    let cfg = config().replace(
        "db:    { type: database, concurrency: 1, ops: { read: { mean: 10ms }, write: { mean: 10ms }, scan: { mean: 10ms } } }",
        "db:    { type: database, flavor: sqlserver, hosting: managed, engine: sqlserver-standard, instance: aws/db.m5.large }",
    );
    let s = start(&cfg).await;
    let done = planned(
        &s,
        json!({ "profile": "peak", "seed": 7, "target": { "multiple": 0.5 }, "cheapest": true, "run_cap": 15 }),
    )
    .await;
    assert_eq!(done["state"], "done", "{done}");
    let secs = done["plan"]["providers"].as_array().unwrap();
    assert_eq!(secs.len(), 2, "{secs:?}");
    assert_eq!(secs[0]["provider"], "gcp", "{secs:?}");
    assert_eq!(secs[0]["kind"], "skipped", "{secs:?}");
    assert_eq!(
        secs[0]["reason"], "db: no managed sqlserver class for gcp in the price table",
        "{secs:?}"
    );
    assert_eq!(secs[1]["provider"], "azure", "{secs:?}");
    assert_eq!(secs[1]["kind"], "answer", "{secs:?}");
    assert!(secs[1]["cost"]["total_micro_usd_hr"].is_u64(), "{secs:?}");
}

/// Spec: a section counts per-use prices at the target run's visits like the main plan. The gateway
/// is managed (an API Gateway, priced by use) and kept as written by the translation; with a budget
/// just under the section's price with per-use prices (and far over its table price), the section is
/// over budget, re-judged, not `holds`.
#[tokio::test]
async fn a_section_is_judged_against_the_budget_with_per_use_prices() {
    let cfg = priced().replace(
        "nginx: { type: gateway, proc: 1ms }",
        "nginx: { type: gateway, proc: 1ms, hosting: managed }",
    );
    assert!(cfg.contains("hosting: managed"), "{cfg}");
    let s = start(&cfg).await;
    let body = |extra: &Value| {
        cheapest(&json!({ "providers": ["gcp"], "run_cap": 15 }))
            .as_object()
            .unwrap()
            .clone()
            .into_iter()
            .chain(extra.as_object().unwrap().clone())
            .collect::<serde_json::Map<String, Value>>()
    };
    let free = planned(&s, Value::Object(body(&json!({})))).await;
    let sec = &free["plan"]["providers"][0];
    assert_eq!(sec["kind"], "answer", "{sec}");
    assert_eq!(sec["plan"]["verdict"]["kind"], "holds", "{sec}");
    let table = sec["plan"]["price_after"].as_u64().unwrap();
    let with_use = sec["cost"]["total_micro_usd_hr"].as_u64().unwrap();
    assert!(
        with_use > table,
        "the gateway's per-use price counts: {with_use} vs {table}"
    );
    #[expect(
        clippy::cast_precision_loss,
        reason = "micro-USD an hour, far under 2^52"
    )]
    // just under the price with per-use prices: far above any table price the walk passes (the
    // gateway's per-use part is most of it), so the table price never binds and the path is the same
    let budget = (with_use - 1_000) as f64 / 1e6;
    let tight = planned(
        &s,
        Value::Object(body(&json!({ "budget_per_hour": budget }))),
    )
    .await;
    let sec = &tight["plan"]["providers"][0];
    assert_eq!(sec["kind"], "answer", "{sec}");
    assert_eq!(sec["plan"]["verdict"]["kind"], "over_budget", "{sec}");
    assert_eq!(sec["plan"]["verdict"]["holds"], true, "{sec}");
    assert_eq!(sec["budget_check"]["rejudged"], true, "{sec}");
}

#[tokio::test]
async fn its_proposed_draft_builds_and_holds() {
    let s = start(&priced()).await;
    let done = planned(&s, cheapest(&json!({ "providers": ["gcp"] }))).await;
    assert_eq!(done["state"], "done", "{done}");
    let p = &done["plan"];
    assert_eq!(p["verify"]["holds"], true, "{p}");
    assert_eq!(
        p["verify"]["p99_ms"], p["end"]["p99_ms"],
        "the re-check is the search's end"
    );
    let text = done["proposed_draft"].as_str().unwrap();
    assert!(
        text.contains("# a comment the proposed draft must keep"),
        "{text}"
    );
    assert!(
        !text.contains("gcp/"),
        "the draft is the main search's design only: {text}"
    );
    let s2 = start(text).await;
    let again = planned(&s2, target()).await;
    assert_eq!(again["plan"]["verdict"]["kind"], "already_holds", "{again}");
}

#[tokio::test]
async fn a_provider_section_prices_the_translated_design_and_carries_the_caveat() {
    let s = start(&priced()).await;
    // no providers named: every other provider the design's classes translate to, in a fixed order
    let done = planned(&s, cheapest(&json!({ "run_cap": 15 }))).await;
    assert_eq!(done["state"], "done", "{done}");
    let p = &done["plan"];
    let secs = p["providers"].as_array().unwrap();
    let names: Vec<&str> = secs
        .iter()
        .map(|x| x["provider"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["gcp", "azure"], "{secs:?}");
    for sec in secs {
        let to = sec["provider"].as_str().unwrap();
        assert_eq!(sec["kind"], "answer", "{sec}");
        let notes = sec["notes"].to_string();
        assert!(notes.contains(CAVEAT), "{sec}");
        assert!(notes.contains(CHEAPEST), "{sec}");
        assert!(
            notes.contains(&format!("app: aws/m5.large to {to}/")),
            "{sec}"
        );
        assert!(sec["plan"]["price_after"].is_u64(), "priced: {sec}");
        assert!(sec["plan"]["verdict"]["kind"].is_string(), "{sec}");
        assert!(sec["info"]["stop"].is_string(), "{sec}");
        assert!(sec["info"]["runs"].as_u64().unwrap() <= 15, "{sec}");
        // only the provider prefix of a class changes: every class step stays on that provider
        for st in sec["plan"]["steps"].as_array().unwrap() {
            if st["mv"]["kind"]["knob"] == "class" {
                let to_class = st["mv"]["kind"]["to"].as_str().unwrap();
                assert!(to_class.starts_with(&format!("{to}/")), "{st}");
            }
        }
    }
    // the main plan stays on AWS
    assert!(!done["proposed_draft"].as_str().unwrap().contains("gcp/"));
}

/// Review Focus 3: a managed Postgres has no managed class on GCP and no Azure SQL equivalent, so
/// both sections are skipped with the node named and what the price table lacks; the main search
/// still answers.
#[tokio::test]
async fn a_provider_that_cannot_translate_is_skipped_with_the_reason() {
    let cfg = config().replace(
        "db:    { type: database, concurrency: 1, ops: { read: { mean: 10ms }, write: { mean: 10ms }, scan: { mean: 10ms } } }",
        "db:    { type: database, flavor: postgres, hosting: managed, instance: aws/db.m5.large }",
    );
    let s = start(&cfg).await;
    let done = planned(
        &s,
        json!({ "profile": "peak", "seed": 7, "target": { "multiple": 0.5 }, "cheapest": true, "run_cap": 15 }),
    )
    .await;
    assert_eq!(done["state"], "done", "{done}");
    let secs = done["plan"]["providers"].as_array().unwrap();
    assert_eq!(secs.len(), 2, "{secs:?}");
    for sec in secs {
        let to = sec["provider"].as_str().unwrap();
        assert_eq!(sec["kind"], "skipped", "{sec}");
        assert_eq!(
            sec["reason"],
            format!("db: no managed postgres class for {to} in the price table"),
            "{sec}"
        );
    }
    assert!(done["plan"]["search"]["stop"].is_string(), "{done}");
}

#[tokio::test]
async fn time_limit_run_cap_and_providers_need_cheapest_and_are_bounded() {
    let s = start(&priced()).await;
    for extra in [
        json!({ "time_limit_s": 60 }),
        json!({ "run_cap": 10 }),
        json!({ "providers": ["gcp"] }),
        json!({ "cheapest": false, "run_cap": 10 }),
        json!({ "cheapest": true, "time_limit_s": 0 }),
        json!({ "cheapest": true, "time_limit_s": 601 }),
        json!({ "cheapest": true, "time_limit_s": -1 }),
        json!({ "cheapest": true, "run_cap": 0 }),
        json!({ "cheapest": true, "run_cap": 2001 }),
        json!({ "cheapest": true, "providers": [] }),
        json!({ "cheapest": true, "providers": ["mars"] }),
        json!({ "cheapest": true, "providers": ["aws"] }),
        json!({ "cheapest": true, "providers": ["gcp", "gcp"] }),
        json!({ "cheapest": true, "providers": ["none", "gcp"] }),
        json!({ "cheapest": true, "providers": "gcp" }),
    ] {
        let mut body = target();
        for (k, v) in extra.as_object().unwrap() {
            body[k] = v.clone();
        }
        let r = plan(&s, body.clone()).await;
        assert_eq!(r.status(), 400, "{body}");
        let v: Value = r.json().await.unwrap();
        assert!(v["error"].is_string(), "{v}");
    }
    let r = plan(
        &s,
        json!({ "profile": "peak", "target": { "multiple": 2 }, "run_cap": 10 }),
    )
    .await;
    let v: Value = r.json().await.unwrap();
    assert!(
        v["error"].as_str().unwrap().contains("needs cheapest"),
        "{v}"
    );
    let runs: Value = s.get("/__kaanal/sim/runs").await.json().await.unwrap();
    assert_eq!(runs["runs"], json!([]), "a refused request starts nothing");
}

/// Opt-in: without `cheapest` the record is the 9c-1 record, field for field.
#[tokio::test]
async fn a_plain_plan_is_unchanged() {
    let s = start(&priced()).await;
    let done = planned(&s, target()).await;
    assert_eq!(done["state"], "done", "{done}");
    let keys = |v: &Value| {
        let mut k: Vec<String> = v.as_object().unwrap().keys().cloned().collect();
        k.sort();
        k
    };
    assert_eq!(
        keys(&done["plan"]),
        [
            "end",
            "families",
            "label",
            "measured_knee",
            "price_after",
            "price_before",
            "request",
            "start",
            "steps",
            "unpriced_nodes",
            "verdict",
            "verify",
            "words"
        ],
        "{done}"
    );
    assert_eq!(
        keys(&done["plan"]["request"]),
        [
            "budget_micro_usd_hr",
            "factor",
            "keep",
            "limit",
            "max_steps",
            "req_s",
            "target"
        ]
    );
}

#[tokio::test]
async fn providers_none_runs_no_sections() {
    let s = start(&priced()).await;
    let done = planned(
        &s,
        cheapest(&json!({ "providers": ["none"], "run_cap": 15 })),
    )
    .await;
    assert_eq!(done["state"], "done", "{done}");
    let p = &done["plan"];
    assert_eq!(
        p["providers"],
        json!([]),
        "a cheapest plan always has the list: {p}"
    );
    assert!(p["search"].is_object(), "{p}");
}

#[tokio::test]
async fn the_work_permit_and_busy_order_still_hold() {
    let s = start(&priced()).await;
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
    // a bad new field is a 400 before the busy check, as every other bad field
    let r = plan(&s, cheapest(&json!({ "run_cap": 0 }))).await;
    assert_eq!(r.status(), 400);
    // a good cheapest request while the slot is held: busy, with the holder's id
    let r = plan(&s, cheapest(&json!({}))).await;
    assert_eq!(r.status(), 409);
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["id"], id.as_str(), "{v}");
    assert_eq!(finished(&s, &id).await["state"], "done");
}

/// Defect 2 of the 9c-2 hand run: a kept node with a table class would stay on AWS while the rest
/// moved, a mixed design. Every section is skipped with the node named; the main search still runs.
/// A kept node with no class (the gateway) mixes nothing, so its section still answers.
#[tokio::test]
async fn a_kept_node_with_a_class_skips_the_sections_rather_than_mix_providers() {
    let s = start(&priced()).await;
    let done = planned(&s, cheapest(&json!({ "keep": ["db"], "run_cap": 10 }))).await;
    assert_eq!(done["state"], "done", "{done}");
    let secs = done["plan"]["providers"].as_array().unwrap();
    assert_eq!(secs.len(), 2, "{secs:?}");
    for sec in secs {
        let to = sec["provider"].as_str().unwrap();
        assert_eq!(sec["kind"], "skipped", "{sec}");
        assert_eq!(
            sec["reason"],
            format!("db: kept on aws, so a section on {to} would mix providers"),
            "{sec}"
        );
    }
    assert!(done["plan"]["search"]["stop"].is_string(), "{done}");
    let done = planned(
        &s,
        cheapest(&json!({ "keep": ["nginx"], "providers": ["gcp"], "run_cap": 10 })),
    )
    .await;
    assert_eq!(done["state"], "done", "{done}");
    let sec = &done["plan"]["providers"][0];
    assert_eq!(sec["kind"], "answer", "{sec}");
    let notes = sec["notes"].to_string();
    assert!(
        notes.contains("nginx: kept as written, not compared (kept by you)"),
        "{notes}"
    );
}
