//! The layout sidecar (`kaanal.layout.json`): node positions travel with the draft, never bump
//! its `rev`, and Save writes them beside the config through the same safe write, refusing a link.

use crate::common::*;
use reqwest::Method;
use serde_json::{Value, json};
use std::path::Path;
use std::time::Duration;

const URL: &str = "/__kaanal/draft";
const SAVE: &str = "/__kaanal/draft/save";
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

/// A fresh folder holding `kaanal.yaml` (BASE) and, when given, a sidecar with `content`.
async fn start_with_sidecar(content: Option<&[u8]>) -> Srv {
    let dir = new_dir();
    if let Some(c) = content {
        std::fs::write(dir.join(SIDECAR), c).unwrap();
    }
    start_at(dir.clone(), &dir, BASE, BASE).await
}

/// A link to a file; false when this machine does not allow it (Windows without the privilege).
fn link_file(target: &Path, link: &Path) -> bool {
    #[cfg(unix)]
    return std::os::unix::fs::symlink(target, link).is_ok();
    #[cfg(windows)]
    return std::os::windows::fs::symlink_file(target, link).is_ok();
}

/// A link to a folder: a symlink on Unix, a junction on Windows (no privilege needed).
fn link_dir(target: &Path, link: &Path) -> bool {
    #[cfg(unix)]
    return std::os::unix::fs::symlink(target, link).is_ok();
    #[cfg(windows)]
    return std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(link)
        .arg(target)
        .output()
        .is_ok_and(|o| o.status.success());
}

fn names(d: &Path) -> Vec<String> {
    let mut v: Vec<_> = std::fs::read_dir(d)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    v.sort();
    v
}

#[tokio::test]
async fn a_layout_round_trips_through_put_and_save() {
    let s = start(BASE).await;
    let layout = json!({ "nginx": [120.0, 80.0], "app": [3.5, -4.0] });
    let r = s.put(URL, json!({ "layout": layout })).await;
    assert_eq!(r.status(), 200);
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["layout"], layout, "{v}");
    assert_eq!(v["layout_note"], Value::Null);
    let v = save(&s).await;
    assert_eq!(v["layout_written"], true, "{v}");
    assert_eq!(
        sidecar(&s.dir),
        json!({ "version": 1, "nodes": layout }),
        "the file is the fixed shape"
    );
    // a new server on the same folder reads it
    let again = start_at(s.dir.clone(), &s.dir, BASE, BASE).await;
    let v = view(&again).await;
    assert_eq!(v["layout"], layout, "{v}");
    assert_eq!(v["layout_note"], Value::Null, "{v}");
}

#[tokio::test]
async fn layout_edits_do_not_bump_rev() {
    let s = start(BASE).await;
    let before = view(&s).await;
    for l in [
        json!({ "app": [1, 2] }),
        json!({ "app": [3, 4] }),
        json!({}),
    ] {
        let v: Value = s
            .put(URL, json!({ "layout": l }))
            .await
            .json()
            .await
            .unwrap();
        assert_eq!(v["rev"], before["rev"], "{v}");
        assert_eq!(
            v["dirty"], false,
            "a position is not a change to the system"
        );
    }
}

#[tokio::test]
async fn put_with_text_and_layout_applies_both() {
    let s = start(BASE).await;
    let text = BASE.replace("seed: 42", "seed: 7");
    let r = s
        .put(URL, json!({ "text": text, "layout": { "db": [5, 6] } }))
        .await;
    assert_eq!(r.status(), 200);
    let v: Value = r.json().await.unwrap();
    assert_eq!((&v["text"], &v["rev"]), (&json!(text), &json!(1)));
    assert_eq!(v["layout"], json!({ "db": [5.0, 6.0] }));
    // and with a patch
    let op = json!({ "op": "set", "path": ["seed"], "value": 8 });
    let v: Value = s
        .put(URL, json!({ "patch": [op], "layout": { "app": [1, 1] } }))
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(v["rev"], 2);
    assert_eq!(
        v["layout"],
        json!({ "app": [1.0, 1.0] }),
        "the whole map is replaced"
    );
}

