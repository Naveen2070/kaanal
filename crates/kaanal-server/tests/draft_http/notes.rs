//! Canvas notes (`canvas_notes`): kept in the layout sidecar beside the positions, never bumping
//! `rev`, written by Save, read again by Reload, and untouched by a rename.

use crate::common::*;
use reqwest::Method;
use serde_json::{Value, json};
use std::path::Path;

const URL: &str = "/__kaanal/draft";
const SAVE: &str = "/__kaanal/draft/save";
const RELOAD: &str = "/__kaanal/draft/reload";
const SIDECAR: &str = "kaanal.layout.json";

async fn view(s: &Srv) -> Value {
    s.get(URL).await.json().await.unwrap()
}

async fn save(s: &Srv) -> Value {
    let r = s.send(Method::POST, SAVE, None).await;
    assert_eq!(r.status(), 200);
    r.json().await.unwrap()
}

fn sidecar(dir: &Path) -> Value {
    serde_json::from_str(&std::fs::read_to_string(dir.join(SIDECAR)).unwrap()).unwrap()
}

fn note(n: u32, text: &str) -> Value {
    json!({ "id": format!("note:{n}"), "text": text, "at": [10.0, 20.0] })
}

async fn put_notes(s: &Srv, notes: Value) -> reqwest::Response {
    s.put(URL, json!({ "canvas_notes": notes })).await
}

#[tokio::test]
async fn a_draft_has_no_notes_until_put() {
    let s = start(BASE).await;
    assert_eq!(view(&s).await["canvas_notes"], json!([]));
}

#[tokio::test]
async fn put_notes_replaces_the_list_and_does_not_bump_rev() {
    let s = start(BASE).await;
    let rev = view(&s).await["rev"].clone();
    let two = json!([note(1, "a"), note(2, "b")]);
    let v: Value = put_notes(&s, two.clone()).await.json().await.unwrap();
    assert_eq!(v["canvas_notes"], two);
    assert_eq!((&v["rev"], &v["layout_dirty"]), (&rev, &json!(true)));
    let one = json!([note(2, "b")]);
    let v: Value = put_notes(&s, one.clone()).await.json().await.unwrap();
    assert_eq!(v["canvas_notes"], one, "the whole list is replaced");
}

#[tokio::test]
async fn notes_alone_are_a_valid_body_and_an_empty_one_is_still_400() {
    let s = start(BASE).await;
    assert_eq!(put_notes(&s, json!([])).await.status(), 200);
    assert_eq!(s.put(URL, json!({})).await.status(), 400);
}

#[tokio::test]
async fn notes_that_cannot_be_kept_are_a_400_and_change_nothing() {
    let s = start(BASE).await;
    let keep = json!([note(1, "kept")]);
    assert_eq!(put_notes(&s, keep.clone()).await.status(), 200);
    let many: Vec<Value> = (0..51).map(|i| note(i, "x")).collect();
    for bad in [
        json!(many),
        json!([note(1, "a"), note(1, "b")]),
        json!([{ "id": "app", "text": "", "at": [0, 0] }]),
        json!([{ "id": "note:1", "text": "", "at": [1e7, 0] }]),
        json!([note(1, &"a".repeat(501))]),
    ] {
        assert_eq!(put_notes(&s, bad.clone()).await.status(), 400, "{bad}");
    }
    // an unknown key or a wrong shape is the extractor's refusal
    let r = put_notes(
        &s,
        json!([{ "id": "note:1", "text": "", "at": [0, 0], "x": 1 }]),
    )
    .await;
    assert!(matches!(r.status().as_u16(), 400 | 422));
    assert_eq!(view(&s).await["canvas_notes"], keep);
    // the edges are allowed
    let fifty: Vec<Value> = (0..50).map(|i| note(i, &"é".repeat(500))).collect();
    assert_eq!(put_notes(&s, json!(fifty)).await.status(), 200);
}

