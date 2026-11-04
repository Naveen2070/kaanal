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
