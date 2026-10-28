//! `scenario` commands: activate, load, save, reset and the `--json` client commands.

use crate::common::*;

/// Every client command, run with `--json`: stdout is empty or all JSON Lines (J2).
#[test]
fn every_client_command_honours_json() {
    let (up, _dir) = scenario_up("json-all");
    let cmds: &[&[&str]] = &[
        &["status"],
        &["pins"],
        &["draft", "show"],
        &["draft", "validate"],
        &["draft", "apply"],
        &["draft", "show"],
        &["draft", "validate"],
        &["draft", "apply"],
        &["scenario", "list"],
        &["scenario", "show", "db-down"],
        &["scenario", "activate", "db-down"],
        &["scenario", "active"],
        &["scenario", "deactivate", "db-down"],
        &["scenario", "set", "db-down"],
        &["scenario", "clear"],
        &["chaos", "set", "db", "--latency", "50ms"],
        &["chaos", "clear", "db"],
        &["node", "down", "db", "--for", "30s"],
        &["node", "up", "db"],
        &["cycle", "GET /users", "--state", "loading"],
        &["cycle", "GET /users", "--off"],
        &["logs", "-n", "5"],
        &["reset"],
    ];
    for c in cmds {
        let mut args = c.to_vec();
        args.push("--json");
        let out = ok(&up, &args);
        for line in out.lines().filter(|l| !l.trim().is_empty()) {
            assert!(
                serde_json::from_str::<serde_json::Value>(line).is_ok(),
                "{c:?}: not JSON: {line}"
            );
        }
    }
    // A 204 prints nothing, a body prints exactly one line.
    assert_eq!(ok(&up, &["chaos", "clear", "db", "--json"]), "");
    assert_eq!(
        ok(&up, &["scenario", "activate", "db-down", "--json"])
            .lines()
            .count(),
        1
    );
}

#[test]
fn live_scenarios_activate_deactivate_and_errors() {
    let (up, dir) = scenario_up("scn-basic");

    let list = ok(&up, &["scenario", "list"]);
    assert!(
        list.contains("db-down") && list.contains("inline"),
        "{list}"
    );
    assert!(!list.contains("(active)"), "{list}");

    ok(&up, &["scenario", "activate", "db-down"]);
    assert!(ok(&up, &["scenario", "list"]).contains("(active)"));
    assert_eq!(up.get_status("/users/42"), 503, "db is down");
    let st = ok(&up, &["status"]);
    assert!(
        st.contains("DOWN") && st.contains("active scenarios: db-down"),
        "{st}"
    );
    let show = ok(&up, &["scenario", "show", "db-down"]);
    assert!(
        show.contains("(inline)") && show.contains("\"down\":true"),
        "{show}"
    );

    // deactivate says what happened
    assert!(ok(&up, &["scenario", "deactivate", "db-down"]).contains("deactivated db-down"));
    assert_eq!(up.get_status("/users/42"), 200);
    assert!(ok(&up, &["scenario", "deactivate", "db-down"]).contains("is not active"));

    ok(&up, &["scenario", "activate", "db-down"]);
    ok(&up, &["scenario", "set"]);
    assert_eq!(up.get_status("/users/42"), 200);
    assert_eq!(ok(&up, &["scenario", "active"]).trim(), "");

    // readable server errors
    let e = fails(&up, &["scenario", "activate", "nope"]);
    assert!(
        e.contains("404") && e.contains("unknown scenario 'nope'"),
        "{e}"
    );
    for reserved in ["active", "save", "reset"] {
        let e = usage_fails(&up, &["scenario", "show", reserved]);
        assert!(e.contains("reserved"), "{reserved}: {e}");
    }
    // a name defined in kaanal.yaml cannot be redefined by load
    let same = dir.join("db-down.yaml");
    std::fs::write(&same, "chaos:\n  - { node: db, latency_ms: 1 }\n").unwrap();
    let e = fails(&up, &["scenario", "load", same.to_str().unwrap()]);
    assert!(e.contains("409"), "{e}");
}

