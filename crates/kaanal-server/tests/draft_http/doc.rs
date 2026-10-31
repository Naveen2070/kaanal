//! The draft as JSON (`doc`), `project_id`, `kind_defaults` and `GET /__kaanal/demo`.

use crate::common::*;
use serde_json::{Value, json};

const URL: &str = "/__kaanal/draft";

async fn draft(s: &Srv) -> Value {
    s.get(URL).await.json().await.unwrap()
}

#[tokio::test]
async fn get_draft_carries_the_parsed_doc() {
    let s = start(BASE).await;
    let v = draft(&s).await;
    assert_eq!(v["doc_error"], Value::Null);
    assert_eq!(v["doc"]["nodes"]["nginx"]["type"], "gateway");
    assert_eq!(v["doc"]["nodes"]["db"]["type"], "database");
    assert_eq!(v["doc"]["edges"][1]["to"], "db");
    assert_eq!(v["rev"], 0, "rev stays");
}

#[tokio::test]
async fn a_half_typed_draft_has_a_null_doc_and_an_error() {
    let s = start(BASE).await;
    let r = s
        .put(URL, json!({ "text": "version: 1\nnodes: [\n" }))
        .await;
    assert_eq!(r.status(), 200);
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["doc"], Value::Null);
    assert!(v["doc_error"]["line"].is_number(), "{v}");
    assert!(v["doc_error"]["message"].is_string());
}

#[tokio::test]
async fn a_bom_draft_has_a_doc() {
    let s = start(BASE).await;
    let text = format!("\u{feff}{BASE}");
    let v: Value = s
        .put(URL, json!({ "text": text }))
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(v["doc"]["nodes"]["app"]["type"], "service", "{v}");
}

#[tokio::test]
async fn doc_is_the_draft_not_the_applied_system() {
    let s = start(BASE).await;
    let before: Value = s.get("/__kaanal/topology").await.json().await.unwrap();
    let text = BASE.replace("app:   { type: service", "api:   { type: service");
    let v: Value = s
        .put(URL, json!({ "text": text }))
        .await
        .json()
        .await
        .unwrap();
    assert!(v["doc"]["nodes"]["api"].is_object(), "{v}");
    assert!(v["doc"]["nodes"]["app"].is_null());
    let after: Value = s.get("/__kaanal/topology").await.json().await.unwrap();
    assert_eq!(before, after, "the applied system did not move");
}

#[tokio::test]
async fn a_patch_response_carries_the_new_doc() {
    let s = start(BASE).await;
    let v: Value = s
        .put(
            URL,
            json!({ "patch": [{ "op": "set", "path": ["nodes", "app", "proc"], "value": "9ms" }] }),
        )
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(v["doc"]["nodes"]["app"]["proc"], "9ms", "{v}");
}

#[tokio::test]
async fn project_id_is_16_hex_and_does_not_contain_the_path() {
    let s = start(BASE).await;
    let id = draft(&s).await["project_id"].as_str().unwrap().to_string();
    assert_eq!(id.len(), 16);
    assert!(id.chars().all(|c| c.is_ascii_hexdigit()), "{id}");
    assert!(!id.contains(&s.dir.to_string_lossy().to_string()));
    assert_eq!(draft(&s).await["project_id"], id, "stable");
    let none = start_with(BASE, None).await;
    assert_eq!(draft(&none).await["project_id"], "none");
}

