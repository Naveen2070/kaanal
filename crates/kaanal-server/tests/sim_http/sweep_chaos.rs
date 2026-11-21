//! A sweep records the chaos in force beside the design fingerprint (RULES 140): none, the named
//! scenario, or the live chaos (active scenarios, a count and a digest of the live entries).

use crate::common::*;
use serde_json::{Value, json};
use std::time::Duration;

/// `system()` with a profile and a scenario; `swap` lists `db` before `app` (a different node order,
/// the same design by name).
fn config(swap: bool) -> String {
    let mut sys = system("1ms");
    if swap {
        let app = "  app:   { type: service, spec: openapi.yaml, proc: 1ms }\n";
        let db = "  db:    { type: database, ops: { read: 1ms, write: 1ms, scan: 1ms } }\n";
        sys = sys.replace(&format!("{app}{db}"), &format!("{db}{app}"));
        assert!(
            sys.find("  db:").unwrap() < sys.find("  app:").unwrap(),
            "{sys}"
        );
    }
    format!(
        "{sys}traffic:
  peak:
    rps: 100
    journeys: [ {{ endpoint: 'GET /users/{{id}}' }} ]
scenarios:
  slow: {{ chaos: [ {{ node: app, latency_ms: 50 }} ] }}
"
    )
}

