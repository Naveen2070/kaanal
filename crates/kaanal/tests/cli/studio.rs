//! `kaanal up --blank` and `kaanal studio`: an instance with no system until the first apply.

use crate::common::*;
use std::path::Path;

/// A valid system that serves the fixture spec's routes.
const SYSTEM: &str = "version: 1\nnodes:\n  gw: { type: gateway }\n  app: { type: service, spec: openapi.yaml }\nedges:\n  - { from: gw, to: app }\n";

#[test]
fn a_blank_up_has_no_system_until_the_first_apply_and_save_writes_the_file() {
    let dir = project("blank-up", "");
    // the config named with -c is where Save writes; it is not read, and need not exist
    let up = Up::launch(dir.clone(), &["--blank"], Path::new("new.yaml"));
    assert_eq!(ok(&up, &["draft", "show"]), kaanal_server::BLANK_TEXT);
    assert_eq!(up.get_status("/users"), 404, "nothing is served yet");
    // an empty draft cannot be applied
    let err = fails(&up, &["draft", "apply"]);
    assert!(err.starts_with("draft:"), "{err}");

    let file = dir.join("next.yaml");
    std::fs::write(&file, SYSTEM).unwrap();
    ok(&up, &["draft", "set", "-f", file.to_str().unwrap()]);
    let out = ok(&up, &["draft", "apply"]);
    assert!(out.contains("applied generation 1"), "{out}");
    assert_ne!(up.get_status("/users"), 404, "the spec's routes are served");

    assert!(
        !dir.join("new.yaml").exists(),
        "nothing is on disk before save"
    );
    let out = ok(&up, &["draft", "save"]);
    assert_eq!(out.trim(), "saved: new.yaml");
    assert_eq!(
        std::fs::read_to_string(dir.join("new.yaml")).unwrap(),
        SYSTEM
    );
}

#[test]
fn studio_starts_blank_and_does_not_need_a_browser_with_no_open() {
    let dir = project("studio", "");
    let up = Up::launch_args(
        dir,
        &["studio", "--no-open", "--port", "0"],
        Path::new("kaanal.yaml"),
    );
    assert_eq!(ok(&up, &["draft", "show"]), kaanal_server::BLANK_TEXT);
    assert_eq!(up.get_status("/users"), 404);
}

#[test]
fn blank_does_not_combine_with_a_background_start() {
    let out = bin().args(["up", "--blank", "-d"]).output().unwrap();
    assert_eq!(out.status.code(), Some(64), "{}", stderr(&out));
}

#[test]
fn studio_prints_the_studio_route_and_up_keeps_the_status_page() {
    use std::io::{BufRead, BufReader};
    use std::process::Stdio;
    let ui_line = |args: &[&str]| {
        let dir = project("studio-url", "");
        let mut child = bin()
            .args(args)
            .args(["--port", "0", "-c", "kaanal.yaml"])
            .current_dir(&dir)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let line = BufReader::new(child.stdout.take().unwrap())
            .lines()
            .map(Result::unwrap)
            .find(|l| l.starts_with("UI:"));
        stop(child);
        line.expect("a UI: line")
    };
    assert!(ui_line(&["studio", "--no-open"]).contains("/__kaanal/studio#token="));
    assert!(ui_line(&["up", "--blank"]).contains("/__kaanal/ui#token="));
}
