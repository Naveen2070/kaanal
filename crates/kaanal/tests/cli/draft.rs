//! `kaanal draft`: show, set, validate and apply the running instance's draft.

use crate::common::*;

/// The project's own `kaanal.yaml` text with its inline `scenarios:` section.
fn base(dir: &std::path::Path) -> String {
    std::fs::read_to_string(dir.join("kaanal.yaml")).unwrap()
}

#[test]
fn draft_show_prints_the_config_text_the_instance_started_from() {
    let (up, dir) = scenario_up("draft-show");
    let out = ok(&up, &["draft", "show"]);
    assert_eq!(out, base(&dir));
    let v = one_json(&ok(&up, &["draft", "show", "--json"]));
    assert_eq!(v["text"], base(&dir));
    assert_eq!(v["dirty"], false);
}

#[test]
fn draft_validate_prints_positions_and_exits_1() {
    let (up, dir) = scenario_up("draft-validate");
    // the unchanged draft is fine
    assert!(ok(&up, &["draft", "validate"]).starts_with("ok"));
    let bad = base(&dir).replace("{ from: app, to: db }", "{ from: app, to: nope }");
    let file = dir.join("bad.yaml");
    std::fs::write(&file, &bad).unwrap();
    let path = file.to_str().unwrap();
    let err = fails(&up, &["draft", "validate", "--file", path]);
    let line = bad.lines().position(|l| l.contains("to: nope")).unwrap() + 1;
    assert!(err.contains(&format!("bad.yaml:{line}:")), "{err}");
    assert!(err.contains("nope"), "{err}");
    // the file was only checked: the stored draft is unchanged
    assert_eq!(ok(&up, &["draft", "show"]), base(&dir));
    // set it, and the stored draft is what validate checks
    ok(&up, &["draft", "set", "-f", path]);
    let err = fails(&up, &["draft", "validate"]);
    assert!(err.starts_with(&format!("draft:{line}:")), "{err}");
}

#[test]
fn draft_apply_prints_kept_dropped_and_flagged() {
    let (up, dir) = scenario_up("draft-apply");
    ok(&up, &["scenario", "activate", "db-down"]);
    ok(&up, &["chaos", "set", "app", "--latency", "20ms"]);
    // a scenario file that names `db`, then a draft without `db` and without the inline scenario
    std::fs::create_dir_all(dir.join("scenarios")).unwrap();
    std::fs::write(
        dir.join("scenarios/f.yaml"),
        "chaos:\n  - { node: db, down: true }\n",
    )
    .unwrap();
    let no_db = "version: 1\nnodes:\n  gw: { type: gateway }\n  app: { type: service, spec: openapi.yaml }\nedges:\n  - { from: gw, to: app }\n";
    let file = dir.join("next.yaml");
    std::fs::write(&file, no_db).unwrap();
    ok(&up, &["draft", "set", "-f", file.to_str().unwrap()]);
    let out = ok(&up, &["draft", "apply"]);
    assert!(out.contains("applied generation 2"), "{out}");
    assert!(out.contains("kept: 1 live chaos"), "{out}");
    assert!(
        out.contains("dropped:") && out.contains("scenario db-down"),
        "{out}"
    );
    assert!(out.contains("flagged:") && out.contains("f.yaml"), "{out}");
    // the instance serves the new system: no `db` node any more
    let status = ok(&up, &["status"]);
    assert!(!status.contains("db"), "{status}");
    // the draft is clean again
    let v = one_json(&ok(&up, &["draft", "show", "--json"]));
    assert_eq!(
        (v["dirty"].as_bool(), v["generation"].as_u64()),
        (Some(false), Some(2))
    );
}

#[test]
fn draft_apply_with_a_bad_draft_exits_1_with_the_position_and_changes_nothing() {
    let (up, dir) = scenario_up("draft-apply-bad");
    let bad = base(&dir).replace("{ from: app, to: db }", "{ from: app, to: nope }");
    let file = dir.join("bad.yaml");
    std::fs::write(&file, &bad).unwrap();
    ok(&up, &["draft", "set", "-f", file.to_str().unwrap()]);
    let err = fails(&up, &["draft", "apply"]);
    assert!(err.starts_with("draft:") && err.contains("nope"), "{err}");
    let v = one_json(&ok(&up, &["draft", "show", "--json"]));
    assert_eq!(v["generation"], 1);
}

#[test]
fn draft_commands_honour_json_and_the_timeout_flag() {
    let (up, _dir) = scenario_up("draft-json");
    let v = one_json(&ok(&up, &["draft", "validate", "--json"]));
    assert_eq!(v["ok"], true);
    let v = one_json(&ok(&up, &["draft", "apply", "--timeout", "5s", "--json"]));
    assert_eq!(
        (v["ok"].as_bool(), v["generation"].as_u64()),
        (Some(true), Some(2))
    );
    let err = usage_fails(&up, &["draft", "apply", "--timeout", "soon"]);
    assert!(err.contains("timeout"), "{err}");
    let err = usage_fails(&up, &["draft", "apply", "--timeout", "2m"]);
    assert!(err.contains("60s"), "{err}");
}
