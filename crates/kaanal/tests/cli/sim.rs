//! `kaanal sim`, `kaanal sample` and `kaanal skeleton`: thin clients of the simulation routes.

use crate::common::*;

const PROFILES: &str = "traffic:
  peak:
    rps: 50
    journeys: [ { endpoint: 'GET /users' } ]
  big:
    rps: 1000
    journeys: [ { endpoint: 'GET /users' } ]
endpoints:
  'GET /extra': {}
";

fn up(name: &str) -> (Up, std::path::PathBuf) {
    let dir = project(name, PROFILES);
    (Up::start_in(dir.clone()), dir)
}

#[test]
fn sim_run_prints_percentiles_and_the_label() {
    let (up, _) = up("sim-run");
    let out = ok(
        &up,
        &["sim", "run", "--profile", "peak", "--duration", "10s"],
    );
    assert!(out.contains("not a prediction"), "{out}");
    assert!(out.contains("p50") && out.contains("p99"), "{out}");
    assert!(out.contains("busiest nodes"), "{out}");
    assert!(out.starts_with("run s1 done (generation 1)"), "{out}");
}

#[test]
fn sim_run_json_is_the_record() {
    let (up, _) = up("sim-json");
    let v = one_json(&ok(
        &up,
        &["sim", "run", "-p", "peak", "--duration", "10s", "--json"],
    ));
    assert_eq!(v["state"], "done");
    assert!(v["summary"]["requests"].as_u64().unwrap() > 100, "{v}");
    let listed = ok(&up, &["sim", "list"]);
    assert!(listed.contains("s1") && listed.contains("done"), "{listed}");
    assert!(ok(&up, &["sim", "show", "s1"]).contains("p99"));
}

#[test]
fn a_profile_file_runs_and_a_busy_server_refuses_a_second_run() {
    let (up, dir) = up("sim-busy");
    let file = dir.join("p.yaml");
    std::fs::write(
        &file,
        "rps: 1000\njourneys: [ { endpoint: 'GET /users' } ]\n",
    )
    .unwrap();
    let path = file.to_str().unwrap();
    let id = ok(
        &up,
        &[
            "sim",
            "run",
            "-f",
            path,
            "--duration",
            "300s",
            "--warmup",
            "1s",
            "--no-wait",
        ],
    );
    assert_eq!(id.trim(), "s1");
    let err = fails(&up, &["sim", "run", "-p", "peak"]);
    assert!(err.contains("already in progress"), "{err}");
}

#[test]
fn sim_run_needs_a_profile_and_a_good_duration() {
    let (up, _) = up("sim-usage");
    usage_fails(&up, &["sim", "run"]);
    let err = usage_fails(&up, &["sim", "run", "-p", "peak", "--duration", "soon"]);
    assert!(err.contains("--duration"), "{err}");
}

#[test]
fn sim_run_without_a_name_lists_the_profiles_and_a_wrong_name_lists_them_too() {
    let (up, _) = up("sim-names");
    // exit 64 as before (the CLI used to refuse it before asking); only the text is new
    let err = usage_fails(&up, &["sim", "run"]);
    assert_eq!(
        err.trim(),
        "no traffic profile named; name one with -p: big, peak"
    );
    // a name the file lacks: exit 1 (the server's 404), and the names that exist
    let err = fails(&up, &["sim", "run", "-p", "x"]);
    assert!(
        err.trim()
            .ends_with("HTTP 404: no traffic profile named 'x'; defined: big, peak"),
        "{err}"
    );
}

#[test]
fn sim_run_in_a_project_without_traffic_says_what_to_add() {
    let dir = project("sim-none", "");
    let up = Up::start_in(dir);
    let err = usage_fails(&up, &["sim", "run"]);
    assert_eq!(
        err.trim(),
        "no traffic profile is defined: add a traffic: section to kaanal.yaml or pass -f profile.yaml (see docs/web/traffic.md)"
    );
    let err = fails(&up, &["sim", "run", "-p", "peak"]);
    assert!(
        err.contains("no traffic profile named 'peak'; none is defined"),
        "{err}"
    );
}

