//! Scenario `ttl`, `cycle` and `pins` commands.

use crate::common::*;
use std::time::{Duration, Instant};

#[test]
fn activate_with_ttl_lists_and_expires() {
    let (up, _dir) = scenario_up("scn-ttl");
    let out = ok(&up, &["scenario", "activate", "db-down", "--ttl", "300ms"]);
    assert!(out.contains("active scenarios: db-down"), "{out}");
    let line = |up: &Up| {
        ok(up, &["scenario", "list"])
            .lines()
            .find(|l| l.starts_with("db-down"))
            .unwrap()
            .to_string()
    };
    // The server may already have expired it on a slow box, so only check the format if live.
    let first = line(&up);
    assert!(
        first.contains("(active, expires in ") || !first.contains("(active"),
        "{first}"
    );
    let end = Instant::now() + Duration::from_secs(3);
    while line(&up).contains("(active") {
        assert!(Instant::now() < end, "never expired");
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[test]
fn bad_ttl_is_rejected_here() {
    let (up, _dir) = scenario_up("scn-badttl");
    let e = usage_fails(&up, &["scenario", "activate", "db-down", "--ttl", "5"]);
    assert!(e.contains("bad ttl"), "{e}");
    assert!(!ok(&up, &["scenario", "list"]).contains("(active"));
}

#[test]
fn active_stays_names_only() {
    let (up, _dir) = scenario_up("scn-ttlnames");
    ok(&up, &["scenario", "activate", "db-down", "--ttl", "30s"]);
    assert_eq!(ok(&up, &["scenario", "active"]).trim(), "db-down");
    let list = ok(&up, &["scenario", "list"]);
    assert!(list.contains("(active, expires in "), "{list}");
}

#[test]
fn cycle_steps_and_prints_the_state() {
    let (up, _dir) = scenario_up("cyc-steps");
    for want in ["loading", "empty", "error", "slow", "success", "loading"] {
        let out = ok(&up, &["cycle", "GET /users"]);
        assert!(
            out.contains(&format!("GET /users -> {want}")),
            "{want}: {out}"
        );
    }
}

#[test]
fn cycle_state_flag_and_off() {
    let (up, _dir) = scenario_up("cyc-flags");
    let out = ok(
        &up,
        &[
            "cycle",
            "GET /users",
            "--state",
            "error",
            "--status",
            "404",
            "--delay",
            "2s",
        ],
    );
    assert_eq!(
        out,
        "GET /users -> error (status 404, delay 2s)
"
    );
    assert!(ok(&up, &["pins"]).contains("GET /users"));
    let out = ok(&up, &["cycle", "GET /users", "--off"]);
    assert_eq!(
        out,
        "GET /users -> unpinned
"
    );
    assert_eq!(
        ok(&up, &["pins"]),
        "no pins
",
        "pin removed"
    );
    // no pin to remove: the server answers 204 either way
    let out = ok(&up, &["cycle", "GET /users", "--off"]);
    assert_eq!(
        out,
        "GET /users -> unpinned
"
    );
    // a braced key survives the query encoding
    ok(&up, &["cycle", "GET /users/{id}", "--state", "slow"]);
    assert!(ok(&up, &["pins"]).contains("GET /users/{id}"));
    ok(&up, &["cycle", "GET /users/{id}", "--off"]);
    assert_eq!(
        ok(&up, &["pins"]),
        "no pins
",
        "braced pin removed"
    );
    let e = up.run(&["cycle", "GET /users", "--off", "--state", "error"]);
    assert_ne!(e.status.code(), Some(0), "--off conflicts with --state");
}

#[test]
fn cycle_bad_state_rejected_here() {
    let (up, _dir) = scenario_up("cyc-bad");
    let e = usage_fails(&up, &["cycle", "GET /users", "--state", "broken"]);
    assert!(e.contains("loading") && e.contains("success"), "{e}");
    let e = usage_fails(
        &up,
        &["cycle", "GET /users", "--state", "slow", "--delay", "soon"],
    );
    assert!(e.contains("delay"), "{e}");
    let pins = ok(&up, &["pins"]);
    assert!(!pins.contains("GET /users"), "nothing was sent: {pins}");
}

#[test]
fn cycle_server_400_is_shown_and_changes_nothing() {
    let (up, _dir) = scenario_up("cyc-400");
    // the first step is loading, which takes no status: the server says so and pins nothing
    let e = fails(&up, &["cycle", "GET /users", "--status", "404"]);
    assert!(e.contains("HTTP 400"), "{e}");
    assert!(!ok(&up, &["pins"]).contains("GET /users"));
}

#[test]
fn pins_lists_live_and_effective_with_source() {
    let (up, dir) = scenario_up("pins-list");
    assert!(ok(&up, &["pins"]).contains("no pins"));
    let f = dir.join("pinned.yaml");
    std::fs::write(
        &f,
        "pins:\n  - { endpoint: \"GET /users/{id}\", state: error, status: 404 }\n",
    )
    .unwrap();
    ok(&up, &["scenario", "load", f.to_str().unwrap()]);
    ok(&up, &["cycle", "GET /users", "--state", "slow"]);
    let out = ok(&up, &["pins"]);
    let want = format!(
        "live pins:
  {:<24} slow
effective pins:
  {:<24} {:<8} live
  {:<24} {:<8} scenario:pinned (status 404)
",
        "GET /users", "GET /users", "slow", "GET /users/{id}", "error"
    );
    assert_eq!(out, want);
    // a scenario-only pin leaves the live section empty
    ok(&up, &["cycle", "GET /users", "--off"]);
    let out = ok(&up, &["pins"]);
    let want = format!(
        "no live pins
effective pins:
  {:<24} {:<8} scenario:pinned (status 404)
",
        "GET /users/{id}", "error"
    );
    assert_eq!(out, want);
}

#[test]
fn status_shows_pins() {
    let (up, _dir) = scenario_up("pins-status");
    assert!(!ok(&up, &["status"]).contains("pin "));
    ok(&up, &["cycle", "GET /users", "--state", "error"]);
    let st = ok(&up, &["status"]);
    assert!(
        st.lines().any(|l| l == "pin GET /users: error (live)"),
        "{st}"
    );
}

#[test]
fn scenario_show_prints_pins() {
    let (up, dir) = scenario_up("pins-show");
    let f = dir.join("shown.yaml");
    std::fs::write(
        &f,
        "pins:\n  - { endpoint: \"GET /users\", state: error, status: 404, delay: 2s }\n",
    )
    .unwrap();
    ok(&up, &["scenario", "load", f.to_str().unwrap()]);
    let show = ok(&up, &["scenario", "show", "shown"]);
    assert!(
        show.contains("pin GET /users: error")
            && show.contains("status 404")
            && show.contains("delay 2s"),
        "{show}"
    );
}

#[test]
fn scenario_load_accepts_pins_only_file_and_still_refuses_limits() {
    let (up, dir) = scenario_up("pins-load");
    let write = |n: &str, t: &str| {
        let f = dir.join(n);
        std::fs::write(&f, t).unwrap();
        f
    };
    let good = write(
        "ui-err.yaml",
        "pins:\n  - { endpoint: \"GET /users\", state: loading }\n",
    );
    ok(&up, &["scenario", "load", good.to_str().unwrap()]);
    assert!(ok(&up, &["scenario", "active"]).contains("ui-err"));

    let bad = write(
        "bad-pin.yaml",
        "pins:\n  - { endpoint: \"GET /users\", state: slow, status: 200 }\n",
    );
    let e = fails(&up, &["scenario", "load", bad.to_str().unwrap()]);
    assert_located(&e, "bad-pin.yaml");
    assert!(!ok(&up, &["scenario", "list"]).contains("bad-pin "));

    let lim = write(
        "lim2.yaml",
        "limits: {}\npins:\n  - { endpoint: \"GET /users\", state: loading }\n",
    );
    let e = fails(&up, &["scenario", "load", lim.to_str().unwrap()]);
    assert!(e.contains("not supported"), "{e}");
}

// ---- logs and trace (C1) ----
