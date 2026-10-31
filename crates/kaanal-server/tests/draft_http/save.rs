//! Patches to the draft and Save: comments survive, the disk is untouched until Save, and Save
//! refuses rather than overwrite.

use crate::common::*;
use serde_json::{Value, json};

const URL: &str = "/__kaanal/draft";
const SAVE: &str = "/__kaanal/draft/save";

async fn patch(s: &Srv, ops: Value) -> reqwest::Response {
    s.put(URL, json!({ "patch": ops })).await
}

fn on_disk(s: &Srv) -> String {
    std::fs::read_to_string(s.dir.join("kaanal.yaml")).unwrap()
}

#[tokio::test]
async fn a_patch_keeps_comments_and_style() {
    let s = start(BASE).await;
    // raw text: `json!` would sort the keys of the value before they were sent
    let body = r#"{ "patch": [
        { "op": "set", "path": ["nodes", "app", "proc"], "value": "5ms" },
        { "op": "insert", "path": ["nodes", "cache"], "value": { "type": "cache", "flavor": "redis" } },
        { "op": "remove", "path": ["edges", 1] }
    ] }"#;
    let r = s
        .client
        .put(format!("{}{URL}", s.base))
        .bearer_auth(&s.token)
        .header("content-type", "application/json")
        .body(body)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 200);
    let v: Value = r.json().await.unwrap();
    let text = v["text"].as_str().unwrap();
    assert!(text.contains("# a comment the draft must keep"), "{text}");
    assert!(
        text.contains("app:   { type: service, proc: 5ms }"),
        "{text}"
    );
    assert!(
        text.contains("cache: { type: cache, flavor: redis }"),
        "{text}"
    );
    assert!(!text.contains("to: db"), "{text}");
    assert_eq!((&v["dirty"], &v["lost"]), (&json!(true), &json!([])));
    // the result is still a config the loader reads
    let r = s
        .send(reqwest::Method::POST, "/__kaanal/draft/validate", None)
        .await;
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["ok"], true, "{v}");
}

#[tokio::test]
async fn a_bad_patch_is_400_and_changes_nothing() {
    let s = start(BASE).await;
    for body in [
        json!({ "patch": [{ "op": "set", "path": ["nodes", "ghost", "proc"], "value": 1 }] }),
        json!({ "patch": [{ "op": "insert", "path": ["nodes", "app"], "value": 1 }] }),
        json!({ "patch": [{ "op": "set", "path": ["nodes", true], "value": 1 }] }),
        json!({ "patch": [{ "op": "set", "path": ["nodes"] }] }),
        json!({ "patch": [{ "op": "insert", "path": [0], "value": 1 }] }),
        json!({ "text": "a: 1\n", "patch": [] }),
        json!({}),
    ] {
        let r = s.put(URL, body.clone()).await;
        assert_eq!(r.status(), 400, "{body}");
    }
    // a body of the wrong shape is the extractor's 422, like every other route
    let r = patch(&s, json!([{ "op": "swap", "path": [] }])).await;
    assert_eq!(r.status(), 422);
    let v: Value = s.get(URL).await.json().await.unwrap();
    assert_eq!((&v["text"], &v["dirty"]), (&json!(BASE), &json!(false)));
}

#[tokio::test]
async fn the_disk_is_untouched_until_save() {
    let s = start(BASE).await;
    let before = std::fs::metadata(s.dir.join("kaanal.yaml"))
        .unwrap()
        .modified()
        .unwrap();
    let r = patch(&s, json!([{ "op": "set", "path": ["seed"], "value": 7 }])).await;
    assert_eq!(r.status(), 200);
    let r = s
        .send(reqwest::Method::POST, "/__kaanal/draft/apply", None)
        .await;
    assert_eq!(r.status(), 200);
    assert_eq!(on_disk(&s), BASE);
    let after = std::fs::metadata(s.dir.join("kaanal.yaml"))
        .unwrap()
        .modified()
        .unwrap();
    assert_eq!(before, after);
}