#[tokio::test]
async fn positions_and_notes_together_must_fit_the_file() {
    let s = start(BASE).await;
    // 2000 long names: the positions alone fit, and so do 50 notes of control characters
    // (six bytes of JSON each), but not both in one file
    let layout: serde_json::Map<String, Value> = (0..2000)
        .map(|i| (format!("{i:0>80}"), json!([0, 0])))
        .collect();
    assert_eq!(s.put(URL, json!({ "layout": layout })).await.status(), 200);
    let heavy: Vec<Value> = (0..50).map(|i| note(i, &"\u{1}".repeat(500))).collect();
    let r = put_notes(&s, json!(heavy)).await;
    assert_eq!(r.status(), 400);
    let body = r.text().await.unwrap();
    assert!(body.contains("positions and notes"), "{body}");
    assert_eq!(view(&s).await["canvas_notes"], json!([]));
}

#[tokio::test]
async fn notes_make_the_draft_dirty_and_save_writes_and_clears_it() {
    let s = start(BASE).await;
    let notes = json!([note(1, "the cache is cold on boot")]);
    put_notes(&s, notes.clone()).await;
    assert_eq!(view(&s).await["saved"], false);
    let r = save(&s).await;
    assert_eq!(r["layout_written"], true, "{r}");
    let v = view(&s).await;
    assert_eq!(
        (&v["layout_dirty"], &v["saved"]),
        (&json!(false), &json!(true))
    );
    assert_eq!(
        sidecar(&s.dir),
        json!({ "version": 1, "nodes": {}, "notes": notes })
    );
}

#[tokio::test]
async fn a_fresh_draft_reads_the_saved_notes() {
    let dir = new_dir();
    let s = start_at(dir.clone(), &dir, BASE, BASE).await;
    let notes = json!([note(1, "a"), note(2, "b")]);
    put_notes(&s, notes.clone()).await;
    save(&s).await;
    let again = start_at(dir.clone(), &dir, BASE, BASE).await;
    let v = view(&again).await;
    assert_eq!(
        (&v["canvas_notes"], &v["layout_note"]),
        (&notes, &Value::Null)
    );
    assert_eq!(v["layout_dirty"], false);
}

#[tokio::test]
async fn a_file_without_notes_has_no_notes_key_and_loads_as_before() {
    let dir = new_dir();
    std::fs::write(dir.join(SIDECAR), br#"{"version":1,"nodes":{"app":[1,2]}}"#).unwrap();
    let s = start_at(dir.clone(), &dir, BASE, BASE).await;
    let v = view(&s).await;
    assert_eq!(
        (&v["canvas_notes"], &v["layout"]),
        (&json!([]), &json!({ "app": [1.0, 2.0] }))
    );
    // positions only: the file Save writes has no notes key
    s.put(URL, json!({ "layout": { "app": [3, 4] } })).await;
    save(&s).await;
    assert_eq!(
        sidecar(&s.dir),
        json!({ "version": 1, "nodes": { "app": [3.0, 4.0] } })
    );
}

#[tokio::test]
async fn deleting_the_last_note_rewrites_the_file_without_notes() {
    let s = start(BASE).await;
    put_notes(&s, json!([note(1, "x")])).await;
    save(&s).await;
    assert_eq!(sidecar(&s.dir)["notes"], json!([note(1, "x")]));
    put_notes(&s, json!([])).await;
    assert_eq!(view(&s).await["layout_dirty"], true);
    let r = save(&s).await;
    assert_eq!(r["layout_written"], true, "{r}");
    assert_eq!(sidecar(&s.dir), json!({ "version": 1, "nodes": {} }));
}

#[tokio::test]
async fn a_file_with_bad_notes_is_refused_whole_with_a_note() {
    let dir = new_dir();
    let bad = r#"{"version":1,"nodes":{"app":[1,2]},"notes":[{"id":"app","text":"","at":[0,0]}]}"#;
    std::fs::write(dir.join(SIDECAR), bad).unwrap();
    let s = start_at(dir.clone(), &dir, BASE, BASE).await;
    let v = view(&s).await;
    assert_eq!((&v["canvas_notes"], &v["layout"]), (&json!([]), &json!({})));
    let note = v["layout_note"].as_str().unwrap();
    assert!(
        note.contains("was not used") && note.contains("notes"),
        "{note}"
    );
}

#[tokio::test]
async fn reload_reads_the_notes_again() {
    let s = start(BASE).await;
    put_notes(&s, json!([note(1, "unsaved")])).await;
    let on_disk = json!({ "version": 1, "nodes": {}, "notes": [note(7, "from disk")] });
    std::fs::write(s.dir.join(SIDECAR), on_disk.to_string()).unwrap();
    let r = s.send(Method::POST, RELOAD, None).await;
    assert_eq!(r.status(), 200);
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["canvas_notes"], json!([note(7, "from disk")]));
    assert_eq!(v["layout_dirty"], false);
}

