//! `kaanal node down|up|list` and timed chaos.

use crate::common::*;
use std::time::Duration;

#[test]
fn node_down_for_reverts_by_itself() {
    let (up, _dir) = scenario_up("node-down-for");
    let out = ok(&up, &["node", "down", "db", "--for", "3s"]);
    assert!(out.contains("(reverts in 3s)"), "{out}");
    assert!(ok(&up, &["status"]).contains("DOWN"));
    std::thread::sleep(Duration::from_millis(3700));
    assert!(!ok(&up, &["status"]).contains("DOWN"));
}

#[test]
fn node_up_clears_only_the_down_flag() {
    let (up, _dir) = scenario_up("node-up-keeps");
    ok(&up, &["chaos", "set", "db", "--latency", "50ms"]);
    ok(&up, &["node", "down", "db"]);
    assert!(ok(&up, &["status"]).contains("DOWN"));
    ok(&up, &["node", "up", "db"]);
    let st = ok(&up, &["status"]);
    assert!(!st.contains("DOWN") && st.contains("+50ms"), "{st}");
}

#[test]
fn node_up_is_a_noop_exit_0_when_nothing_is_down() {
    let (up, _dir) = scenario_up("node-up-noop");
    ok(&up, &["node", "up", "db"]);
}

#[test]
fn node_up_exits_1_when_a_scenario_holds_it_down() {
    let (up, _dir) = scenario_up("node-up-held");
    ok(&up, &["scenario", "activate", "db-down"]);
    let e = fails(&up, &["node", "up", "db"]);
    assert!(
        e.contains("db is still down: held by an active scenario or override"),
        "{e}"
    );
    assert!(e.contains("kaanal scenario active"), "{e}");
}

#[test]
fn node_down_unknown_node_is_exit_1() {
    let (up, _dir) = scenario_up("node-down-unknown");
    let e = fails(&up, &["node", "down", "nope"]);
    assert!(e.contains("unknown node 'nope'"), "{e}");
}

#[test]
fn chaos_set_for_sends_a_ttl_and_a_bad_value_is_64() {
    let (up, _dir) = scenario_up("chaos-for");
    let out = ok(&up, &["chaos", "set", "db", "--down", "--for", "30s"]);
    assert!(out.contains("(reverts in 30s)"), "{out}");
    // A ttl was sent: the lease runs out by itself.
    ok(&up, &["chaos", "set", "db", "--down", "--for", "3s"]);
    assert!(ok(&up, &["status"]).contains("DOWN"));
    std::thread::sleep(Duration::from_millis(3700));
    assert!(!ok(&up, &["status"]).contains("DOWN"));
    for bad in ["5", "soon", "50ms", "25h"] {
        let e = usage_fails(&up, &["chaos", "set", "db", "--down", "--for", bad]);
        assert!(e.contains("bad --for"), "{bad}: {e}");
        let e = usage_fails(&up, &["node", "down", "db", "--for", bad]);
        assert!(e.contains("bad --for"), "{bad}: {e}");
    }
}

#[test]
fn completions_mention_node() {
    let out = bin().args(["completions", "bash"]).output().unwrap();
    assert!(stdout(&out).contains("node"));
}

#[test]
fn node_list_matches_the_status_table() {
    let up = Up::start("node-list-table");
    ok(&up, &["chaos", "set", "db", "--latency", "300ms"]);
    let status = ok(&up, &["status"]);
    let table: Vec<&str> = status
        .lines()
        .take_while(|l| !l.starts_with("active"))
        .collect();
    assert!(
        table[0].starts_with("NODE") && table[0].contains("STATE"),
        "{status}"
    );
    assert_eq!(
        ok(&up, &["node", "list"]),
        format!("{}\n", table.join("\n"))
    );
}

#[test]
fn node_list_shows_state_and_the_revert_countdown() {
    let up = Up::start("node-list-down");
    ok(&up, &["node", "down", "db", "--for", "30s"]);
    let t = ok(&up, &["node", "list"]);
    let db = t.lines().find(|l| l.starts_with("db")).expect("db row");
    assert_eq!(db.split_whitespace().nth(3), Some("down"), "{t}");
    assert!(db.contains("reverts in"), "{t}");
}

#[test]
fn node_list_json_is_an_array_with_state() {
    let up = Up::start("node-list-json");
    ok(&up, &["node", "down", "db", "--for", "30s"]);
    let rows = one_json(&ok(&up, &["node", "list", "--json"]));
    let db = rows
        .as_array()
        .expect("array")
        .iter()
        .find(|r| r["node"] == "db")
        .expect("db");
    assert_eq!(db["state"], "down");
    assert!(
        db["expires_in_ms"].as_u64().is_some_and(|ms| ms > 0),
        "{db}"
    );
}

#[test]
fn endpoint_flag_may_follow_the_subcommand() {
    let up = Up::start("endpoint-after");
    let (port, token) = port_and_token(&up);
    let elsewhere = scratch("endpoint-after-elsewhere");
    let url = format!("http://127.0.0.1:{port}");
    let envs = [("KAANAL_TOKEN", token.as_str())];
    // Where no subcommand flag collides.
    let out = remote(&elsewhere, &["status", "--endpoint", &url], &envs);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(stdout(&out).contains("postgres"), "{}", stdout(&out));
    // Where the old route-key spelling `--endpoint` also exists, a URL still means the instance (`Cli::fix_route`).
    let out = remote(
        &elsewhere,
        &["chaos", "set", "db", "--down", "--endpoint", &url],
        &envs,
    );
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let out = remote(&elsewhere, &["logs", "--endpoint", &url], &envs);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let out = remote(&elsewhere, &["chaos", "clear", "--endpoint", &url], &envs);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
}

#[test]
fn old_endpoint_spelling_still_names_a_route() {
    let up = Up::start("route-alias");
    ok(
        &up,
        &["chaos", "set", "--endpoint", "GET /users/{id}", "--down"],
    );
    assert_eq!(up.get_status("/users/42"), 503, "the old spelling works");
    ok(&up, &["chaos", "clear", "--endpoint", "GET /users/{id}"]);
    assert_eq!(up.get_status("/users/42"), 200);
    let out = up.run(&["chaos", "set", "--help"]);
    let help = stdout(&out);
    assert!(help.contains("--route"), "{help}");
}
