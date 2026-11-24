//! `GET /sim/runs/{id}/report` on a cheapest plan (Step 9c-2): the search, the comparison and the
//! provider sections in Markdown, the same data as JSON, and the re-run flags.

use super::plan::planned;
use super::plan_cheapest::{cheapest, priced};
use crate::common::*;
use serde_json::{Value, json};

/// The text of the fenced block opened by `fence`.
fn fenced<'a>(md: &'a str, fence: &str) -> &'a str {
    md.split(fence)
        .nth(1)
        .expect("a fence")
        .split("\n```")
        .next()
        .unwrap()
}

async fn report(s: &Srv, id: &str, query: &str) -> reqwest::Response {
    s.get(&format!("/__kaanal/sim/runs/{id}/report{query}"))
        .await
}

#[tokio::test]
async fn the_report_route_serves_a_cheapest_plan_in_both_formats() {
    let s = start(&priced()).await;
    let done = planned(&s, cheapest(&json!({ "run_cap": 15 }))).await;
    assert_eq!(done["state"], "done", "{done}");
    let id = done["id"].as_str().unwrap();
    let get = |q: &'static str| report(&s, id, q);
    let md = get("").await;
    assert_eq!(md.status(), 200);
    let md = md.text().await.unwrap();
    assert!(md.starts_with(&format!("# Sizing plan {id}\n")), "{md}");
    for said in [
        "*cheapest found within the limit, not proven cheapest*",
        "## Search\n",
        "- Stopped because ",
        "- Runs: ",
        "## Comparison\n",
        "## If it ran on GCP\n",
        "## If it ran on Azure\n",
        "This what-if assumes wires, egress, data location and application are unchanged; on-demand price estimates.",
        "--cheapest --time=150s --run-cap=15 --providers=gcp,azure",
    ] {
        assert!(md.contains(said), "{said}: {md}");
    }
    let conclusion = done["plan"]["comparison"]["conclusion"].as_str().unwrap();
    // the fixture's three designs all hold; the design in force with the changes is the cheapest
    assert!(
        conclusion.starts_with("Winner: the design in force, with the changes above, $"),
        "{conclusion}"
    );
    assert!(md.contains(conclusion), "{md}");
    assert!(md.contains("This plan is not a prediction."), "{md}");
    let data: Value = serde_json::from_str(fenced(&md, "## Data\n\n```json\n")).unwrap();
    let j = get("?format=json").await;
    assert_eq!(j.status(), 200);
    let j: Value = j.json().await.unwrap();
    assert_eq!(j, data, "the data block is the JSON form");
    // what the record says is what the report says
    assert_eq!(j["plan"]["search"], done["plan"]["search"]);
    assert_eq!(j["plan"]["providers"], done["plan"]["providers"]);
    assert_eq!(j["plan"]["cheapest_note"], done["plan"]["cheapest_note"]);
    assert_eq!(j["plan"]["providers"].as_array().unwrap().len(), 2, "{j}");
    assert!(
        j["recipe"]["command"].as_str().unwrap().ends_with(
            "--max-steps 30 --cheapest --time=150s --run-cap=15 --providers=gcp,azure --seed 7"
        ),
        "{j}"
    );
    for q in ["", "?format=json"] {
        let a = get(q).await.bytes().await.unwrap();
        let b = get(q).await.bytes().await.unwrap();
        assert_eq!(a, b, "the same bytes on every fetch: {q}");
    }
}

/// Without `cheapest` the route serves the 9c-1 report: none of the new parts.
#[tokio::test]
async fn a_plain_plan_report_has_no_search_or_sections() {
    let s = start(&priced()).await;
    let done = planned(&s, super::plan::target()).await;
    let id = done["id"].as_str().unwrap();
    let md = report(&s, id, "").await.text().await.unwrap();
    for gone in [
        "## Search",
        "## Comparison",
        "If it ran on",
        "cheapest",
        "--cheapest",
    ] {
        assert!(!md.contains(gone), "{gone}: {md}");
    }
    assert!(md.contains("## Menu"), "{md}");
}
