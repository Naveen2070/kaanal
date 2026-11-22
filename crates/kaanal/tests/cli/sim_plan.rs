//! `kaanal sim plan`: a thin client of the sizing planner route (Step 9c-1).

use crate::common::*;

/// One slot at 10 ms behind the gateway: about 100 req/s of capacity at `peak` (50 req/s), so 3x
/// (150 req/s) needs a second copy of `app`.
fn plan_up(name: &str) -> Up {
    let dir = project(name, "");
    std::fs::write(
        dir.join("kaanal.yaml"),
        "version: 1\nnodes:\n  gw: { type: gateway }\n  app: { type: service, spec: openapi.yaml, concurrency: 1, proc: 10ms }\nedges:\n  - { from: gw, to: app }\ntraffic:\n  peak:\n    rps: 50\n    journeys: [ { endpoint: 'GET /users' } ]\n",
    )
    .unwrap();
    Up::start_in(dir)
}

fn record(up: &Up, args: &[&str]) -> serde_json::Value {
    let mut a = vec!["sim", "plan", "-p", "peak"];
    a.extend_from_slice(args);
    a.push("--json");
    one_json(&ok(up, &a))
}

#[test]
fn plan_prints_the_steps_and_the_verdict() {
    let up = plan_up("plan-text");
    let out = ok(&up, &["sim", "plan", "-p", "peak", "--target", "3x"]);
    assert!(out.starts_with("plan s1 done (generation 1)"), "{out}");
    assert!(out.contains("Target: 3x peak (150 req/s)"), "{out}");
    assert!(out.contains("p99 <= 1s, errors <= 1%"), "{out}");
    assert!(out.contains("Now: "), "{out}");
    assert!(out.contains("Plan (holds 150 req/s)"), "{out}");
    assert!(out.contains(" 1  app  1 copy -> 2 copies"), "{out}");
    assert!(out.contains("capacity est."), "{out}");
    assert!(out.contains("Checked: "), "{out}");
    assert!(out.contains("not a prediction"), "{out}");
    assert!(!out.contains("will "), "no claim about the future: {out}");
}

#[test]
fn target_parses_a_multiple_and_a_rate_and_rejects_nonsense() {
    let up = plan_up("plan-target");
    let v = record(&up, &["--target=2.5x"]);
    assert_eq!(v["plan"]["request"]["target"]["multiple"], 2.5, "{v}");
    let v = record(&up, &["--target", "150"]);
    assert_eq!(v["plan"]["request"]["target"]["req_s"], 150.0, "{v}");
    assert_eq!(v["plan"]["request"]["factor"], 3.0, "{v}");
    for bad in [
        "", "x", "abc", "0", "0x", "-1", "-2x", "1e999", "nan", "3 x", "3xx",
    ] {
        let arg = format!("--target={bad}");
        let err = usage_fails(&up, &["sim", "plan", "-p", "peak", &arg]);
        assert!(err.contains("--target"), "{bad}: {err}");
    }
    // a missing target is clap's usage error
    usage_fails(&up, &["sim", "plan", "-p", "peak"]);
    // the limits are read as the sweep reads them
    let v = record(&up, &["--target", "2x", "--p99", "500ms", "--errors", "2%"]);
    assert_eq!(v["plan"]["request"]["limit"]["p99_ms"], 500.0, "{v}");
    let err = usage_fails(&up, &["sim", "plan", "--target", "2x", "--p99", "soon"]);
    assert!(err.contains("--p99"), "{err}");
}

#[test]
fn budget_and_keep_are_sent() {
    let up = plan_up("plan-body");
    let v = record(
        &up,
        &[
            "--target=2x",
            "--budget=2.00",
            "--keep=gw",
            "--keep",
            "app",
            "--max-steps",
            "5",
            "--seed=9",
        ],
    );
    let r = &v["plan"]["request"];
    assert_eq!(r["budget_micro_usd_hr"], 2_000_000, "{v}");
    assert_eq!(r["keep"], serde_json::json!(["gw", "app"]), "{v}");
    assert_eq!(r["max_steps"], 5, "{v}");
    assert_eq!(v["seed"], 9, "{v}");
    let v = record(&up, &["--target=2x", "--budget=1.234567", "--max-steps=7"]);
    assert_eq!(
        v["plan"]["request"]["budget_micro_usd_hr"], 1_234_567,
        "{v}"
    );
    assert_eq!(v["plan"]["request"]["max_steps"], 7, "{v}");
    for bad in ["x", "-1", "nan", "inf", ""] {
        let arg = format!("--budget={bad}");
        let err = usage_fails(&up, &["sim", "plan", "--target", "2x", &arg]);
        assert!(err.contains("--budget"), "{bad}: {err}");
    }
    // a node the design lacks is the server's 400
    let err = fails(
        &up,
        &[
            "sim", "plan", "-p", "peak", "--target", "2x", "--keep", "nope",
        ],
    );
    assert!(err.contains("not a node"), "{err}");
}