/// One slot at 10 ms behind the gateway: about 100 req/s of capacity, so `peak` (50 req/s) has a
/// knee inside the default ladder (up to 8x).
fn sweep_up(name: &str) -> Up {
    sweep_up_with(name, "concurrency: 1, proc: 10ms", 50)
}

/// A gateway and `app` (with `app_props`) whose `peak` profile runs at `rps`.
fn sweep_up_with(name: &str, app_props: &str, rps: u64) -> Up {
    let dir = project(name, "");
    std::fs::write(
        dir.join("kaanal.yaml"),
        format!("version: 1\nnodes:\n  gw: {{ type: gateway }}\n  app: {{ type: service, spec: openapi.yaml, {app_props} }}\nedges:\n  - {{ from: gw, to: app }}\ntraffic:\n  peak:\n    rps: {rps}\n    journeys: [ {{ endpoint: 'GET /users' }} ]\n"),
    )
    .unwrap();
    Up::start_in(dir)
}

#[test]
fn sim_sweep_prints_a_cut_ladder_once() {
    // 200 000 req/s x 8 passes the engine's 1e6 req/s per stream, so the sim cuts the ladder;
    // its warning tells it, and the CLI adds no line of its own
    let up = sweep_up_with("sweep-cut", "proc: 1ms", 200_000);
    let out = ok(&up, &["sim", "sweep", "-p", "peak", "--p99", "1h"]);
    assert_eq!(out.matches("ladder cut at ").count(), 1, "{out}");
    // a cut ladder does not claim the top
    assert!(out.contains("held to "), "{out}");
    assert!(!out.contains("holds to "), "{out}");
    assert!(out.contains("warning: ladder cut at "), "{out}");
    assert!(!out.contains("run too large"), "{out}");
}

#[test]
fn sim_sweep_prints_the_knee_the_first_node_and_the_label() {
    let up = sweep_up("sweep-text");
    let out = ok(
        &up,
        &[
            "sim", "sweep", "-p", "peak", "--p99", "500ms", "--errors", "1",
        ],
    );
    assert!(out.starts_with("sweep s1 done (generation 1)"), "{out}");
    assert!(out.contains("knee at ") && out.contains(" req/s"), "{out}");
    assert!(out.contains("p99 over 500ms"), "{out}");
    assert!(out.contains("first to saturate: app at "), "{out}");
    assert!(out.contains("not a prediction"), "{out}");
    assert!(
        out.contains("load req/s") && out.contains("goodput") && out.contains("errors"),
        "{out}"
    );
    // the points table: a row that passes and one that does not, as the last column
    assert!(
        out.lines().any(|l| l.trim_end().ends_with("  yes")),
        "{out}"
    );
    assert!(out.lines().any(|l| l.trim_end().ends_with("  no")), "{out}");
}

#[test]
fn sim_sweep_json_is_the_record() {
    let up = sweep_up("sweep-json");
    let v = one_json(&ok(
        &up,
        &[
            "sim",
            "sweep",
            "-p",
            "peak",
            "--p99",
            "500ms",
            "--errors",
            "2%",
            "--max-factor",
            "4",
            "--seed",
            "7",
            "--json",
        ],
    ));
    assert_eq!(v["state"], "done", "{v}");
    assert_eq!(v["kind"], "sweep", "{v}");
    let sw = &v["sweep"];
    assert_eq!(sw["limit"]["p99_ms"], 500.0, "{v}");
    assert_eq!(sw["limit"]["error_rate"], 0.02, "{v}");
    assert_eq!(sw["verdict"]["kind"], "knee", "{v}");
    assert!(!sw["points"].as_array().unwrap().is_empty(), "{v}");
    // `sim list` tells a sweep from a run
    let listed = ok(&up, &["sim", "list"]);
    assert!(
        listed.contains("s1") && listed.contains("sweep") && listed.contains("done"),
        "{listed}"
    );
}

#[test]
fn sim_sweep_rejects_a_bad_limit() {
    let up = sweep_up("sweep-usage");
    for (flag, bad) in [
        ("--errors", "150%"),
        ("--errors", "-1"),
        ("--errors", "lots"),
        ("--p99", "soon"),
        ("--p99", "0s"),
        ("--max-factor", "nan"),
        ("--max-factor", "inf"),
    ] {
        // `--flag=value`, so clap hands `-1` to the parser instead of reading it as a flag
        let arg = format!("{flag}={bad}");
        let err = usage_fails(&up, &["sim", "sweep", "-p", "peak", &arg]);
        assert!(err.contains(&format!("bad {flag}")), "{arg}: {err}");
    }
    // refused before any request: nothing was started
    assert!(ok(&up, &["sim", "list"]).contains("no runs"));
}

