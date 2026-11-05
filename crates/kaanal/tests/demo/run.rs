//! `kaanal demo` itself: startup, the banner, the folder it leaves, locale and port.

use crate::common::*;

impl Drop for Demo {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
        let _ = std::fs::remove_file(&self.err);
        let _ = std::fs::remove_dir_all(demo_config(&self.dir));
    }
}

impl Demo {
    pub(crate) fn stderr(&self) -> String {
        std::fs::read_to_string(&self.err).unwrap_or_default()
    }

    /// `kaanal down` from the same folder; the demo then exits 0.
    pub(crate) fn down(&mut self) {
        let o = run(&self.dir, &["down"]);
        assert_eq!(
            o.status.code(),
            Some(0),
            "{}",
            String::from_utf8_lossy(&o.stderr)
        );
        let status = self.child.wait().unwrap();
        assert_eq!(status.code(), Some(0), "{}", self.stderr());
    }
}

#[test]
fn demo_serves_the_storefront_and_leaves_only_dot_kaanal() {
    let mut d = demo_in(&empty("serves"), &["--port", "0"]);
    let token = run_token(&d.dir);
    // One URL with the token in the fragment, the seed and locale, how to stop, one story.
    let out = d.out.clone();
    assert!(
        out.contains(&format!(
            "Demo:  http://localhost:{}/#token={token}",
            d.port
        )),
        "{out}"
    );
    assert!(out.contains("Seed:  7") && out.contains("en-IN"), "{out}");
    assert!(
        out.contains("Ctrl-C") && out.contains("kaanal down"),
        "{out}"
    );
    assert!(out.contains("kaanal scenario activate slow-db"), "{out}");
    let base = format!("http://127.0.0.1:{}", d.port);
    let page = agent()
        .get(&format!("{base}/"))
        .call()
        .unwrap()
        .body_mut()
        .read_to_string()
        .unwrap();
    assert_eq!(page, site_file("index.html"));
    assert_eq!(status_of(&format!("{base}/products")), 200);
    d.down();
    assert_eq!(entries(&d.dir), [".kaanal"]);
    assert!(d.dir.join(".kaanal/demo/kaanal.yaml").exists());
}

#[test]
fn run_json_is_removed_after_stop() {
    let mut d = demo_with(&empty("json"), &["--json"], &["--port", "0"]);
    let token = run_token(&d.dir);
    let line = d.out.lines().last().unwrap().to_string();
    let j: serde_json::Value = serde_json::from_str(&line).unwrap();
    assert!(
        !line.contains(&token),
        "the object never holds the token: {line}"
    );
    assert_eq!(j["locale"], "en-IN", "{j}");
    assert_eq!(j["url"], format!("http://localhost:{}/", d.port), "{j}");
    assert_eq!(j["seed"], 7, "{j}");
    assert!(d.dir.join(".kaanal/run.json").exists());
    d.down();
    assert!(!d.dir.join(".kaanal/run.json").exists());
}

#[test]
fn a_second_demo_exits_64_and_leaves_the_files_alone() {
    let d = demo_in(&empty("twice"), &["--port", "0"]);
    let page = d.dir.join(".kaanal/demo/site/index.html");
    std::fs::write(&page, "edited").unwrap();
    let o = run(&d.dir, &["demo", "--port", "0", "--locale", "de-DE"]);
    let err = String::from_utf8_lossy(&o.stderr);
    assert_eq!(o.status.code(), Some(64), "{err}");
    assert!(err.contains("already running"), "{err}");
    assert_eq!(std::fs::read_to_string(&page).unwrap(), "edited");
    let cfg = std::fs::read_to_string(d.dir.join(".kaanal/demo/kaanal.yaml")).unwrap();
    assert!(cfg.contains("locale: en-IN"), "{cfg}");
    assert_eq!(
        status_of(&format!("http://127.0.0.1:{}/products", d.port)),
        200
    );
}

#[test]
fn demo_next_to_a_kaanal_yaml_notes_it_and_works() {
    let dir = empty("beside");
    let mine = "not: [a, valid, project\n";
    std::fs::write(dir.join("kaanal.yaml"), mine).unwrap();
    let d = demo_in(&dir, &["--port", "0"]);
    assert_eq!(
        status_of(&format!("http://127.0.0.1:{}/products", d.port)),
        200
    );
    assert!(d.stderr().contains("kaanal.yaml"), "{}", d.stderr());
    assert_eq!(
        std::fs::read_to_string(dir.join("kaanal.yaml")).unwrap(),
        mine
    );
}