#[tokio::test]
async fn project_name_is_the_config_folders_name_and_never_a_path() {
    let s = start(BASE).await;
    let name = draft(&s).await["project_name"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(name, s.dir.file_name().unwrap().to_string_lossy());
    assert!(!name.contains(['/', '\\']), "{name}");
    let none = start_with(BASE, None).await;
    assert!(draft(&none).await["project_name"].is_null());
}

#[tokio::test]
async fn kind_defaults_has_an_entry_for_every_node_kind() {
    let s = start(BASE).await;
    let v = draft(&s).await;
    for kind in [
        "gateway",
        "service",
        "monolith",
        "database",
        "cache",
        "external-api",
        "auth",
    ] {
        assert_eq!(v["kind_defaults"][kind]["type"], kind, "{kind}: {v}");
    }
    assert_eq!(v["kind_defaults"]["gateway"]["concurrency"], 256);
    assert_eq!(v["kind_defaults"]["database"]["pool"]["size"], 20);
}

#[tokio::test]
async fn the_demo_builds_validates_and_runs_in_an_empty_folder() {
    // nothing in the folder but the draft: no spec, no site, no other file
    let s = start_blank_in(new_dir()).await;
    let v: Value = s.get("/__kaanal/demo").await.json().await.unwrap();
    let text = v["text"].as_str().unwrap();
    assert!(!text.contains("spec:") && !text.contains("site:"), "{text}");
    let r = s.put(URL, json!({ "text": text })).await;
    assert_eq!(r.status(), 200);
    let r: Value = s
        .send(
            reqwest::Method::POST,
            "/__kaanal/draft/validate",
            Some(json!({ "text": text })),
        )
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(r["ok"], true, "{r}");
    let d = draft(&s).await;
    assert_eq!(d["doc_error"], Value::Null, "{d}");
    for (name, kind) in [
        ("nginx", "gateway"),
        ("shop", "service"),
        ("search", "service"),
        ("db", "database"),
        ("cache", "cache"),
    ] {
        assert_eq!(d["doc"]["nodes"][name]["type"], kind, "{d}");
        assert!(d["kind_defaults"][kind].is_object(), "{kind}");
    }
    let run = s
        .send(
            reqwest::Method::POST,
            "/__kaanal/sim/run",
            Some(json!({ "source": "draft", "profile": "peak" })),
        )
        .await;
    assert_eq!(run.status(), 202, "{}", run.text().await.unwrap());
    let id = run.json::<Value>().await.unwrap()["id"]
        .as_str()
        .unwrap()
        .to_string();
    for _ in 0..1200 {
        let rec: Value = s
            .get(&format!("/__kaanal/sim/runs/{id}"))
            .await
            .json()
            .await
            .unwrap();
        if rec["state"] != "running" {
            assert_eq!(rec["state"], "done", "{rec}");
            assert!(
                !rec["summary"]["nodes"].as_array().unwrap().is_empty(),
                "{rec}"
            );
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("the demo run did not finish");
}

/// A "billion laughs" draft of about `bytes`: each anchor lists the previous one ten times.
fn bomb(bytes: usize) -> String {
    use std::fmt::Write as _;
    let mut text = String::from(
        "version: 1\nnodes: {}\nx0: &a0 [lol, lol, lol, lol, lol, lol, lol, lol, lol, lol]\n",
    );
    let mut n = 1;
    while text.len() < bytes {
        let prev = format!("*a{}", n - 1);
        let _ = writeln!(text, "x{n}: &a{n} [{}]", vec![prev; 10].join(", "));
        n += 1;
    }
    text
}

/// Runs `body` with a 15 s limit. On a hang the process exits at once: a runtime blocked inside a
/// handler could not be dropped, so a plain panic would hang the test run instead of failing it.
async fn bounded(body: impl std::future::Future<Output = ()>) {
    if tokio::time::timeout(std::time::Duration::from_secs(15), body)
        .await
        .is_err()
    {
        eprintln!(
            "FAIL: the alias bomb was not refused within 15 s (the guard is gone or bypassed)"
        );
        std::process::exit(1);
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn an_alias_bomb_draft_stays_cheap() {
    bounded(async {
        let srv = start(BASE).await;
        let resp = srv.put(URL, json!({ "text": bomb(1_000_000) })).await;
        assert_eq!(resp.status(), 200);
        let t0 = std::time::Instant::now();
        let resp = srv.get(URL).await;
        assert_eq!(resp.status(), 200);
        let body: Value = resp.json().await.unwrap();
        eprintln!("alias bomb GET /draft took {:?}", t0.elapsed());
        assert_eq!(body["doc"], Value::Null);
        let msg = body["doc_error"]["message"].as_str().unwrap();
        assert!(msg.contains("alias expansion"), "{msg}");
        // the same bomb as a second document
        let two = format!("version: 1\nnodes: {{}}\n---\n{}", bomb(2_000));
        let body: Value = srv
            .put(URL, json!({ "text": two }))
            .await
            .json()
            .await
            .unwrap();
        assert!(
            body["doc_error"]["message"]
                .as_str()
                .unwrap()
                .contains("alias expansion")
        );
    })
    .await;
}

#[tokio::test(flavor = "multi_thread")]
async fn validate_apply_and_a_draft_run_refuse_an_alias_bomb_and_free_their_permits() {
    bounded(async {
        let srv = start(&system("1ms")).await;
        let post =
            |path: &'static str, body: Value| srv.send(reqwest::Method::POST, path, Some(body));
        let run = || {
            json!({ "profile": { "rps": 100, "journeys": [ { "endpoint": "GET /users/{id}" } ] },
                "duration_s": 10, "source": "draft" })
        };
        srv.put(URL, json!({ "text": bomb(1_000_000) })).await;
        let t0 = std::time::Instant::now();
        // many rounds: with two work permits, a leak or a stuck parse would block the third
        for _ in 0..4 {
            let v: Value = srv
                .send(reqwest::Method::POST, "/__kaanal/draft/validate", None)
                .await
                .json()
                .await
                .unwrap();
            assert_eq!(v["ok"], false, "{v}");
            let r = post("/__kaanal/draft/apply", json!({})).await;
            assert_eq!(r.status(), 400);
            let v: Value = r.json().await.unwrap();
            assert!(
                v["error"].as_str().unwrap().contains("alias expansion"),
                "{v}"
            );
            let r = post("/__kaanal/sim/run", run()).await;
            assert_eq!(r.status(), 400);
            let v: Value = r.json().await.unwrap();
            assert!(v.to_string().contains("alias expansion"), "{v}");
        }
        eprintln!("bomb validate/apply/run x4 took {:?}", t0.elapsed());
        // and a normal draft runs afterwards
        srv.put(URL, json!({ "text": system("2ms") })).await;
        assert_eq!(post("/__kaanal/sim/run", run()).await.status(), 202);
    })
    .await;
}

#[tokio::test]
async fn the_cached_doc_follows_every_change_of_the_text() {
    let s = start(BASE).await;
    let first = draft(&s).await;
    assert_eq!(
        draft(&s).await["doc"],
        first["doc"],
        "a second GET is the same"
    );
    // broken, then fixed, then a patch: each change must show, never a stale parse
    let v: Value = s
        .put(URL, json!({ "text": "nodes: [\n" }))
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(v["doc"], Value::Null);
    assert_eq!(draft(&s).await["doc"], Value::Null);
    let v: Value = s
        .put(URL, json!({ "text": BASE }))
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(v["doc"], first["doc"]);
    assert_eq!(draft(&s).await["doc"], first["doc"]);
}

#[tokio::test]
async fn boot_is_16_hex_and_steady_across_calls() {
    let s = start(BASE).await;
    let (a, b) = (draft(&s).await, draft(&s).await);
    let boot = a["boot"].as_str().expect("boot is text");
    assert_eq!(boot.len(), 16, "{a}");
    assert!(boot.bytes().all(|c| c.is_ascii_hexdigit()), "{boot}");
    assert_eq!(a["boot"], b["boot"]);
    let v: Value = s
        .put(URL, json!({ "text": BASE }))
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(v["boot"], a["boot"], "PUT answers with the same view");
}

#[tokio::test]
async fn append_adds_one_item_to_a_list_and_keeps_the_rest_of_the_text() {
    let s = start(BASE).await;
    let r = s
        .put(
            URL,
            json!({ "patch": [
                { "op": "append", "path": ["edges"], "value": { "from": "nginx", "to": "db" } },
            ] }),
        )
        .await;
    assert_eq!(r.status(), 200);
    let v: Value = r.json().await.unwrap();
    let want = format!("{BASE}  - {{ from: nginx, to: db }}\n");
    assert_eq!(v["text"], want.as_str(), "{v}");
    assert_eq!(v["doc"]["edges"][2]["to"], "db");
    assert_eq!(v["lost"], json!([]));
    // not a list, or missing: refused, the draft unchanged
    for path in [json!(["nodes"]), json!(["nope"])] {
        s.put(
            URL,
            json!({ "patch": [ { "op": "append", "path": path, "value": 1 } ] }),
        )
        .await;
        assert_eq!(draft(&s).await["text"], want.as_str(), "{path}");
    }
}