#[tokio::test]
async fn a_rename_and_removing_every_node_leave_the_notes_alone() {
    let s = start(BASE).await;
    let notes = json!([note(1, "about the db")]);
    put_notes(&s, notes.clone()).await;
    let op = json!({ "op": "rename_node", "from": "db", "to": "store" });
    let v: Value = s
        .put(URL, json!({ "patch": [op] }))
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(v["canvas_notes"], notes);
    let v: Value = s
        .put(URL, json!({ "text": "version: 1\n" }))
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(v["canvas_notes"], notes);
}

#[tokio::test]
async fn a_sidecar_that_was_not_used_is_not_replaced_by_save() {
    let dir = new_dir();
    // one bad note id: the whole file is refused on load, so its positions and notes were never read
    let bad = br#"{"version":1,"nodes":{"app":[1,2]},"notes":[{"id":"app","text":"keep me","at":[0,0]}]}"#;
    std::fs::write(dir.join(SIDECAR), bad).unwrap();
    let s = start_at(dir.clone(), &dir, BASE, BASE).await;
    put_notes(&s, json!([note(1, "new")])).await;
    let r = save(&s).await;
    assert_eq!(r["layout_written"], false, "{r}");
    assert!(
        r["layout_note"].as_str().unwrap().contains("not replaced"),
        "{r}"
    );
    assert_eq!(std::fs::read(dir.join(SIDECAR)).unwrap(), bad.as_slice());
    // the way out: delete the file and Reload, and Save works again
    std::fs::remove_file(dir.join(SIDECAR)).unwrap();
    assert_eq!(s.send(Method::POST, RELOAD, None).await.status(), 200);
    put_notes(&s, json!([note(1, "new")])).await;
    assert_eq!(save(&s).await["layout_written"], true);
    assert_eq!(sidecar(&dir)["notes"], json!([note(1, "new")]));
}

#[tokio::test]
async fn a_rename_that_would_make_the_file_too_big_is_refused_and_changes_nothing() {
    let s = start(BASE).await;
    let wide = |pad: usize| -> serde_json::Map<String, Value> {
        (0..1900)
            .map(|i| (format!("{i:0>pad$}"), json!([0, 0])))
            .collect()
    };
    // the widest names that still fit the file
    let (mut lo, mut hi) = (10usize, 300usize);
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        let ok = s.put(URL, json!({ "layout": wide(mid) })).await.status() == 200;
        if ok { lo = mid } else { hi = mid - 1 }
    }
    // then short entries until one is refused: less than one entry of room is left
    let mut layout = wide(lo);
    layout.insert("db".into(), json!([0, 0]));
    assert_eq!(
        s.put(URL, json!({ "layout": layout.clone() }))
            .await
            .status(),
        200
    );
    for i in 0..100 {
        layout.insert(format!("s{i}"), json!([0, 0]));
        if s.put(URL, json!({ "layout": layout.clone() }))
            .await
            .status()
            != 200
        {
            layout.remove(&format!("s{i}"));
            break;
        }
    }
    s.put(URL, json!({ "layout": layout.clone() })).await;
    let before = view(&s).await;
    let op = json!({ "op": "rename_node", "from": "db", "to": "d".repeat(150) });
    let r = s.put(URL, json!({ "patch": [op] })).await;
    assert_eq!(r.status(), 400);
    let after = view(&s).await;
    assert_eq!(
        (&after["text"], &after["rev"]),
        (&before["text"], &before["rev"])
    );
    assert!(
        after["layout"].get("db").is_some(),
        "the name was not moved"
    );
}