#[test]
fn a_sweep_over_the_limit_at_the_lowest_point_says_so() {
    // 1 ms is under the 10 ms service time: even 0.25x is over
    let up = sweep_up("sweep-broken");
    let out = ok(&up, &["sim", "sweep", "-p", "peak", "--p99", "1ms"]);
    assert!(
        out.contains("over the limit already at the lowest point"),
        "{out}"
    );
    assert!(out.contains("p99 over 1ms"), "{out}");
    assert!(
        !out.lines().any(|l| l.trim_end().ends_with("  yes")),
        "{out}"
    );
}

#[test]
fn a_sweep_that_never_crosses_the_limit_says_how_far_it_held() {
    let up = sweep_up("sweep-holds");
    let out = ok(
        &up,
        &[
            "sim",
            "sweep",
            "-p",
            "peak",
            "--p99",
            "1h",
            "--errors",
            "100",
            "--max-factor",
            "1",
        ],
    );
    assert!(out.contains("holds to "), "{out}");
    assert!(
        !out.lines().any(|l| l.trim_end().ends_with("  no")),
        "{out}"
    );
}

#[test]
fn a_sweep_that_cannot_run_fails_with_its_id_and_the_reason() {
    // a one-hour node makes each point's window ten hours: over the hop caps at the lowest point
    let up = sweep_up_with("sweep-fail", "proc: 1h", 100_000);
    let err = fails(&up, &["sim", "sweep", "-p", "peak"]);
    assert!(err.contains("sweep s1: "), "{err}");
    assert!(err.contains("too large"), "{err}");
    // the record is a failed sweep, shown as one
    let out = ok(&up, &["sim", "show", "s1"]);
    assert!(out.starts_with("sweep s1 failed"), "{out}");
    assert!(out.contains("error: "), "{out}");
}

#[test]
fn sim_show_prints_a_sweep_record() {
    let up = sweep_up("sweep-show");
    let id = ok(&up, &["sim", "sweep", "-p", "peak", "--no-wait"]);
    assert_eq!(id.trim(), "s1");
    // `show` is not `sweep`: wait for the record through `--json` before reading the text
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    while one_json(&ok(&up, &["sim", "show", "s1", "--json"]))["state"] == "running" {
        assert!(
            std::time::Instant::now() < deadline,
            "the sweep never ended"
        );
        std::thread::sleep(std::time::Duration::from_millis(200));
    }
    let out = ok(&up, &["sim", "show", "s1"]);
    assert!(out.starts_with("sweep s1 done"), "{out}");
    assert!(
        out.contains("knee at ") && out.contains("load req/s"),
        "{out}"
    );
    assert!(
        !out.contains("busiest nodes"),
        "a sweep is not printed as a run: {out}"
    );
}

#[test]
fn sample_prints_a_body_that_parses() {
    let (up, _) = up("sample");
    let out = ok(&up, &["sample", "GET /users"]);
    let v: serde_json::Value = serde_json::from_str(&out).unwrap();
    assert!(v.is_array(), "{out}");
    let err = fails(&up, &["sample", "GET /nope"]);
    assert!(err.contains("no route"), "{err}");
}

#[test]
fn skeleton_to_a_file_refuses_to_overwrite_without_force() {
    let (up, dir) = up("skeleton");
    let file = dir.join("skeleton.yaml");
    let path = file.to_str().unwrap();
    let printed = ok(&up, &["skeleton"]);
    assert!(
        printed.contains("x-kaanal-skeleton") && printed.contains("/extra"),
        "{printed}"
    );
    assert!(ok(&up, &["skeleton", "-o", path]).contains("wrote"));
    assert!(std::fs::read_to_string(&file).unwrap().contains("/extra"));
    let err = fails(&up, &["skeleton", "-o", path]);
    assert!(err.contains("--force"), "{err}");
    ok(&up, &["skeleton", "-o", path, "--force"]);
}