/// Sweeps `peak` (five points) with `extra` in the body; the finished record.
async fn swept(s: &Srv, extra: Value) -> Value {
    let mut body = json!({ "profile": "peak", "max_factor": 1 });
    for (k, v) in extra.as_object().unwrap() {
        body[k] = v.clone();
    }
    let r = s
        .send(reqwest::Method::POST, "/__kaanal/sim/sweep", Some(body))
        .await;
    assert_eq!(r.status(), 202);
    let id = r.json::<Value>().await.unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    for _ in 0..2400 {
        let v: Value = s
            .get(&format!("/__kaanal/sim/runs/{id}"))
            .await
            .json()
            .await
            .unwrap();
        if v["state"] != "running" {
            assert_eq!(v["state"], "done", "{v}");
            return v;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("sweep {id} did not finish");
}

async fn report(s: &Srv, rec: &Value) -> String {
    let id = rec["id"].as_str().unwrap();
    s.get(&format!("/__kaanal/sim/runs/{id}/report"))
        .await
        .text()
        .await
        .unwrap()
}

async fn set_chaos(s: &Srv, body: Value) {
    let r = s.put("/__kaanal/chaos", body).await;
    assert!(r.status().is_success(), "{}", r.text().await.unwrap());
}

#[tokio::test]
async fn no_chaos_is_recorded_as_none() {
    let s = start(&config(false)).await;
    let rec = swept(&s, json!({})).await;
    assert_eq!(rec["live_chaos"]["entries"], 0, "{rec}");
    assert_eq!(rec["live_chaos"]["scenarios"], json!([]));
    assert_eq!(rec["live_chaos"]["digest"].as_str().unwrap().len(), 12);
    assert!(report(&s, &rec).await.contains("- Chaos in force: none.\n"));
}

#[tokio::test]
async fn a_named_scenario_is_the_chaos_and_no_live_chaos_is_recorded() {
    let s = start(&config(false)).await;
    // live chaos set but not used: --scenario runs under that scenario alone
    set_chaos(&s, json!({ "node": "db", "latency_ms": 5 })).await;
    let rec = swept(&s, json!({ "scenario": "slow" })).await;
    assert_eq!(rec["scenario"], "slow");
    assert!(rec.get("live_chaos").is_none(), "{rec}");
    let md = report(&s, &rec).await;
    assert!(md.contains("- Chaos in force: scenario slow.\n"), "{md}");
    assert!(!md.contains("live chaos in force"), "{md}");
}

#[tokio::test]
async fn live_chaos_and_active_scenarios_are_recorded_and_the_recipe_warns() {
    let s = start(&config(false)).await;
    let none = swept(&s, json!({})).await["live_chaos"]["digest"].clone();
    set_chaos(&s, json!({ "node": "db", "latency_ms": 5 })).await;
    assert!(
        s.send(
            reqwest::Method::POST,
            "/__kaanal/scenarios/slow/activate",
            None
        )
        .await
        .status()
        .is_success()
    );
    let rec = swept(&s, json!({})).await;
    let lc = &rec["live_chaos"];
    assert_eq!(lc["entries"], 1, "{rec}");
    assert_eq!(lc["scenarios"], json!(["slow"]));
    assert_ne!(lc["digest"], none, "a live entry changes the digest");
    let md = report(&s, &rec).await;
    let what = format!(
        "active scenarios slow; 1 entry, digest {}",
        lc["digest"].as_str().unwrap()
    );
    assert!(
        md.contains(&format!("- Chaos in force: live chaos: {what}.\n")),
        "{md}"
    );
    assert!(
        md.contains(&format!(
            "# live chaos in force when this ran ({what}): the command alone cannot reproduce it"
        )),
        "{md}"
    );
    // a different value under the same key is a different digest
    set_chaos(&s, json!({ "node": "db", "latency_ms": 6 })).await;
    let again = swept(&s, json!({})).await;
    assert_ne!(again["live_chaos"]["digest"], lc["digest"]);
}

/// The live entries are a set keyed by name: the digest is the same whatever order they were set in
/// and whatever order the config lists the nodes in (node ids differ, names do not). The active
/// scenarios are an ordered stack (a later one wins), so their order is recorded as it is.
#[tokio::test]
async fn the_live_chaos_digest_ignores_set_order_and_node_order() {
    let entries = [
        json!({ "node": "db", "latency_ms": 5 }),
        json!({ "endpoint": "GET /users/{id}", "latency_ms": 7 }),
        json!({ "node": "app", "endpoint": "GET /users/{id}", "error_rate": 0.01 }),
    ];
    let mut digests = Vec::new();
    for (swap, reverse) in [(false, false), (false, true), (true, false)] {
        let s = start(&config(swap)).await;
        let mut order: Vec<&Value> = entries.iter().collect();
        if reverse {
            order.reverse();
        }
        for e in order {
            set_chaos(&s, e.clone()).await;
        }
        let rec = swept(&s, json!({})).await;
        assert_eq!(rec["live_chaos"]["entries"], 3, "{rec}");
        digests.push(rec["live_chaos"]["digest"].clone());
    }
    assert_eq!(digests[0], digests[1], "set order");
    assert_eq!(digests[0], digests[2], "node order");
}

/// The digest of the compiled chaos a sweep ran under (RULES 140): it covers what a name does not.
fn digest(rec: &Value) -> String {
    let d = rec["chaos_digest"]
        .as_str()
        .unwrap_or_else(|| panic!("no chaos_digest: {rec}"));
    assert_eq!(d.len(), 12, "{d}");
    d.to_string()
}

async fn put_scenario(s: &Srv, name: &str, latency_ms: u64) {
    let r = s
        .put(
            &format!("/__kaanal/scenarios/{name}"),
            json!({ "chaos": [ { "node": "db", "latency_ms": latency_ms } ] }),
        )
        .await;
    assert!(r.status().is_success(), "{}", r.text().await.unwrap());
}

#[tokio::test]
async fn a_runtime_scenario_redefined_between_two_sweeps_changes_the_chaos_digest() {
    let s = start(&config(false)).await;
    put_scenario(&s, "rt", 5).await;
    let a = swept(&s, json!({ "scenario": "rt" })).await;
    let again = swept(&s, json!({ "scenario": "rt" })).await;
    assert_eq!(
        digest(&a),
        digest(&again),
        "the same chaos twice: the same digest"
    );
    assert_eq!(a["fingerprint"], again["fingerprint"]);
    put_scenario(&s, "rt", 9).await;
    let b = swept(&s, json!({ "scenario": "rt" })).await;
    assert_eq!(
        a["fingerprint"], b["fingerprint"],
        "the design files did not change"
    );
    assert_ne!(
        digest(&a),
        digest(&b),
        "the redefined scenario is different chaos"
    );
    assert_eq!(b["runtime_scenarios"], json!(["rt"]), "{b}");
    let md = report(&s, &b).await;
    assert!(
        md.contains(&format!("- Chaos digest: {}.", digest(&b))),
        "{md}"
    );
    assert!(
        md.contains(
            "# scenarios defined at runtime in force (rt): the command alone cannot reproduce it"
        ),
        "{md}"
    );
    // active on the stack (no --scenario): the runtime name is still flagged
    assert!(
        s.send(
            reqwest::Method::POST,
            "/__kaanal/scenarios/rt/activate",
            None
        )
        .await
        .status()
        .is_success()
    );
    let live = swept(&s, json!({})).await;
    assert_eq!(live["runtime_scenarios"], json!(["rt"]), "{live}");
    assert_eq!(
        digest(&live),
        digest(&b),
        "the same compiled chaos, reached through the stack"
    );
}

#[tokio::test]
async fn a_file_scenario_edited_and_applied_changes_the_chaos_digest() {
    let s = start(&config(false)).await;
    let file = s.dir.join("scenarios/onfile.yaml");
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    let apply = || s.send(reqwest::Method::POST, "/__kaanal/draft/apply", None);
    std::fs::write(&file, "chaos:\n  - { node: db, latency_ms: 5 }\n").unwrap();
    assert_eq!(apply().await.status(), 200);
    let a = swept(&s, json!({ "scenario": "onfile" })).await;
    assert!(
        a.get("runtime_scenarios").is_none(),
        "a file scenario is not runtime: {a}"
    );
    std::fs::write(&file, "chaos:\n  - { node: db, latency_ms: 8 }\n").unwrap();
    assert_eq!(apply().await.status(), 200);
    let b = swept(&s, json!({ "scenario": "onfile" })).await;
    assert_ne!(digest(&a), digest(&b));
    let md = report(&s, &b).await;
    assert!(!md.contains("defined at runtime"), "{md}");
}

#[tokio::test]
async fn every_sweep_records_a_chaos_digest_and_no_chaos_differs_from_some() {
    let s = start(&config(false)).await;
    let none = swept(&s, json!({})).await;
    let scenario = swept(&s, json!({ "scenario": "slow" })).await;
    assert_ne!(digest(&none), digest(&scenario));
    // the inline scenario `slow` is app +50 ms: the same chaos set live gives the same digest
    set_chaos(&s, json!({ "node": "app", "latency_ms": 50 })).await;
    let live = swept(&s, json!({})).await;
    assert_eq!(
        digest(&live),
        digest(&scenario),
        "the compiled chaos, not where it came from"
    );
}