#[test]
fn scenario_load_checks_the_file_here() {
    let (up, dir) = scenario_up("scn-load");
    let write = |n: &str, t: &str| {
        let f = dir.join(n);
        std::fs::write(&f, t).unwrap();
        f
    };

    let good = write("bug-123.yaml", "chaos:\n  - { node: db, latency_ms: 5 }\n");
    ok(&up, &["scenario", "load", good.to_str().unwrap()]);
    assert!(ok(&up, &["scenario", "active"]).contains("bug-123"));
    assert!(ok(&up, &["status"]).contains("+5ms"));

    let bad = write("typo.yaml", "chaos:\n  - { node: db, latncy_ms: 5 }\n");
    let e = fails(&up, &["scenario", "load", bad.to_str().unwrap()]);
    assert!(e.contains("typo.yaml:2:"), "{e}");

    let badname = write("Bad Name.yaml", "chaos:\n  - { node: db, down: true }\n");
    let e = usage_fails(&up, &["scenario", "load", badname.to_str().unwrap()]);
    assert!(e.contains("invalid scenario name"), "{e}");

    // reserved keys are refused here, like the server would, and nothing is defined
    let lim = write(
        "lim.yaml",
        "limits: {}\nchaos:\n  - { node: db, down: true }\n",
    );
    let e = fails(&up, &["scenario", "load", lim.to_str().unwrap()]);
    assert!(e.contains("not supported"), "{e}");
    let list = ok(&up, &["scenario", "list"]);
    assert!(!list.contains("lim "), "{list}");
}

#[test]
fn scenario_load_places_shape_errors_in_the_file_and_sends_nothing() {
    let (up, dir) = scenario_up("scn-shape");
    let cases = [
        (
            "rate.yaml",
            "chaos:\n  - { node: db, error_rate: 2 }\n",
            "error_rate",
        ),
        (
            "kind.yaml",
            "chaos:\n  - { node: db, error_rate: 0.5, error_kind: boom }\n",
            "error_kind",
        ),
        (
            "dup.yaml",
            "chaos:\n  - { node: db, latency_ms: 1 }\n  - { node: db, down: true }\n",
            "same key",
        ),
        ("empty.yaml", "chaos: []\n", "at least one of chaos or pins"),
        (
            "status.yaml",
            "chaos:\n  - { node: db, error_rate: 0.5, error_status: 404 }\n",
            "500-504",
        ),
        (
            "slow.yaml",
            "chaos:\n  - { node: db, latency_ms: 999999 }\n",
            "latency_ms",
        ),
        ("nothing.yaml", "chaos:\n  - { node: db }\n", "sets nothing"),
        (
            "target.yaml",
            "chaos:\n  - { down: true }\n",
            "node, endpoint or edge",
        ),
        (
            "path.yaml",
            "chaos:\n  - { endpoint: users, down: true }\n",
            "METHOD /path",
        ),
    ];
    for (f, text, want) in cases {
        let file = dir.join(f);
        std::fs::write(&file, text).unwrap();
        let e = fails(&up, &["scenario", "load", file.to_str().unwrap()]);
        assert_located(&e, f);
        assert!(e.contains(want), "{f}: {e}");
    }
    // nothing was defined or activated
    let list = ok(&up, &["scenario", "list"]);
    for (f, ..) in cases {
        assert!(
            !list.contains(&format!("{} ", f.trim_end_matches(".yaml"))),
            "{list}"
        );
    }
    assert_eq!(ok(&up, &["scenario", "active"]).trim(), "");
}