#[tokio::test]
async fn save_writes_the_draft_and_the_next_save_is_not_a_conflict() {
    let s = start(BASE).await;
    patch(&s, json!([{ "op": "set", "path": ["seed"], "value": 7 }])).await;
    let r = s.send(reqwest::Method::POST, SAVE, None).await;
    assert_eq!(r.status(), 200);
    let v: Value = r.json().await.unwrap();
    assert_eq!(
        (&v["ok"], &v["written"]),
        (&json!(true), &json!(["kaanal.yaml"]))
    );
    let want = BASE.replace("seed: 42", "seed: 7");
    assert_eq!(on_disk(&s), want);
    let names: Vec<_> = std::fs::read_dir(&s.dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    assert!(
        !names.iter().any(|n| n.contains("kaanal-")),
        "no temp file is left: {names:?}"
    );
    // we wrote it, so it is what we expect on disk next time
    patch(&s, json!([{ "op": "set", "path": ["seed"], "value": 8 }])).await;
    let r = s.send(reqwest::Method::POST, SAVE, None).await;
    assert_eq!(r.status(), 200);
    assert_eq!(on_disk(&s), BASE.replace("seed: 42", "seed: 8"));
}

#[tokio::test]
async fn save_without_confirm_lists_what_would_be_lost() {
    let text = "version: 1\nseed: &s 42\nnodes: {}\n";
    let s = start(BASE).await;
    s.put(URL, json!({ "text": text })).await;
    let r = patch(&s, json!([{ "op": "set", "path": ["seed"], "value": 7 }])).await;
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["lost"].as_array().unwrap().len(), 1, "{v}");
    let r = s.send(reqwest::Method::POST, SAVE, None).await;
    assert_eq!(r.status(), 409);
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["lost"].as_array().unwrap().len(), 1, "{v}");
    assert_eq!(on_disk(&s), BASE, "nothing was written");

    let r = s
        .send(
            reqwest::Method::POST,
            SAVE,
            Some(json!({ "confirm_loss": true })),
        )
        .await;
    assert_eq!(r.status(), 200);
    assert_eq!(on_disk(&s), text, "the draft is written as it stands");
    let v: Value = s.get(URL).await.json().await.unwrap();
    assert_eq!(v["lost"], json!([]), "saved, so nothing is pending");
}

#[tokio::test]
async fn replacing_the_text_clears_what_was_lost() {
    let s = start(BASE).await;
    s.put(URL, json!({ "text": "seed: &s 42\n" })).await;
    patch(&s, json!([{ "op": "set", "path": ["seed"], "value": 7 }])).await;
    let v: Value = s
        .put(URL, json!({ "text": BASE }))
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(v["lost"], json!([]));
}

#[tokio::test]
async fn save_refuses_when_the_file_changed_or_vanished_on_disk() {
    let s = start(BASE).await;
    patch(&s, json!([{ "op": "set", "path": ["seed"], "value": 7 }])).await;
    std::fs::write(
        s.dir.join("kaanal.yaml"),
        "version: 1\n# edited elsewhere\n",
    )
    .unwrap();
    let r = s.send(reqwest::Method::POST, SAVE, None).await;
    assert_eq!(r.status(), 409);
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["conflict"], true, "{v}");
    assert_eq!(
        on_disk(&s),
        "version: 1\n# edited elsewhere\n",
        "their edit is kept"
    );

    std::fs::remove_file(s.dir.join("kaanal.yaml")).unwrap();
    let r = s.send(reqwest::Method::POST, SAVE, None).await;
    assert_eq!(r.status(), 409);
    assert!(
        !s.dir.join("kaanal.yaml").exists(),
        "not recreated behind their back"
    );
}

#[tokio::test]
async fn save_with_no_config_file_is_409() {
    let s = start_with(BASE, None).await;
    let r = s.send(reqwest::Method::POST, SAVE, None).await;
    assert_eq!(r.status(), 409);
}

