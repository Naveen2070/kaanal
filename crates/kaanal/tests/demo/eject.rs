//! `kaanal demo --eject`: what it writes, refuses and leaves alone.

use crate::common::*;
use crate::demo;

use std::path::Path;

/// The relative paths of every file under `dir`, sorted.
pub(crate) fn files_under(dir: &Path) -> Vec<String> {
    pub(crate) fn walk(base: &Path, at: &Path, out: &mut Vec<String>) {
        for e in std::fs::read_dir(at).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                walk(base, &p, out);
            } else {
                let rel = p.strip_prefix(base).unwrap();
                out.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    let mut v = Vec::new();
    walk(dir, dir, &mut v);
    v.sort();
    v
}

pub(crate) fn template_paths() -> Vec<String> {
    let mut v: Vec<String> = demo::FILES.iter().map(|(p, _)| (*p).to_string()).collect();
    v.sort();
    v
}

/// What `--eject` leaves: the template plus the `.gitignore` entry for `.kaanal/`.
pub(crate) fn ejected_paths() -> Vec<String> {
    let mut v = template_paths();
    v.push(".gitignore".to_string());
    v.sort();
    v
}

#[test]
fn eject_writes_the_project_and_starts_no_server() {
    let work = empty("eject-basic");
    let _clean = Cleanup(work.clone());
    let o = run(&work, &["demo", "--eject"]);
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    let dir = work.join("kaanal-demo");
    assert_eq!(files_under(&dir), ejected_paths());
    let ignore = std::fs::read_to_string(dir.join(".gitignore")).unwrap();
    assert!(ignore.lines().any(|l| l == ".kaanal/"), "{ignore}");
    for (path, text) in demo::FILES {
        let got = std::fs::read_to_string(dir.join(path)).unwrap();
        assert_eq!(got, *text, "{path}");
    }
    // It returned (a server would still be running) and left no state beside the project.
    assert_eq!(entries(&work), ["kaanal-demo"]);
    assert_eq!(run(&dir, &["validate"]).status.code(), Some(0));
}

#[test]
fn eject_refuses_a_non_empty_directory_without_force() {
    let work = empty("eject-busy");
    let _clean = Cleanup(work.clone());
    let dir = work.join("mine");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("notes.txt"), "keep").unwrap();
    let o = run(&work, &["demo", "--eject", "mine"]);
    let err = String::from_utf8_lossy(&o.stderr);
    assert_eq!(o.status.code(), Some(64), "{err}");
    assert!(
        err.contains("not empty") && err.contains("--force"),
        "{err}"
    );
    assert_eq!(entries(&dir), ["notes.txt"]);
}

#[test]
fn eject_into_an_empty_existing_directory_works() {
    let work = empty("eject-emptydir");
    let _clean = Cleanup(work.clone());
    std::fs::create_dir_all(work.join("out")).unwrap();
    let o = run(&work, &["demo", "--eject", "out"]);
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    assert_eq!(files_under(&work.join("out")), ejected_paths());
}

#[test]
fn ejected_project_serves_the_same_storefront_under_plain_up() {
    let work = empty("eject-up");
    let _clean = Cleanup(work.clone());
    let o = run(&work, &["demo", "--eject", "shop"]);
    assert_eq!(o.status.code(), Some(0));
    let dir = work.join("shop");
    let (_up, port) = up(&dir);
    let base = format!("http://127.0.0.1:{port}");
    let http = agent();
    let page = http
        .get(&format!("{base}/"))
        .call()
        .unwrap()
        .body_mut()
        .read_to_string()
        .unwrap();
    assert_eq!(page, site_file("index.html"));
    let products = get_json(&http, &format!("{base}/products"));
    assert_eq!(products.as_array().unwrap().len(), 24);
    let cart = format!("{base}/cart");
    let mut added = http
        .post(&cart)
        .send_json(serde_json::json!({"productId": 1, "qty": 2}))
        .unwrap();
    assert_eq!(added.status().as_u16(), 201);
    let item: serde_json::Value = added.body_mut().read_json().unwrap();
    assert_eq!(get_json(&http, &cart).as_array().unwrap().len(), 1);
    let id = item["id"].as_u64().unwrap();
    let removed = http.delete(&format!("{cart}/{id}")).call().unwrap();
    assert_eq!(removed.status().as_u16(), 204);
    let mut list = http
        .get(&format!("{base}/__kaanal/scenarios"))
        .header("authorization", &format!("Bearer {}", run_token(&dir)))
        .call()
        .unwrap();
    let list: serde_json::Value = list.body_mut().read_json().unwrap();
    let names: Vec<&str> = list
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, STORIES);
}