#[test]
fn scenario_save_and_reset() {
    let (up, dir) = scenario_up("scn-save");
    let eff_file = dir.join("scenarios/eff.yaml");

    ok(&up, &["chaos", "set", "db", "--latency", "7ms"]);
    ok(&up, &["scenario", "save", "mine"]);
    let yaml = std::fs::read_to_string(dir.join("scenarios/mine.yaml")).unwrap();
    assert!(
        yaml.contains("\"latency_ms\":7") && yaml.contains("\"node\":\"db\""),
        "{yaml}"
    );
    assert!(
        !yaml.contains("error_rate") && !yaml.contains("down"),
        "only set fields: {yaml}"
    );
    let e = fails(&up, &["scenario", "save", "mine"]);
    assert!(e.contains("already exists"), "{e}");
    ok(&up, &["chaos", "set", "db", "--latency", "9ms"]);
    ok(&up, &["scenario", "save", "mine", "--force"]);
    let yaml = std::fs::read_to_string(dir.join("scenarios/mine.yaml")).unwrap();
    assert!(yaml.contains("\"latency_ms\":9"), "forced over: {yaml}");
    let e = usage_fails(&up, &["scenario", "save", "x", "--from", "mine"]);
    assert!(e.contains("scenario:NAME"), "{e}");

    // --clear-live moves the live chaos into the scenario
    ok(&up, &["scenario", "save", "moved", "--clear-live"]);
    assert!(!ok(&up, &["status"]).contains("+9ms"));
    ok(&up, &["scenario", "activate", "moved"]);
    assert!(ok(&up, &["status"]).contains("+9ms"));
    ok(
        &up,
        &["scenario", "save", "copy", "--from", "scenario:mine"],
    );
    let read = |n: &str| std::fs::read_to_string(dir.join(format!("scenarios/{n}.yaml"))).unwrap();
    assert_eq!(read("copy"), read("mine"), "a copy has the same content");
    assert!(read("copy").contains("\"latency_ms\":9"));

    // from effective, then a fresh load gives the same chaos
    ok(&up, &["scenario", "save", "eff", "--from", "effective"]);
    assert!(eff_file.exists());
    let rows = |st: String| -> String {
        st.lines()
            .filter(|l| !l.starts_with("active scenarios"))
            .collect::<Vec<_>>()
            .join("\n")
    };
    let before = rows(ok(&up, &["status"]));
    ok(&up, &["scenario", "clear"]);
    assert!(!ok(&up, &["status"]).contains("+9ms"));
    let again = dir.join("eff2.yaml");
    std::fs::copy(&eff_file, &again).unwrap();
    ok(&up, &["scenario", "load", again.to_str().unwrap()]);
    assert_eq!(rows(ok(&up, &["status"])), before);

    // reset: scenarios off, runtime ones gone, file ones stay
    ok(&up, &["scenario", "activate", "db-down"]);
    ok(&up, &["reset"]);
    let st = ok(&up, &["status"]);
    assert!(
        !st.contains("+9ms") && st.contains("active scenarios: -"),
        "{st}"
    );
    let list = ok(&up, &["scenario", "list"]);
    assert!(list.contains("mine") && list.contains("db-down"), "{list}");
    assert!(!list.contains("eff2"), "runtime scenario undefined: {list}");
}

#[test]
fn status_shows_each_failure_band_at_its_own_share() {
    let dir = project(
        "scn-bands",
        "scenarios:\n  a: { chaos: [ { node: db, error_rate: 0.3, error_kind: reset } ] }\n  b: { chaos: [ { node: db, error_rate: 0.3 } ] }\n",
    );
    let up = Up::start_in(dir);
    ok(&up, &["scenario", "set", "a", "b"]);
    // b is on top: 30% of requests fail with its 503, and of the rest 30% reset (21% overall).
    let st = ok(&up, &["status"]);
    assert!(
        st.contains("30% errors (503), 21% errors (reset)"),
        "not cumulative: {st}"
    );
    assert!(!st.contains("51%"), "{st}");
}

#[test]
fn status_shows_endpoint_rows_and_chaos_warns() {
    let (up, _dir) = scenario_up("scn-rows");
    let out = up.run(&["chaos", "set", "db", "--kind", "hang"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(stderr(&out).contains("warning:"), "{}", stderr(&out));
    ok(&up, &["chaos", "clear"]);

    let ep = "GET /users/{id}";
    ok(&up, &["chaos", "set", "--route", ep, "--latency", "9ms"]);
    ok(
        &up,
        &["chaos", "set", "db", "--route", ep, "--latency", "11ms"],
    );
    let st = ok(&up, &["status"]);
    assert!(st.contains("endpoint GET /users/{id}: +9ms"), "{st}");
    assert!(st.contains("db @ GET /users/{id}: +11ms"), "{st}");
}

#[test]
fn chaos_set_without_any_setting_is_an_error_not_an_empty_patch() {
    let dir = scratch("empty-patch");
    let out = bin()
        .args(["chaos", "set", "db"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(64), "{}", stderr(&out));
    assert!(stderr(&out).contains("nothing to set"), "{}", stderr(&out));
}