#[test]
fn report_and_o_write_the_files_and_refuse_to_clobber_without_force() {
    let up = plan_up("plan-files");
    let (report, draft) = (up.dir.join("plan.md"), up.dir.join("plan.yaml"));
    let (r, d) = (report.to_str().unwrap(), draft.to_str().unwrap());
    let out = ok(
        &up,
        &[
            "sim", "plan", "-p", "peak", "--target", "3x", "--report", r, "-o", d,
        ],
    );
    assert!(out.starts_with("plan s1 done"), "{out}");
    assert!(
        std::fs::read_to_string(&report)
            .unwrap()
            .starts_with("# Sizing plan s1")
    );
    let text = std::fs::read_to_string(&draft).unwrap();
    assert!(text.contains("replicas: 2"), "{text}");
    // existing files are refused before the plan is posted: the slot is free and no s2 exists
    for flag in [["--report", r], ["-o", d]] {
        let err = fails(
            &up,
            &[
                "sim", "plan", "-p", "peak", "--target", "3x", flag[0], flag[1],
            ],
        );
        assert!(err.contains("exists; --force overwrites it"), "{err}");
    }
    assert!(!ok(&up, &["sim", "list"]).contains("s2"));
    std::fs::write(&draft, "old").unwrap();
    ok(
        &up,
        &[
            "sim", "plan", "-p", "peak", "--target", "3x", "-o", d, "--force",
        ],
    );
    assert!(
        std::fs::read_to_string(&draft)
            .unwrap()
            .contains("replicas: 2")
    );
    // one file cannot be both the report and the draft
    let err = usage_fails(
        &up,
        &[
            "sim", "plan", "-p", "peak", "--target", "3x", "--report", r, "-o", r, "--force",
        ],
    );
    assert!(err.contains("same file"), "{err}");
    // --force alone has nothing to overwrite
    usage_fails(
        &up,
        &["sim", "plan", "-p", "peak", "--target", "3x", "--force"],
    );
}

#[test]
fn no_wait_conflicts_with_report_and_o() {
    let up = plan_up("plan-nowait");
    for flag in ["--report", "-o"] {
        let err = usage_fails(
            &up,
            &[
                "sim",
                "plan",
                "-p",
                "peak",
                "--target",
                "3x",
                "--no-wait",
                flag,
                "x",
            ],
        );
        assert!(err.contains("--no-wait"), "{err}");
    }
    let id = ok(
        &up,
        &["sim", "plan", "-p", "peak", "--target", "3x", "--no-wait"],
    );
    assert_eq!(id.trim(), "s1");
}

#[test]
fn show_and_report_work_on_a_plan_id() {
    let up = plan_up("plan-show");
    ok(
        &up,
        &["sim", "plan", "-p", "peak", "--target", "3x", "--seed", "4"],
    );
    let shown = ok(&up, &["sim", "show", "s1"]);
    assert!(shown.starts_with("plan s1 done"), "{shown}");
    assert!(shown.contains("Plan (holds 150 req/s)"), "{shown}");
    assert!(
        shown.contains("seed 4") && shown.contains("fingerprint "),
        "{shown}"
    );
    let md = ok(&up, &["sim", "report", "s1"]);
    assert!(md.starts_with("# Sizing plan s1"), "{md}");
    assert!(one_json(&ok(&up, &["sim", "report", "s1", "--json"]))["report_version"].is_number());
    assert!(ok(&up, &["sim", "list"]).contains("plan"));
}

/// The report's re-run command is accepted as printed, and plans the same thing.
#[test]
fn the_reports_re_run_recipe_is_accepted_as_printed() {
    let up = plan_up("plan-recipe");
    ok(
        &up,
        &[
            "sim",
            "plan",
            "-p",
            "peak",
            "--target",
            "3x",
            "--budget",
            "9",
            "--keep",
            "gw",
            "--seed",
            "4",
            "--max-steps",
            "6",
            "--p99",
            "750ms",
            "--errors",
            "2%",
        ],
    );
    let md = ok(&up, &["sim", "report", "s1"]);
    let cmd = md
        .lines()
        .find(|l| l.starts_with("kaanal sim plan "))
        .unwrap_or_else(|| panic!("no recipe: {md}"));
    let args: Vec<&str> = cmd.split_whitespace().skip(1).collect();
    let again = one_json(&ok(&up, &[args.as_slice(), &["--json"]].concat()));
    let first = one_json(&ok(&up, &["sim", "show", "s1", "--json"]));
    for k in [
        "target",
        "factor",
        "req_s",
        "budget_micro_usd_hr",
        "keep",
        "max_steps",
        "limit",
    ] {
        assert_eq!(
            again["plan"]["request"][k], first["plan"]["request"][k],
            "{k}: {cmd}"
        );
    }
    assert_eq!(again["plan"]["steps"], first["plan"]["steps"], "{cmd}");
    assert_eq!(again["seed"], first["seed"]);
    assert_eq!(again["profile"], first["profile"]);
    assert_eq!(
        first["plan"]["request"]["limit"]["p99_ms"], 750.0,
        "{first}"
    );
}