#[test]
fn eject_writes_the_chosen_locale() {
    let work = empty("eject-locale");
    let _clean = Cleanup(work.clone());
    let o = run(&work, &["demo", "--eject", "de", "--locale", "de-DE"]);
    assert_eq!(o.status.code(), Some(0));
    let cfg = std::fs::read_to_string(work.join("de/kaanal.yaml")).unwrap();
    assert!(cfg.lines().any(|l| l == "locale: de-DE"), "{cfg}");
    assert!(!cfg.contains("en-IN"), "{cfg}");
    // Without --locale the template's en-IN stays.
    run(&work, &["demo", "--eject", "en"]);
    let cfg = std::fs::read_to_string(work.join("en/kaanal.yaml")).unwrap();
    assert!(cfg.lines().any(|l| l == "locale: en-IN"), "{cfg}");
    // An unknown tag is a usage error and creates nothing.
    let o = run(&work, &["demo", "--eject", "bad", "--locale", "xx-XX"]);
    assert_eq!(o.status.code(), Some(64));
    assert!(!work.join("bad").exists());
}

#[test]
fn eject_prints_the_next_command() {
    let work = empty("eject-next");
    let _clean = Cleanup(work.clone());
    let o = run(&work, &["demo", "--eject"]);
    let out = String::from_utf8_lossy(&o.stdout);
    let root = std::path::absolute(work.join("kaanal-demo")).unwrap();
    assert!(out.contains(&root.display().to_string()), "{out}");
    assert!(out.contains("cd kaanal-demo && kaanal up"), "{out}");
    // A directory with spaces is quoted.
    let o = run(&work, &["demo", "--eject", "my shop"]);
    let out = String::from_utf8_lossy(&o.stdout);
    assert!(out.contains("cd \"my shop\" && kaanal up"), "{out}");
    // --json: one object with the root, the files and the command.
    let o = run(&work, &["--json", "demo", "--eject", "j"]);
    let j: serde_json::Value = serde_json::from_slice(&o.stdout).unwrap();
    assert_eq!(
        j["root"],
        std::path::absolute(work.join("j"))
            .unwrap()
            .display()
            .to_string()
    );
    assert_eq!(j["next"], "cd j && kaanal up");
    let mut written: Vec<String> = j["written"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();
    written.sort();
    assert_eq!(written, ejected_paths());
}

#[test]
fn eject_while_a_demo_runs_works() {
    let d = demo_in(&empty("eject-running"), &["--port", "0"]);
    let o = run(&d.dir, &["demo", "--eject", "copy"]);
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    assert_eq!(files_under(&d.dir.join("copy")), ejected_paths());
    assert_eq!(
        status_of(&format!("http://127.0.0.1:{}/products", d.port)),
        200
    );
}

#[test]
fn force_without_eject_is_usage_error_64() {
    let work = empty("eject-force-alone");
    let _clean = Cleanup(work.clone());
    let o = run(&work, &["demo", "--force"]);
    assert_eq!(o.status.code(), Some(64));
    // --port and --open make no sense when nothing runs.
    for flag in [&["--port", "0"][..], &["--open"]] {
        let mut args = vec!["demo", "--eject"];
        args.extend_from_slice(flag);
        let o = run(&work, &args);
        assert_eq!(o.status.code(), Some(64), "{flag:?}");
    }
    assert_eq!(std::fs::read_dir(&work).unwrap().count(), 0);
}

#[test]
fn eject_does_not_create_dot_kaanal() {
    let work = empty("eject-nostate");
    let _clean = Cleanup(work.clone());
    run(&work, &["demo", "--eject", "x"]);
    assert_eq!(entries(&work), ["x"]);
    assert!(!work.join("x/.kaanal").exists());
}

#[test]
fn eject_force_keeps_foreign_files_and_overwrites_template_files() {
    let work = empty("eject-force");
    let _clean = Cleanup(work.clone());
    let dir = work.join("p");
    std::fs::create_dir_all(dir.join("site")).unwrap();
    std::fs::write(dir.join("mine.txt"), "mine").unwrap();
    std::fs::write(dir.join("site/extra.js"), "extra").unwrap();
    std::fs::write(dir.join("kaanal.yaml"), "edited").unwrap();
    let o = run(&work, &["demo", "--eject", "p", "--force"]);
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("mine.txt")).unwrap(),
        "mine"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("site/extra.js")).unwrap(),
        "extra"
    );
    let cfg = std::fs::read_to_string(dir.join("kaanal.yaml")).unwrap();
    assert!(cfg.contains("locale: en-IN"), "{cfg}");
    let mut want = ejected_paths();
    want.extend(["mine.txt".to_string(), "site/extra.js".to_string()]);
    want.sort();
    assert_eq!(files_under(&dir), want);
}

#[test]
fn eject_through_a_linked_dir_is_refused() {
    let work = empty("eject-link");
    let _clean = Cleanup(work.clone());
    let outside = empty("eject-link-outside");
    let _clean2 = Cleanup(outside.clone());
    if !link_dir(&work.join("l"), &outside) {
        return; // links cannot be made here
    }
    for force in [&[][..], &["--force"]] {
        let mut args = vec!["demo", "--eject", "l"];
        args.extend_from_slice(force);
        let o = run(&work, &args);
        let err = String::from_utf8_lossy(&o.stderr);
        assert_eq!(o.status.code(), Some(1), "{err}");
        assert!(err.contains("symbolic link"), "{err}");
        assert!(entries(&outside).is_empty(), "wrote through the link");
    }
}