#[test]
fn kaanal_chaos_from_the_same_folder_changes_the_demo() {
    let d = demo_in(&empty("chaos"), &["--port", "0"]);
    let products = format!("http://127.0.0.1:{}/products", d.port);
    assert_eq!(status_of(&products), 200);
    let o = run(&d.dir, &["chaos", "set", "nginx", "--down"]);
    assert_eq!(
        o.status.code(),
        Some(0),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    assert_eq!(status_of(&products), 503);
}

#[test]
fn locale_flag_is_written_into_the_demo_config() {
    // The products are fixed examples, so the locale shows in the config and the banner.
    let d = demo_in(&empty("locale"), &["--port", "0", "--locale", "de-DE"]);
    let cfg = std::fs::read_to_string(d.dir.join(".kaanal/demo/kaanal.yaml")).unwrap();
    assert!(cfg.lines().any(|l| l == "locale: de-DE"), "{cfg}");
    assert!(!cfg.contains("en-IN"), "{cfg}");
    assert!(d.out.contains("de-DE"), "{}", d.out);
}

#[test]
fn an_unknown_locale_is_a_usage_error_64() {
    let dir = empty("badlocale");
    let _clean = Cleanup(dir.clone());
    for bad in ["xx-XX", "../x", "en-us"] {
        let o = run(&dir, &["demo", "--port", "0", "--locale", bad]);
        let err = String::from_utf8_lossy(&o.stderr);
        assert_eq!(o.status.code(), Some(64), "{bad}: {err}");
        assert!(err.contains("en-US"), "lists the packs: {err}");
        assert!(entries(&dir).is_empty(), "{bad}: wrote {:?}", entries(&dir));
    }
}

#[test]
fn a_busy_default_port_falls_back_and_prints_the_real_url() {
    // Whoever holds 8080 (this test, or something else on the machine), the demo moves on.
    let _hold = std::net::TcpListener::bind("127.0.0.1:8080");
    let d = demo_in(&empty("busy"), &[]);
    assert_ne!(d.port, 8080, "{}", d.out);
    assert_eq!(status_of(&format!("http://127.0.0.1:{}/", d.port)), 200);
}

#[test]
fn demo_works_in_a_folder_with_spaces_and_non_ascii() {
    let mut d = demo_in(&empty("sp\u{e4}ce \u{65e5}\u{672c}"), &["--port", "0"]);
    let base = format!("http://127.0.0.1:{}", d.port);
    assert_eq!(status_of(&format!("{base}/")), 200);
    assert_eq!(status_of(&format!("{base}/cart")), 200);
    d.down();
    assert_eq!(entries(&d.dir), [".kaanal"]);
}

#[test]
fn demo_has_no_host_or_detach_flag() {
    let dir = empty("flags");
    let _clean = Cleanup(dir.clone());
    for flags in [
        &["--host", "0.0.0.0"][..],
        &["-d"],
        &["--detach"],
        &["--detached"],
        &["--wait", "1s"],
    ] {
        let o = bin()
            .arg("demo")
            .args(flags)
            .current_dir(&dir)
            .output()
            .unwrap();
        assert_eq!(o.status.code(), Some(64), "{flags:?}");
        assert!(entries(&dir).is_empty(), "{flags:?}");
    }
    let help = run(&dir, &["demo", "--help"]);
    let help = String::from_utf8_lossy(&help.stdout);
    assert!(
        help.contains("--port") && help.contains("--locale") && help.contains("--open"),
        "{help}"
    );
    assert!(!help.contains("--host"), "{help}");
}

#[test]
fn demo_refuses_to_write_through_a_linked_folder() {
    for (name, linked) in [("linkdemo", "demo"), ("linkstate", "")] {
        let dir = empty(name);
        let outside = empty(&format!("{name}-outside"));
        std::fs::write(outside.join("keep.txt"), "mine").unwrap();
        let ok = if linked.is_empty() {
            link_dir(&dir.join(".kaanal"), &outside)
        } else {
            std::fs::create_dir_all(dir.join(".kaanal")).unwrap();
            link_dir(&dir.join(".kaanal").join(linked), &outside)
        };
        if !ok {
            continue; // links cannot be made here
        }
        let o = run(&dir, &["demo", "--port", "0"]);
        let err = String::from_utf8_lossy(&o.stderr);
        assert_eq!(o.status.code(), Some(1), "{name}: {err}");
        assert!(err.contains("symbolic link"), "{name}: {err}");
        let mut left: Vec<_> = std::fs::read_dir(&outside)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        left.sort();
        assert_eq!(left, ["keep.txt"], "{name}: wrote through the link");
    }
}

// ---- `kaanal demo --eject` ----
