//! Exit code 64 for every kind of bad argument.

use crate::common::*;

#[test]
fn bad_log_filter_exits_64_without_an_instance_in_both_modes() {
    for args in [
        &["logs", "--status", "9xx"][..],
        &["logs", "-f", "--status", "9xx"],
        &["logs", "--since", "yesterday"],
        &["logs", "-f", "--since", "yesterday"],
        &["logs", "--scope", ""],
        &["logs", "-f", "--scope", ""],
    ] {
        assert_64(args);
    }
}

#[test]
fn bad_names_in_set_and_save_exit_64() {
    assert_64(&["scenario", "set", "ok", "Bad Name"]);
    assert_64(&["scenario", "save", "Bad Name"]);
}

#[test]
fn unknown_flag_exits_64() {
    assert_64(&["status", "--bogus"]);
}

#[test]
fn unknown_subcommand_exits_64() {
    assert_64(&["frobnicate"]);
}

#[test]
fn missing_required_arg_exits_64() {
    assert_64(&["trace"]);
    assert_64(&["chaos", "set"]);
}

#[test]
fn conflicting_flags_exit_64() {
    assert_64(&["cycle", "GET /x", "--off", "--state", "error"]);
}

#[test]
fn help_and_version_exit_0() {
    for args in [
        &["--help"][..],
        &["-V"],
        &["--version"],
        &["help", "chaos"],
        &["chaos", "set", "--help"],
    ] {
        let out = bin().args(args).output().unwrap();
        assert_eq!(out.status.code(), Some(0), "{args:?}: {}", stderr(&out));
    }
}

#[test]
fn bare_command_without_subcommand_exits_64() {
    assert_64(&["chaos"]);
    assert_64(&[]);
}

#[test]
fn bad_ttl_exits_64() {
    assert_64(&["scenario", "activate", "db-down", "--ttl", "5"]);
}

#[test]
fn bad_scenario_name_exits_64() {
    assert_64(&["scenario", "show", "active"]);
    assert_64(&["scenario", "deactivate", "Bad Name"]);
}

#[test]
fn bad_state_exits_64() {
    assert_64(&["cycle", "GET /x", "--state", "broken"]);
}

#[test]
fn bad_delay_exits_64() {
    assert_64(&["cycle", "GET /x", "--delay", "soon"]);
}

#[test]
fn bad_from_exits_64() {
    assert_64(&["scenario", "save", "x", "--from", "mine"]);
}

#[test]
fn n_out_of_range_exits_64() {
    assert_64(&["logs", "-n", "0"]);
    assert_64(&["logs", "-n", "1001"]);
}

#[test]
fn empty_scope_exits_64() {
    assert_64(&["logs", "--scope", ""]);
}

#[test]
fn chaos_set_nothing_exits_64() {
    assert_64(&["chaos", "set", "db"]);
}

#[test]
fn bad_latency_exits_64() {
    assert_64(&["chaos", "set", "db", "--latency", "15"]);
}

#[test]
fn bad_rate_exits_64() {
    assert_64(&["chaos", "set", "db", "--error-rate", "150%"]);
}

#[test]
fn scenario_load_bad_file_name_exits_64() {
    let d = scratch("usage-load");
    let f = d.join("Bad Name.yaml");
    std::fs::write(
        &f,
        "chaos: []
",
    )
    .unwrap();
    assert_64(&["scenario", "load", f.to_str().unwrap()]);
}

#[test]
fn a_server_400_is_still_exit_1() {
    let up = Up::start("usage-400");
    // parses here, the server refuses the id
    let e = fails(&up, &["trace", "bad id!"]);
    assert!(e.contains("HTTP 400"), "{e}");
}

// ---- lifecycle: stale state, startup window, stop paths (C2a task 3) ----
