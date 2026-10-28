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

/// A server with two scenario files, `f` (names `db`) and `ok` (names `app`), whose draft has
/// had `db` renamed to `store`: `f` is flagged until it is rewritten.
async fn renamed_project() -> Srv {
    let s = start(BASE).await;
    let dir = s.dir.join("scenarios");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("f.yaml"),
        "# the database\nchaos:\n  - { node: db, down: true }\n  - { edge: { from: app, to: db }, latency_ms: 5 }\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("ok.yaml"),
        "chaos:\n  - { node: app, latency_ms: 5 }\n",
    )
    .unwrap();
    let op = json!({ "op": "rename_node", "from": "db", "to": "store" });
    assert_eq!(
        s.put("/__kaanal/draft", json!({ "patch": [op] }))
            .await
            .status(),
        200
    );
    s
}

async fn save_with(s: &Srv, fix: &str) -> reqwest::Response {
    s.send(Method::POST, SAVE, Some(json!({ "fix_scenarios": fix })))
        .await
}

#[tokio::test]
async fn rewrite_follows_a_rename_into_the_scenario_files() {
    let s = renamed_project().await;
    // before: the draft validates but the file is flagged
    let r = s.send(Method::POST, "/__kaanal/draft/validate", None).await;
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["flagged"][0]["scenario"], "f", "{v}");
    let ok_before = std::fs::read(s.dir.join("scenarios/ok.yaml")).unwrap();

    let r = save_with(&s, "rewrite").await;
    assert_eq!(r.status(), 200);
    let v: Value = r.json().await.unwrap();
    let written: Vec<String> = v["written"]
        .as_array()
        .unwrap()
        .iter()
        .map(|w| w.as_str().unwrap().replace('\\', "/"))
        .collect();
    assert_eq!(written, ["kaanal.yaml", "scenarios/f.yaml"], "{v}");
    // only the names changed: the comment and the layout are the file's own
    let f = std::fs::read_to_string(s.dir.join("scenarios/f.yaml")).unwrap();
    assert_eq!(
        f,
        "# the database\nchaos:\n  - { node: store, down: true }\n  - { edge: { from: app, to: store }, latency_ms: 5 }\n"
    );
    assert_eq!(
        std::fs::read(s.dir.join("scenarios/ok.yaml")).unwrap(),
        ok_before,
        "an untouched file stays byte for byte"
    );
    let r = s.send(Method::POST, "/__kaanal/draft/validate", None).await;
    let v: Value = r.json().await.unwrap();
    assert_eq!((&v["ok"], &v["flagged"]), (&json!(true), &json!([])), "{v}");
    // the renames are spent: a second rewrite has nothing to do
    assert_eq!(save_with(&s, "rewrite").await.status(), 400);
}

#[tokio::test]
async fn renames_chain_and_a_whole_text_put_forgets_them() {
    let s = renamed_project().await;
    let op = json!({ "op": "rename_node", "from": "store", "to": "vault" });
    assert_eq!(
        s.put("/__kaanal/draft", json!({ "patch": [op] }))
            .await
            .status(),
        200
    );
    assert_eq!(save_with(&s, "rewrite").await.status(), 200);
    let f = std::fs::read_to_string(s.dir.join("scenarios/f.yaml")).unwrap();
    assert!(
        f.contains("node: vault") && f.contains("to: vault"),
        "db -> store -> vault: {f}"
    );

    // replacing the text is not a rename: nothing is known about the old names any more
    let s = renamed_project().await;
    let text = s
        .get("/__kaanal/draft")
        .await
        .json::<Value>()
        .await
        .unwrap()["text"]
        .clone();
    assert_eq!(
        s.put("/__kaanal/draft", json!({ "text": text }))
            .await
            .status(),
        200
    );
    assert_eq!(save_with(&s, "rewrite").await.status(), 400);
}

#[tokio::test]
async fn rewrite_without_a_rename_is_400_and_a_file_that_cannot_be_rewritten_stops_the_save() {
    let s = flagged_project().await;
    let r = save_with(&s, "rewrite").await;
    assert_eq!(r.status(), 400, "no rename_node was made");
    assert!(exists(&s, "scenarios/f.yaml"));
    // a name inside an anchored value cannot be changed safely: nothing is written
    let s = renamed_project().await;
    let anchored = "chaos: &c\n  - { node: db, down: true }\n";
    std::fs::write(s.dir.join("scenarios/f.yaml"), anchored).unwrap();
    let before = std::fs::read_to_string(s.dir.join("kaanal.yaml")).unwrap();
    let r = save_with(&s, "rewrite").await;
    assert_eq!(r.status(), 400);
    let v: Value = r.json().await.unwrap();
    assert!(
        v["error"].as_str().unwrap().contains("nothing was saved"),
        "{v}"
    );
    assert_eq!(
        std::fs::read_to_string(s.dir.join("scenarios/f.yaml")).unwrap(),
        anchored
    );
    assert_eq!(
        std::fs::read_to_string(s.dir.join("kaanal.yaml")).unwrap(),
        before
    );
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
