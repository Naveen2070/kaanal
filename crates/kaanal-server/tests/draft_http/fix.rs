//! Save with `fix_scenarios`: scenario files the draft no longer resolves against.

use crate::common::*;
use reqwest::Method;
use serde_json::{Value, json};

const SAVE: &str = "/__kaanal/draft/save";

/// A server whose project has two scenario files, `f` (names `db`) and `ok` (names `app`), and a
/// draft with `db` removed, so `f` is flagged.
async fn flagged_project() -> Srv {
    let s = start(BASE).await;
    let dir = s.dir.join("scenarios");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("f.yaml"), "chaos:\n  - { node: db, down: true }\n").unwrap();
    std::fs::write(
        dir.join("ok.yaml"),
        "chaos:\n  - { node: app, latency_ms: 5 }\n",
    )
    .unwrap();
    let r = s
        .put(
            "/__kaanal/draft",
            json!({ "patch": [
                { "op": "remove", "path": ["edges", 1] },
                { "op": "remove", "path": ["nodes", "db"] },
            ] }),
        )
        .await;
    assert_eq!(r.status(), 200);
    s
}

fn exists(s: &Srv, rel: &str) -> bool {
    s.dir.join(rel).exists()
}

#[tokio::test]
async fn remove_deletes_only_the_flagged_files_and_the_next_load_works() {
    let s = flagged_project().await;
    let r = s
        .send(
            Method::POST,
            SAVE,
            Some(json!({ "fix_scenarios": "remove" })),
        )
        .await;
    assert_eq!(r.status(), 200);
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["written"], json!(["kaanal.yaml"]), "{v}");
    let removed = v["removed"][0].as_str().unwrap().replace('\\', "/");
    assert_eq!(removed, "scenarios/f.yaml", "{v}");
    assert!(!exists(&s, "scenarios/f.yaml"));
    assert!(
        exists(&s, "scenarios/ok.yaml"),
        "a file that still resolves stays"
    );
    // the saved project is strict-clean: the draft validates with nothing flagged
    let r = s.send(Method::POST, "/__kaanal/draft/validate", None).await;
    let v: Value = r.json().await.unwrap();
    assert_eq!((&v["ok"], &v["flagged"]), (&json!(true), &json!([])), "{v}");
}

#[tokio::test]
async fn without_a_fix_the_flagged_file_stays() {
    let s = flagged_project().await;
    for body in [None, Some(json!({ "fix_scenarios": "none" }))] {
        let r = s.send(Method::POST, SAVE, body).await;
        assert_eq!(r.status(), 200);
        assert!(exists(&s, "scenarios/f.yaml"));
    }
}

#[tokio::test]
async fn rewrite_is_refused_until_rename_exists() {
    let s = flagged_project().await;
    let r = s
        .send(
            Method::POST,
            SAVE,
            Some(json!({ "fix_scenarios": "rewrite" })),
        )
        .await;
    assert_eq!(r.status(), 400);
    assert!(exists(&s, "scenarios/f.yaml"));
    let r = s
        .send(
            Method::POST,
            SAVE,
            Some(json!({ "fix_scenarios": "nonsense" })),
        )
        .await;
    assert_eq!(r.status(), 422);
}

#[tokio::test]
async fn a_refused_save_deletes_nothing() {
    let s = flagged_project().await;
    std::fs::write(
        s.dir.join("kaanal.yaml"),
        "version: 1\n# edited elsewhere\n",
    )
    .unwrap();
    let r = s
        .send(
            Method::POST,
            SAVE,
            Some(json!({ "fix_scenarios": "remove" })),
        )
        .await;
    assert_eq!(r.status(), 409, "the file changed on disk");
    assert!(exists(&s, "scenarios/f.yaml"));
}

#[tokio::test]
async fn remove_on_a_draft_that_does_not_build_is_400() {
    let s = flagged_project().await;
    s.put("/__kaanal/draft", json!({ "text": "nodes: [" }))
        .await;
    let r = s
        .send(
            Method::POST,
            SAVE,
            Some(json!({ "fix_scenarios": "remove" })),
        )
        .await;
    assert_eq!(r.status(), 400);
    assert!(exists(&s, "scenarios/f.yaml"));
}
