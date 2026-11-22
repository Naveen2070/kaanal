//! `GET /sim/runs/{id}/report` and `GET /sim/runs/{id}/draft` on a plan record (Step 9c-1).

use super::plan::{config, finished, plan, planned, target};
use crate::common::*;
use serde_json::{Value, json};

async fn report(s: &Srv, id: &str, query: &str) -> reqwest::Response {
    s.get(&format!("/__kaanal/sim/runs/{id}/report{query}"))
        .await
}

async fn draft(s: &Srv, id: &str) -> reqwest::Response {
    s.get(&format!("/__kaanal/sim/runs/{id}/draft")).await
}

/// The text of the fenced block opened by `fence`.
fn fenced<'a>(md: &'a str, fence: &str) -> &'a str {
    let rest = md.split(fence).nth(1).expect("a fence");
    rest.split("\n```").next().unwrap()
}

#[tokio::test]
async fn the_report_route_serves_a_plan_as_markdown_and_json_with_the_same_bytes_everywhere() {
    let s = start(&config()).await;
    let done = planned(&s, target()).await;
    let id = done["id"].as_str().unwrap();
    let md = report(&s, id, "").await;
    assert_eq!(md.status(), 200);
    assert_eq!(md.headers()["content-type"], "text/markdown; charset=utf-8");
    let md = md.text().await.unwrap();
    assert!(md.starts_with(&format!("# Sizing plan {id}\n")), "{md}");
    assert!(md.contains("This plan is not a prediction."), "{md}");
    assert!(md.contains("copies 1 to 2"), "the steps: {md}");
    assert!(
        md.contains("## Verdict\n\nThe plan holds the target."),
        "{md}"
    );
    let data: Value = serde_json::from_str(fenced(&md, "## Data\n\n```json\n")).unwrap();
    let j = report(&s, id, "?format=json").await;
    assert_eq!(j.status(), 200);
    assert_eq!(j.headers()["content-type"], "application/json");
    let j: Value = j.json().await.unwrap();
    assert_eq!(j, data, "the data block is the JSON form");
    // what the record says is what the report says
    assert_eq!(j["plan"]["steps"], done["plan"]["steps"]);
    assert_eq!(j["plan"]["verdict"], done["plan"]["verdict"]);
    assert_eq!(j["recipe"]["seed"], 7);
    assert_eq!(j["recipe"]["fingerprint"], done["fingerprint"]);
    assert_eq!(j["recipe"]["chaos_digest"], done["chaos_digest"]);
    assert_eq!(
        j["recipe"]["command"],
        "kaanal sim plan --profile=peak --target=2.5x --p99 1s --errors 1% --max-steps 30 --seed 7"
    );
    // the same bytes on every fetch, in both forms
    for q in ["", "?format=json"] {
        let a = report(&s, id, q).await.bytes().await.unwrap();
        let b = report(&s, id, q).await.bytes().await.unwrap();
        assert_eq!(a, b, "{q}");
    }
    assert_eq!(
        report(&s, id, "?format=xml").await.status(),
        400,
        "another format is refused"
    );
}

#[tokio::test]
async fn the_draft_route_serves_the_proposed_text() {
    let s = start(&config()).await;
    let done = planned(&s, target()).await;
    let id = done["id"].as_str().unwrap();
    let r = draft(&s, id).await;
    assert_eq!(r.status(), 200);
    assert_eq!(r.headers()["content-type"], "text/plain; charset=utf-8");
    let text = r.text().await.unwrap();
    assert_eq!(text, done["proposed_draft"].as_str().unwrap());
    assert!(text.contains("# a comment the proposed draft must keep"));
    assert!(text.contains("replicas: 3"), "{text}");
    // fetching it changes nothing: the draft Studio holds is untouched
    let held: Value = s.get("/__kaanal/draft").await.json().await.unwrap();
    assert!(!held.to_string().contains("replicas: 3"), "{held}");
    assert_eq!(draft(&s, "s999").await.status(), 404);
    assert_eq!(report(&s, "s999", "").await.status(), 404);
}

#[tokio::test]
async fn a_sweep_report_is_unchanged() {
    // the bytes are pinned by tests/golden/sweep-report.md (report/tests.rs); here the route still
    // hands a sweep to the sweep maker, and a sweep has no draft
    let s = start(&config()).await;
    let r = s
        .send(
            reqwest::Method::POST,
            "/__kaanal/sim/sweep",
            Some(json!({ "profile": "peak", "max_factor": 1 })),
        )
        .await;
    let id = r.json::<Value>().await.unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    finished(&s, &id).await;
    let md = report(&s, &id, "").await.text().await.unwrap();
    assert!(md.starts_with(&format!("# Load sweep {id}\n")), "{md}");
    assert!(!md.contains("Sizing plan"), "{md}");
    assert!(md.contains("kaanal sim sweep --profile=peak"), "{md}");
    let d = draft(&s, &id).await;
    assert_eq!(d.status(), 409);
    let why: Value = d.json().await.unwrap();
    assert!(why["error"].as_str().unwrap().contains("sweep"), "{why}");
}

// The running state is held deterministically by the unit tests (control/plan.rs for the draft,
// report/plan/recipe_tests.rs for the report); here only a failed plan, which needs no timing.
#[tokio::test]
async fn report_and_draft_routes_refuse_a_failed_plan() {
    // a plan that failed has neither, and the error says why
    let s = start(&format!(
        "{}traffic:
  huge:
    rps: 100000
    journeys: [ {{ endpoint: 'GET /users/{{id}}' }} ]
",
        system("1h")
    ))
    .await;
    let r = plan(
        &s,
        json!({ "profile": "huge", "target": { "multiple": 1 } }),
    )
    .await;
    assert_eq!(r.status(), 202);
    let id = r.json::<Value>().await.unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    let rec = finished(&s, &id).await;
    assert_eq!(rec["state"], "failed", "{rec}");
    for res in [report(&s, &id, "").await, draft(&s, &id).await] {
        assert_eq!(res.status(), 409);
        let v: Value = res.json().await.unwrap();
        let e = v["error"].as_str().unwrap();
        assert!(e.contains("plan failed"), "{v}");
        assert!(e.contains(rec["error"].as_str().unwrap()), "{v}");
    }
}

#[tokio::test]
async fn a_plain_run_has_no_plan_report_or_draft() {
    let s = start(&config()).await;
    let r = s
        .send(
            reqwest::Method::POST,
            "/__kaanal/sim/run",
            Some(json!({ "profile": "peak", "duration_s": 2, "warmup_s": 0.2 })),
        )
        .await;
    let id = r.json::<Value>().await.unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    finished(&s, &id).await;
    assert_eq!(report(&s, &id, "").await.status(), 409);
    assert_eq!(draft(&s, &id).await.status(), 409);
}
