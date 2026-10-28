//! Cases the review of the draft/apply step turned up: runtime scenarios, link caps, validate
//! against Apply, a draft edited during an Apply, traffic across swaps and the body limit.

use crate::common::*;
use reqwest::Method;
use serde_json::{Value, json};
use std::time::{Duration, Instant};

const DRAFT: &str = "/__kaanal/draft";

async fn put_draft(s: &Srv, text: &str) -> reqwest::Response {
    s.put(DRAFT, json!({ "text": text })).await
}

async fn apply(s: &Srv) -> Value {
    let r = s.send(Method::POST, "/__kaanal/draft/apply", None).await;
    assert_eq!(r.status(), 200);
    r.json().await.unwrap()
}

#[tokio::test]
async fn a_runtime_scenario_survives_an_apply_and_yields_to_a_draft_definition() {
    let s = start(&system("1ms")).await;
    let sc = json!({ "chaos": [ { "node": "db", "latency_ms": 5 } ] });
    assert_eq!(s.put("/__kaanal/scenarios/x", sc).await.status(), 200);
    let r = s
        .send(Method::POST, "/__kaanal/scenarios/x/activate", None)
        .await;
    assert_eq!(r.status(), 200);
    // same system, nodes reordered: db changes id, the scenario must still name db
    let reordered = "version: 1\nseed: 42\nnodes:\n  db:    { type: database, ops: { read: 1ms, write: 1ms, scan: 1ms } }\n  app:   { type: service, spec: openapi.yaml, proc: 1ms }\n  nginx: { type: gateway, proc: 1ms }\nedges:\n  - { from: nginx, to: app, net: 1ms }\n  - { from: app, to: db, net: 1ms }\n";
    assert_eq!(put_draft(&s, reordered).await.status(), 200);
    let v = apply(&s).await;
    assert_eq!(v["kept"]["runtime_scenarios"], json!(["x"]), "{v}");
    assert_eq!(v["kept"]["active_scenarios"], json!(["x"]));
    let got: Value = s.get("/__kaanal/scenarios/x").await.json().await.unwrap();
    assert_eq!(got["source"], "runtime", "{got}");
    assert_eq!(got["chaos"][0]["node"], "db", "{got}");
    // now the draft defines `x` itself: its definition wins and the swap says so
    let inline =
        format!("{reordered}scenarios:\n  x: {{ chaos: [ {{ node: app, latency_ms: 3 }} ] }}\n");
    assert_eq!(put_draft(&s, &inline).await.status(), 200);
    let v = apply(&s).await;
    assert_eq!(v["kept"]["runtime_scenarios"], json!([]), "{v}");
    assert!(
        v["dropped"]["scenario_chaos"][0]
            .as_str()
            .unwrap()
            .contains("replaced by the draft"),
        "{v}"
    );
    let got: Value = s.get("/__kaanal/scenarios/x").await.json().await.unwrap();
    assert_eq!(got["source"], "inline");
}

#[tokio::test]
async fn a_link_cap_is_dropped_when_the_edge_loses_its_bandwidth() {
    let linked = system("1ms").replace(
        "{ from: app, to: db, net: 1ms }",
        "{ from: app, to: db, net: 1ms, bandwidth: 1000000 }",
    );
    let s = start(&linked).await;
    let body = json!({ "edge": { "from": "app", "to": "db" }, "bandwidth_cap": 500_000, "latency_ms": 10 });
    assert_eq!(s.put("/__kaanal/chaos", body).await.status(), 200);
    // same bandwidth: the cap stays
    let v = apply(&s).await;
    assert_eq!(v["dropped"]["live_chaos"], json!([]), "{v}");
    // no bandwidth any more: the cap goes, the latency stays
    assert_eq!(put_draft(&s, &system("1ms")).await.status(), 200);
    let v = apply(&s).await;
    let dropped = v["dropped"]["live_chaos"].as_array().unwrap();
    assert!(
        dropped
            .iter()
            .any(|l| l.as_str().unwrap().contains("bandwidth_cap")),
        "{v}"
    );
    let c: Value = s.get("/__kaanal/chaos").await.json().await.unwrap();
    assert_eq!(c["edges"][0]["latency_ms"], 10, "{c}");
    assert!(c["edges"][0].get("bandwidth_cap").is_none(), "{c}");
}