#[tokio::test]
async fn save_refuses_a_draft_that_names_a_file_outside_the_project() {
    let s = start(BASE).await;
    let outside = s.dir.join("openapi.yaml").display().to_string();
    for text in [
        with_spec(&format!("'{outside}'")),
        BASE.replace("nodes:", "data: ../other.yaml\nnodes:"),
        format!("{BASE}endpoints: '/etc/endpoints.yaml'\n"),
    ] {
        assert_eq!(s.put(URL, json!({ "text": text })).await.status(), 200);
        let r = s.send(reqwest::Method::POST, SAVE, None).await;
        assert_eq!(r.status(), 400, "{text}");
        let v: Value = r.json().await.unwrap();
        assert!(
            v["error"]
                .as_str()
                .unwrap()
                .contains("inside the project folder"),
            "{v}"
        );
        assert_eq!(on_disk(&s), BASE, "nothing was written");
    }
    // spellings a leaf walker would skip: an anchor, an alias, a block scalar
    let named = with_spec("&s '/etc/x.yaml'");
    for text in [
        named,
        with_spec("&s /etc/x.yaml").replace("  db:", "  app2:  { type: service, spec: *s }\n  db:"),
        BASE.replace("nodes:", "data: |\n  ../other.yaml\nnodes:"),
        BASE.replace("nodes:", "data: &d ../other.yaml\nnodes:"),
    ] {
        s.put(URL, json!({ "text": text })).await;
        let r = s.send(reqwest::Method::POST, SAVE, None).await;
        assert_eq!(r.status(), 400, "{text}");
        assert_eq!(on_disk(&s), BASE, "nothing was written");
    }
    // YAML that the loader would reject for another reason must still have its paths checked
    let other_error = BASE.replace(
        "app:   { type: service,",
        "app:   { type: service, bogus: 1, spec: /etc/x.yaml,",
    );
    s.put(URL, json!({ "text": other_error })).await;
    assert_eq!(
        s.send(reqwest::Method::POST, SAVE, None).await.status(),
        400
    );
    assert_eq!(on_disk(&s), BASE);
    // a node whose name is not text (the loader reads `1:` as the name "1"), a path that is a
    // number-like, tagged or list value: checked or refused, never skipped
    for text in [
        BASE.replace(
            "  app:   { type: service,",
            "  7:   { type: service, spec: /etc/x.yaml,",
        ),
        BASE.replace(
            "  app:   { type: service,",
            "  true:   { type: service, spec: ../x.yaml,",
        ),
        BASE.replace(
            "  app:   { type: service,",
            "  app:   { type: service, spec: !tag /etc/x.yaml,",
        ),
        BASE.replace(
            "  app:   { type: service,",
            "  app:   { type: service, spec: [/etc/x.yaml],",
        ),
        BASE.replace("nodes:", "data: { a: b }\nnodes:"),
    ] {
        s.put(URL, json!({ "text": text })).await;
        let r = s.send(reqwest::Method::POST, SAVE, None).await;
        assert_eq!(r.status(), 400, "{text}");
        assert_eq!(on_disk(&s), BASE, "nothing was written");
    }
    // a relative path inside the project and a draft that is not YAML at all can still be saved
    let ok = with_spec("openapi.yaml");
    s.put(URL, json!({ "text": ok })).await;
    assert_eq!(
        s.send(reqwest::Method::POST, SAVE, None).await.status(),
        200
    );
    s.put(URL, json!({ "text": "nodes: [" })).await;
    assert_eq!(
        s.send(reqwest::Method::POST, SAVE, None).await.status(),
        200
    );
    assert_eq!(on_disk(&s), "nodes: [");
}

/// BASE with `spec:` on the `app` service (the only kind of node in BASE that has one).
fn with_spec(spec: &str) -> String {
    BASE.replace(
        "app:   { type: service,",
        &format!("app:   {{ type: service, spec: {spec},"),
    )
}

#[tokio::test]
async fn saved_is_true_again_after_save() {
    let s = start(BASE).await;
    let r = patch(&s, json!([{ "op": "set", "path": ["seed"], "value": 7 }])).await;
    assert_eq!(r.status(), 200);
    let v: Value = s.get(URL).await.json().await.unwrap();
    assert_eq!(v["saved"], false);
    let r = s.send(reqwest::Method::POST, SAVE, None).await;
    assert_eq!(r.status(), 200);
    let v: Value = s.get(URL).await.json().await.unwrap();
    assert_eq!(v["saved"], true);
}