#[tokio::test]
async fn put_with_nothing_is_still_400() {
    let s = start(BASE).await;
    for body in [
        json!({}),
        json!({ "text": BASE, "patch": [] }),
        json!({ "text": BASE, "patch": [], "layout": {} }),
    ] {
        assert_eq!(s.put(URL, body.clone()).await.status(), 400, "{body}");
    }
    // an unknown key is still refused (the extractor's 422)
    let body = json!({ "layout": { "app": [1, 2] }, "other": 1 });
    assert_eq!(s.put(URL, body).await.status(), 422);
    // a layout that cannot be kept leaves the text alone too
    let r = s
        .put(
            URL,
            json!({ "text": "seed: 1\n", "layout": { "app": [1e7, 0] } }),
        )
        .await;
    assert_eq!(r.status(), 400);
    assert_eq!(view(&s).await["text"], BASE);
}

#[tokio::test]
async fn non_finite_and_out_of_range_positions_are_a_400() {
    let s = start(BASE).await;
    assert_eq!(
        s.put(URL, json!({ "layout": { "app": [1, 2] } }))
            .await
            .status(),
        200
    );
    let many: serde_json::Map<String, Value> = (0..2001)
        .map(|i| (format!("n{i}"), json!([0, 0])))
        .collect();
    for layout in [
        json!({ "app": [1e7, 0] }),
        json!({ "app": [0, -1e7] }),
        json!({ "app": [1_000_000.5, 0] }),
        json!({ "": [0, 0] }),
        Value::Object(many),
    ] {
        let r = s.put(URL, json!({ "layout": layout })).await;
        assert_eq!(r.status(), 400, "{layout}");
    }
    // a body of the wrong shape is the extractor's refusal, like every other route (422; serde_json
    // calls an over-long array a syntax error, so that one is 400)
    for layout in [
        json!({ "app": [0] }),
        json!({ "app": [0, 0, 0] }),
        json!({ "app": ["1", 0] }),
        json!({ "app": null }),
        json!([1, 2]),
    ] {
        let r = s.put(URL, json!({ "layout": layout })).await;
        assert!(matches!(r.status().as_u16(), 400 | 422), "{layout}");
    }
    // infinity cannot be spelled in JSON, but an overflowing number can
    let r = s
        .client
        .put(format!("{}{URL}", s.base))
        .bearer_auth(&s.token)
        .header("content-type", "application/json")
        .body(r#"{ "layout": { "app": [1e999, 0] } }"#)
        .send()
        .await
        .unwrap();
    assert_eq!(r.status(), 400);
    // the edges are allowed, and nothing above changed the layout
    let v = view(&s).await;
    assert_eq!(v["layout"], json!({ "app": [1.0, 2.0] }));
    let edge = json!({ "app": [1e6, -1e6] });
    assert_eq!(s.put(URL, json!({ "layout": edge })).await.status(), 200);
    let exactly: serde_json::Map<String, Value> = (0..2000)
        .map(|i| (format!("n{i}"), json!([0, 0])))
        .collect();
    assert_eq!(s.put(URL, json!({ "layout": exactly })).await.status(), 200);
}

#[tokio::test]
async fn node_names_with_hash_slash_space_and_unicode_round_trip() {
    // a text that does not parse: nothing is filtered by node name, so every key comes back
    let dir = new_dir();
    let text = "nodes: [ not closed\n";
    let s = start_at(dir.clone(), &dir, BASE, text).await;
    let layout = json!({
        "#tag": [1.0, 1.0], "a/b": [2.0, 2.0], "../up": [3.0, 3.0], "a b": [4.0, 4.0],
        "நகரம் 😀": [5.0, 5.0], "back\\slash": [6.0, 6.0], "\u{0}nul": [7.0, 7.0]
    });
    let v: Value = s
        .put(URL, json!({ "layout": layout }))
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(v["layout"], layout, "{v}");
    let v = save(&s).await;
    assert_eq!(v["layout_written"], true, "{v}");
    assert_eq!(sidecar(&dir)["nodes"], layout);
    assert_eq!(
        names(&dir),
        ["kaanal.layout.json", "kaanal.yaml"],
        "a name is never a path"
    );
    let again = start_at(dir.clone(), &dir, BASE, text).await;
    assert_eq!(view(&again).await["layout"], layout);
}

#[tokio::test]
async fn a_rename_moves_the_layout_key() {
    let s = start(BASE).await;
    let l = json!({ "db": [1.0, 2.0], "app": [3.0, 4.0] });
    assert_eq!(s.put(URL, json!({ "layout": l })).await.status(), 200);
    let op = json!({ "op": "rename_node", "from": "db", "to": "store" });
    let v: Value = s
        .put(URL, json!({ "patch": [op] }))
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(
        v["layout"],
        json!({ "store": [1.0, 2.0], "app": [3.0, 4.0] }),
        "{v}"
    );
    save(&s).await;
    assert_eq!(
        sidecar(&s.dir)["nodes"],
        json!({ "store": [1.0, 2.0], "app": [3.0, 4.0] })
    );
}

#[tokio::test]
async fn a_position_for_a_deleted_node_is_dropped_on_save() {
    let s = start(BASE).await;
    let l = json!({ "nginx": [0, 0], "app": [1, 1], "db": [2, 2], "ghost": [3, 3] });
    let v: Value = s
        .put(URL, json!({ "layout": l }))
        .await
        .json()
        .await
        .unwrap();
    assert!(v["layout"].get("ghost").is_none(), "not shown: {v}");
    // remove db (its node and its edge)
    let ops = json!([
        { "op": "remove", "path": ["edges", 1] },
        { "op": "remove", "path": ["nodes", "db"] }
    ]);
    let v: Value = s
        .put(URL, json!({ "patch": ops }))
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(
        v["layout"],
        json!({ "nginx": [0.0, 0.0], "app": [1.0, 1.0] }),
        "{v}"
    );
    save(&s).await;
    assert_eq!(
        sidecar(&s.dir)["nodes"],
        json!({ "nginx": [0.0, 0.0], "app": [1.0, 1.0] })
    );
    // a text that does not parse drops nothing, and is still written
    let l = json!({ "app": [1.0, 1.0], "ghost": [3.0, 3.0] });
    let r = s
        .put(URL, json!({ "text": "nodes: [\n", "layout": l }))
        .await;
    assert_eq!(r.status(), 200);
    let v: Value = r.json().await.unwrap();
    assert_eq!(v["layout"], l);
    assert_eq!(save(&s).await["layout_written"], true);
    assert_eq!(sidecar(&s.dir)["nodes"], l);
}

#[tokio::test]
async fn empty_layout_does_not_delete_an_existing_sidecar() {
    let old = br#"{"version":1,"nodes":{"app":[9,9]}}"#;
    let s = start_with_sidecar(Some(old)).await;
    assert_eq!(view(&s).await["layout"], json!({ "app": [9.0, 9.0] }));
    assert_eq!(s.put(URL, json!({ "layout": {} })).await.status(), 200);
    let v = save(&s).await;
    assert_eq!(v["layout_written"], false, "{v}");
    assert!(v["layout_note"].is_string(), "{v}");
    assert_eq!(std::fs::read(s.dir.join(SIDECAR)).unwrap(), old);
}

#[tokio::test]
async fn a_corrupt_sidecar_loads_empty_with_a_note() {
    for bad in [
        &b"{ not json"[..],
        br#"{"version":2,"nodes":{"app":[1,2]}}"#,
        br#"{"nodes":{"app":[1,2]}}"#,
        br#"{"version":1,"nodes":{"app":[1,2]},"extra":true}"#,
        br#"{"version":1,"nodes":{"app":[1,2,3]}}"#,
        br#"{"version":1,"nodes":{"app":[1e7,2]}}"#,
        br#"{"version":1,"nodes":{"":[1,2]}}"#,
        br#"{"version":1,"nodes":{"app":[1e999,2]}}"#,
        b"\xff\xfe not utf-8",
        b"",
    ] {
        let s = start_with_sidecar(Some(bad)).await;
        let v = view(&s).await;
        assert_eq!(v["layout"], json!({}), "{v}");
        let note = v["layout_note"].as_str().unwrap_or_default();
        assert!(note.contains(SIDECAR), "{note}");
        assert!(
            !note.contains(&s.dir.display().to_string()),
            "no absolute path: {note}"
        );
        // Save writes nothing over it while the layout is empty
        save(&s).await;
        assert_eq!(std::fs::read(s.dir.join(SIDECAR)).unwrap(), bad);
    }
    // a folder in its place is not a sidecar either
    let dir = new_dir();
    std::fs::create_dir(dir.join(SIDECAR)).unwrap();
    let s = start_at(dir.clone(), &dir, BASE, BASE).await;
    let v = view(&s).await;
    assert_eq!(v["layout"], json!({}));
    assert!(v["layout_note"].is_string(), "{v}");
    // and Save refuses to replace it
    assert_eq!(
        s.put(URL, json!({ "layout": { "app": [1, 1] } }))
            .await
            .status(),
        200
    );
    let v = save(&s).await;
    assert_eq!(v["layout_written"], false, "{v}");
    assert!(dir.join(SIDECAR).is_dir());
}

#[tokio::test]
async fn an_oversized_sidecar_is_ignored() {
    // valid JSON, padded past 256 KiB
    let mut big = br#"{"version":1,"nodes":{"app":[1,2]}}"#.to_vec();
    big.resize(256 * 1024 + 1, b' ');
    let s = start_with_sidecar(Some(&big)).await;
    let v = view(&s).await;
    assert_eq!(v["layout"], json!({}), "{v}");
    assert!(
        v["layout_note"].as_str().unwrap().contains("256 KiB"),
        "{v}"
    );
    // exactly at the limit is read
    big.truncate(256 * 1024);
    let s = start_with_sidecar(Some(&big)).await;
    assert_eq!(view(&s).await["layout"], json!({ "app": [1.0, 2.0] }));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_huge_or_deeply_nested_sidecar_does_not_hang_or_crash() {
    let deep = "[".repeat(200_000);
    let entries: Vec<String> = (0..10_000).map(|i| format!(r#""n{i}":[0,0]"#)).collect();
    let mut many = format!(r#"{{"version":1,"nodes":{{{}"#, entries.join(","));
    many.push_str("}}");
    let deep_obj = format!(r#"{{"version":1,"nodes":{}"#, "{\"a\":".repeat(50_000));
    for bad in [deep.into_bytes(), many.into_bytes(), deep_obj.into_bytes()] {
        // spawned, so a load that blocked its worker still lets this thread's timer fire
        let task = tokio::spawn(async move {
            let s = start_with_sidecar(Some(&bad)).await;
            view(&s).await
        });
        let r = tokio::time::timeout(Duration::from_secs(20), task)
            .await
            .expect("loading a hostile sidecar did not finish in time")
            .unwrap();
        assert_eq!(r["layout"], json!({}), "{r}");
        assert!(r["layout_note"].is_string(), "{r}");
    }
}

#[tokio::test]
async fn a_symlinked_sidecar_is_refused_and_the_config_is_still_saved() {
    let root = new_dir();
    let dir = root.join("proj");
    std::fs::create_dir(&dir).unwrap();
    let victim = root.join("victim.json");
    std::fs::write(&victim, r#"{"version":1,"nodes":{"app":[1,2]}}"#).unwrap();
    if !link_file(&victim, &dir.join(SIDECAR)) {
        eprintln!("skipped: cannot create file symlinks here");
        let _ = std::fs::remove_dir_all(&root);
        return;
    }
    let s = start_at(root.clone(), &dir, BASE, BASE).await;
    // on load: not followed
    let v = view(&s).await;
    assert_eq!(v["layout"], json!({}), "{v}");
    assert!(v["layout_note"].is_string(), "{v}");
    // on Save: refused, the config still saved
    let text = BASE.replace("seed: 42", "seed: 9");
    let r = s
        .put(URL, json!({ "text": text, "layout": { "app": [5, 5] } }))
        .await;
    assert_eq!(r.status(), 200);
    let v = save(&s).await;
    assert_eq!(v["ok"], true);
    assert_eq!(v["layout_written"], false, "{v}");
    let note = v["layout_note"].as_str().unwrap();
    assert!(note.contains(SIDECAR), "{note}");
    assert_eq!(
        std::fs::read_to_string(dir.join("kaanal.yaml")).unwrap(),
        text
    );
    assert_eq!(
        std::fs::read_to_string(&victim).unwrap(),
        r#"{"version":1,"nodes":{"app":[1,2]}}"#,
        "the link's target is untouched"
    );
    assert!(
        std::fs::symlink_metadata(dir.join(SIDECAR))
            .unwrap()
            .file_type()
            .is_symlink(),
        "the link is not replaced"
    );
}

#[tokio::test]
async fn a_sidecar_outside_the_project_is_never_written() {
    // 1. the sidecar's name is a link to a folder outside the project
    let root = new_dir();
    let (dir, outside) = (root.join("proj"), root.join("outside"));
    std::fs::create_dir(&dir).unwrap();
    std::fs::create_dir(&outside).unwrap();
    assert!(link_dir(&outside, &dir.join(SIDECAR)), "a folder link");
    let s = start_at(root.clone(), &dir, BASE, BASE).await;
    assert!(view(&s).await["layout_note"].is_string());
    assert_eq!(
        s.put(URL, json!({ "layout": { "app": [1, 1] } }))
            .await
            .status(),
        200
    );
    let v = save(&s).await;
    assert_eq!(v["layout_written"], false, "{v}");
    assert!(names(&outside).is_empty(), "nothing went through the link");
    assert_eq!(
        names(&dir),
        ["kaanal.layout.json", "kaanal.yaml"],
        "no temp left"
    );
    drop(s);

    // 2. the config is reached through a linked folder: the sidecar goes beside the config (in the
    // folder the link names), and nowhere else
    let root = new_dir();
    let (real, link) = (root.join("real"), root.join("link"));
    std::fs::create_dir(&real).unwrap();
    assert!(link_dir(&real, &link), "a folder link");
    let s = start_at(root.clone(), &link, BASE, BASE).await;
    assert_eq!(
        s.put(URL, json!({ "layout": { "app": [1, 1] } }))
            .await
            .status(),
        200
    );
    assert_eq!(save(&s).await["layout_written"], true);
    assert_eq!(names(&real), ["kaanal.layout.json", "kaanal.yaml"]);
    assert_eq!(names(&root), ["link", "real"], "nothing beside the project");
}

/// A valid config whose node names are awkward as keys, paths and query values.
const AWKWARD: &str = r##"version: 1
seed: 1
nodes:
  "a/b": { type: gateway, proc: 1ms }
  "#x": { type: service, proc: 1ms }
  "a b": { type: service, proc: 1ms }
  "நகரம் 😀": { type: service, proc: 1ms }
  "1": { type: database, flavor: postgres }
edges:
  - { from: "a/b", to: "#x", net: 1ms }
  - { from: "#x", to: "a b", net: 1ms }
  - { from: "#x", to: "நகரம் 😀", net: 1ms }
  - { from: "#x", to: "1", net: 1ms }
endpoints:
  "GET /x":
    override: { body: { ok: true } }
    chain: [{ call: "#x", chain: [{ call: "a b" }, { call: "நகரம் 😀" }, { call: "1", op: read }] }]
traffic:
  t: { rps: 50, duration: 5s, journeys: [{ endpoint: "GET /x" }] }
"##;

#[tokio::test]
async fn awkward_node_names_are_keys_in_doc_layout_save_and_the_log_filter() {
    let dir = new_dir();
    let s = start_at(dir.clone(), &dir, BASE, AWKWARD).await;
    let names = ["a/b", "#x", "a b", "நகரம் 😀", "1"];
    let v = view(&s).await;
    assert_eq!(v["doc_error"], Value::Null, "{v}");
    for n in names {
        assert!(v["doc"]["nodes"][n].is_object(), "{n}: {v}");
    }
    assert_eq!(v["doc"]["nodes"].as_object().unwrap().len(), names.len());
    // a position for each, plus one for a name the text lacks
    let mut layout: serde_json::Map<String, Value> = names
        .iter()
        .zip([0.5, 1.5, 2.5, 3.5, 4.5])
        .map(|(n, x)| ((*n).to_string(), json!([x, 1.0])))
        .collect();
    layout.insert("ghost".into(), json!([9.0, 9.0]));
    let v: Value = s
        .put(URL, json!({ "layout": layout }))
        .await
        .json()
        .await
        .unwrap();
    for n in names {
        assert_eq!(v["layout"][n], layout[n], "{n} survives the view: {v}");
    }
    assert!(v["layout"].get("ghost").is_none(), "{v}");
    let v = save(&s).await;
    assert_eq!(v["layout_written"], true, "{v}");
    let on_disk = sidecar(&dir);
    for n in names {
        assert_eq!(on_disk["nodes"][n], layout[n], "{n} saved");
    }
    assert!(on_disk["nodes"].get("ghost").is_none(), "{on_disk}");
    // the log's `node=` filter takes such a name, URL-encoded
    let r = s
        .send(
            Method::POST,
            "/__kaanal/sim/run",
            Some(json!({ "source": "draft", "profile": "t" })),
        )
        .await;
    assert_eq!(r.status(), 202);
    let id = r.json::<Value>().await.unwrap()["id"]
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
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    for n in ["#x", "a b", "நகரம் 😀", "1"] {
        let mut q = reqwest::Url::parse("http://x/").unwrap();
        q.query_pairs_mut()
            .append_pair("node", n)
            .append_pair("limit", "50");
        let r = s
            .get(&format!(
                "/__kaanal/sim/runs/{id}/log?{}",
                q.query().unwrap()
            ))
            .await;
        assert_eq!(r.status(), 200, "{n}");
        let v: Value = r.json().await.unwrap();
        let lines = v["lines"].as_array().unwrap();
        assert!(!lines.is_empty(), "{n}: {v}");
        assert!(lines.iter().all(|l| l["node"] == n), "{n}: {v}");
    }
}

/// The loader reads `1:` and `true:` as node names (paths.rs relies on it); the canvas must too.
#[tokio::test]
async fn non_string_yaml_node_keys_are_names_in_doc_and_layout() {
    let text = "version: 1\nnodes:\n  1: { type: gateway }\n  true: { type: service }\n  2.5: { type: service }\nedges:\n  - { from: 1, to: true }\n";
    let dir = new_dir();
    let s = start_at(dir.clone(), &dir, BASE, text).await;
    let v: Value = s
        .send(
            Method::POST,
            "/__kaanal/draft/validate",
            Some(json!({ "text": text })),
        )
        .await
        .json()
        .await
        .unwrap();
    let v2 = view(&s).await;
    assert_eq!(v["ok"], true, "{v}");
    assert_eq!(v2["doc_error"], Value::Null, "{v2}");
    assert!(v2["doc"]["nodes"]["1"].is_object(), "{v2}");
    assert!(v2["doc"]["nodes"]["true"].is_object(), "{v2}");
    // known limit: an edge end written bare stays a number or a bool in `doc` (a Studio client compares `String(from)`)
    assert_eq!(
        v2["doc"]["edges"][0],
        json!({ "from": 1, "to": true }),
        "{v2}"
    );
}

#[tokio::test]
async fn save_does_not_read_or_replace_an_oversized_existing_sidecar() {
    // an existing 300 KiB file: Save's undo copy must not read it, and it is not replaced
    let big = vec![b' '; 300 * 1024];
    let s = start_with_sidecar(Some(&big)).await;
    let op = json!({ "op": "set", "path": ["nodes", "app", "proc"], "value": "9ms" });
    assert_eq!(s.put(URL, json!({ "patch": [op] })).await.status(), 200);
    assert_eq!(
        s.put(URL, json!({ "layout": { "app": [1.0, 2.0] } }))
            .await
            .status(),
        200
    );
    let v = save(&s).await;
    assert_eq!(v["layout_written"], false, "{v}");
    assert!(
        v["layout_note"].as_str().unwrap().contains("256 KiB"),
        "{v}"
    );
    assert!(
        std::fs::read_to_string(s.dir.join("kaanal.yaml"))
            .unwrap()
            .contains("9ms"),
        "the config is still saved"
    );
    assert_eq!(std::fs::read(s.dir.join(SIDECAR)).unwrap(), big);
}

#[tokio::test]
async fn layout_dirty_tracks_the_sidecar() {
    let s = start(BASE).await;
    assert_eq!(view(&s).await["layout_dirty"], false);
    let l = json!({ "app": [1, 2] });
    let v: Value = s
        .put(URL, json!({ "layout": l }))
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(v["layout_dirty"], true, "{v}");
    // dragging back to what the sidecar holds is clean again
    let v: Value = s
        .put(URL, json!({ "layout": {} }))
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(v["layout_dirty"], false, "{v}");
    s.put(URL, json!({ "layout": l })).await;
    let v = save(&s).await;
    assert_eq!(v["layout_written"], true, "{v}");
    assert_eq!(view(&s).await["layout_dirty"], false);
}

#[tokio::test]
async fn a_load_note_never_quotes_the_file() {
    let s = start_with_sidecar(Some(br#"{"version":1,"nodes":{"app":"HUNTER2-SECRET"}}"#)).await;
    let v = view(&s).await;
    let note = v["layout_note"].as_str().unwrap_or_default();
    assert!(note.contains(SIDECAR), "{note}");
    assert!(
        !note.contains("HUNTER2"),
        "the note quoted the file: {note}"
    );
}

#[tokio::test]
async fn a_blank_start_reads_the_sidecar_beside_it() {
    let dir = new_dir();
    std::fs::write(dir.join(SIDECAR), br#"{"version":1,"nodes":{"app":[7,8]}}"#).unwrap();
    let s = start_blank_in(dir).await;
    let v = view(&s).await;
    assert_eq!(v["layout_note"], Value::Null, "{v}");
    // the blank text defines no node `app`, so its position is not shown
    assert_eq!(v["layout"], json!({}), "{v}");
    assert_eq!(v["layout_dirty"], false, "{v}");
    // once the text defines `app`, the position read from the sidecar shows
    let v: Value = s
        .put(URL, json!({ "text": BASE }))
        .await
        .json()
        .await
        .unwrap();
    assert_eq!(v["layout"], json!({ "app": [7.0, 8.0] }), "{v}");
}