#[tokio::test]
async fn validate_and_apply_agree_when_an_auth_node_has_no_key() {
    let s = start(&system("1ms")).await;
    let with_auth = system("1ms").replace("edges:", "  login: { type: auth }\nedges:");
    assert_eq!(put_draft(&s, &with_auth).await.status(), 200);
    let v: Value = s
        .send(Method::POST, "/__kaanal/draft/validate", None)
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(
        v["ok"], false,
        "validate must not say ok where apply fails: {v}"
    );
    assert!(
        v["diagnostics"][0]["message"]
            .as_str()
            .unwrap()
            .contains("signing key"),
        "{v}"
    );
    let r = s.send(Method::POST, "/__kaanal/draft/apply", None).await;
    assert_eq!(r.status(), 400);
}

#[tokio::test]
async fn a_draft_edited_during_an_apply_stays_dirty_with_its_new_text() {
    let slow = system("800ms");
    let s = start(&slow).await;
    let (base, client) = (s.base.clone(), s.client.clone());
    let inflight = tokio::spawn(async move {
        client
            .get(format!("{base}/users/1"))
            .send()
            .await
            .unwrap()
            .status()
            .as_u16()
    });
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert_eq!(put_draft(&s, &system("1ms")).await.status(), 200);
    let (base, token, client) = (s.base.clone(), s.token.clone(), s.client.clone());
    let applier = tokio::spawn(async move {
        client
            .post(format!("{base}/__kaanal/draft/apply"))
            .bearer_auth(token)
            .send()
            .await
            .unwrap()
            .status()
            .as_u16()
    });
    tokio::time::sleep(Duration::from_millis(150)).await;
    let edited = format!("{}# edited while applying\n", system("1ms"));
    assert_eq!(put_draft(&s, &edited).await.status(), 200);
    assert_eq!(applier.await.unwrap(), 200);
    assert_eq!(inflight.await.unwrap(), 200);
    let d: Value = s.get(DRAFT).await.json().await.unwrap();
    assert_eq!(
        d["text"], edited,
        "the edit made during the Apply is not lost"
    );
    assert_eq!(
        (d["dirty"].as_bool(), d["generation"].as_u64()),
        (Some(true), Some(2))
    );
}

#[tokio::test]
async fn traffic_never_sees_a_gap_across_swaps() {
    let s = start(&system("1ms")).await;
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mut tasks = Vec::new();
    for _ in 0..6 {
        let (base, client, stop) = (s.base.clone(), s.client.clone(), stop.clone());
        tasks.push(tokio::spawn(async move {
            let mut statuses = Vec::new();
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                let r = client.get(format!("{base}/users/1")).send().await.unwrap();
                statuses.push(r.status().as_u16());
            }
            statuses
        }));
    }
    for proc in ["2ms", "1ms", "3ms"] {
        // let the load run against the current system before each swap
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(put_draft(&s, &system(proc)).await.status(), 200);
        let t = Instant::now();
        apply(&s).await;
        assert!(
            t.elapsed() < Duration::from_secs(5),
            "a swap under light load is quick"
        );
    }
    // and after the last one, on the final system
    tokio::time::sleep(Duration::from_millis(300)).await;
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    let mut all = Vec::new();
    for t in tasks {
        all.extend(t.await.unwrap());
    }
    assert!(all.len() > 30, "the load really ran: {}", all.len());
    assert!(
        all.iter().all(|s| *s == 200),
        "every request answered 200 across three swaps: {all:?}"
    );
}

#[tokio::test]
async fn a_draft_of_escapes_is_judged_by_the_text_limit_not_the_body_limit() {
    let s = start(&system("1ms")).await;
    // every newline is `\n` (two bytes) in the JSON body: 1 MiB of them is 2 MiB on the wire
    let at_limit = "\n".repeat(1024 * 1024);
    assert_eq!(put_draft(&s, &at_limit).await.status(), 200);
    let over = "\n".repeat(1024 * 1024 + 1);
    let r = put_draft(&s, &over).await;
    assert_eq!(
        r.status(),
        400,
        "the documented 400, not a 413 from the body limit"
    );
    let v: Value = r.json().await.unwrap();
    assert!(v["error"].as_str().unwrap().contains("1048576"), "{v}");
}
