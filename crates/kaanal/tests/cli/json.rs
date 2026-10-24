//! The global `--json` switch.

use crate::common::*;

#[test]
fn json_flag_works_before_and_after_the_subcommand() {
    let up = Up::start("json-pos");
    for args in [["status", "--json"], ["--json", "status"]] {
        let rows = one_json(&ok(&up, &args));
        let rows = rows.as_array().expect("an array");
        assert!(
            rows.iter()
                .all(|r| r["node"].is_string() && r["type"].is_string())
        );
        assert!(rows.iter().all(|r| r["state"] == "up"), "{rows:?}");
        assert!(rows.iter().any(|r| r["node"] == "db"));
    }
}

#[test]
fn json_env_switches_it_on_and_empty_env_is_off() {
    let up = Up::start("json-env");
    let run = |v: &str| {
        let out = bin()
            .arg("status")
            .env("KAANAL_JSON", v)
            .current_dir(&up.dir)
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
        stdout(&out)
    };
    assert!(one_json(&run("1")).is_array());
    assert!(one_json(&run("true")).is_array());
    assert!(run("").starts_with("NODE"));
}

#[test]
fn json_pins_and_scenario_reads_parse() {
    let (up, _dir) = scenario_up("json-reads");
    assert!(one_json(&ok(&up, &["pins", "--json"])).is_object());
    assert!(one_json(&ok(&up, &["scenario", "list", "--json"])).is_array());
    assert!(one_json(&ok(&up, &["scenario", "active", "--json"])).is_object());
    let v = one_json(&ok(&up, &["scenario", "show", "db-down", "--json"]));
    assert!(v["chaos"].is_array(), "{v}");
}

#[test]
fn logs_and_trace_json_output_is_unchanged() {
    let up = Up::start("json-logs");
    let (_, id) = up.hit("/health", Some("g-1"));
    let rows = ok(&up, &["--json", "logs"]);
    assert_eq!(one_json(&rows)["client_id"], "g-1");
    assert_eq!(rows, ok(&up, &["logs", "--json"]));
    assert_eq!(
        one_json(&ok(&up, &["--json", "trace", &id]))["id"],
        id.as_str()
    );
}

#[test]
fn json_mode_keeps_errors_on_stderr() {
    let dir = scratch("json-err");
    let out = bin()
        .args(["status", "--json"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(stdout(&out), "");
    assert!(stderr(&out).contains("not running"), "{}", stderr(&out));
}
