//! `kaanal sim plan --cheapest`: flags, body and text (Step 9c-2).

use crate::common::*;

/// The `sim_plan` design: one slot at 10 ms behind the gateway, 50 req/s at `peak`.
fn plan_up(name: &str) -> Up {
    plan_up_with(name, "")
}

/// The same design with `extra` on `app` (an `instance:` makes it translatable).
fn plan_up_with(name: &str, extra: &str) -> Up {
    let dir = project(name, "");
    std::fs::write(
        dir.join("kaanal.yaml"),
        format!("version: 1\nnodes:\n  gw: {{ type: gateway }}\n  app: {{ type: service, spec: openapi.yaml, concurrency: 1, proc: 10ms{extra} }}\nedges:\n  - {{ from: gw, to: app }}\ntraffic:\n  peak:\n    rps: 50\n    journeys: [ {{ endpoint: 'GET /users' }} ]\n"),
    )
    .unwrap();
    Up::start_in(dir)
}

fn record(up: &Up, args: &[&str]) -> serde_json::Value {
    let mut a = vec!["sim", "plan", "-p", "peak", "--target", "3x"];
    a.extend_from_slice(args);
    a.push("--json");
    one_json(&ok(up, &a))
}

#[test]
fn cheapest_prints_the_search_the_comparison_and_the_sections() {
    let up = plan_up_with("cheapest-text", ", instance: aws/m5.large");
    let out = ok(
        &up,
        &[
            "sim",
            "plan",
            "-p",
            "peak",
            "--target",
            "3x",
            "--cheapest",
            "--time",
            "60s",
            "--run-cap",
            "20",
            "--providers",
            "gcp,azure",
        ],
    );
    for want in [
        "cheapest found within the limit, not proven cheapest",
        "Search:",
        "stopped because ",
        "runs ",
        "Comparison (",
        "the design in force",
        "the design translated to GCP",
        "the design translated to Azure",
        "If it ran on GCP: ",
        "If it ran on Azure: ",
    ] {
        assert!(out.contains(want), "missing {want:?} in:\n{out}");
    }
    // all three designs hold and the design in force, with the changes, is the cheapest
    assert!(
        out.contains("Winner: the design in force, with the changes above, $")
            && out.contains("not proven cheapest."),
        "{out}"
    );
    assert!(
        out.contains("    1  the design in force, with the changes above: "),
        "{out}"
    );
    assert!(!out.contains("recommend"), "{out}");
    assert!(!out.contains("will "), "no claim about the future: {out}");
}

#[test]
fn time_run_cap_and_providers_need_cheapest() {
    let up = plan_up("cheapest-needs");
    for flag in [["--time", "5s"], ["--run-cap", "5"], ["--providers", "aws"]] {
        usage_fails(
            &up,
            &[
                "sim", "plan", "-p", "peak", "--target", "3x", flag[0], flag[1],
            ],
        );
    }
}

#[test]
fn time_parses_seconds_and_minutes_and_rejects_nonsense() {
    let up = plan_up("cheapest-time");
    for (t, want) in [("1500ms", 2), ("2m", 120), ("10m", 600), ("7s", 7)] {
        let v = record(
            &up,
            &[
                "--cheapest",
                "--time",
                t,
                "--run-cap",
                "1",
                "--providers",
                "none",
            ],
        );
        assert_eq!(
            v["plan"]["request"]["cheapest"]["time_limit_s"], want,
            "{t}: {v}"
        );
    }
    for bad in ["soon", "0s", "601s", "11m", "", "-3s"] {
        let arg = format!("--time={bad}");
        let err = usage_fails(
            &up,
            &[
                "sim",
                "plan",
                "-p",
                "peak",
                "--target",
                "3x",
                "--cheapest",
                &arg,
            ],
        );
        assert!(err.contains("--time"), "{bad}: {err}");
    }
    for bad in ["0", "2001", "x", "-1"] {
        let arg = format!("--run-cap={bad}");
        usage_fails(
            &up,
            &[
                "sim",
                "plan",
                "-p",
                "peak",
                "--target",
                "3x",
                "--cheapest",
                &arg,
            ],
        );
    }
    for bad in ["x", "none,aws", "aws,", "", "aws,,gcp"] {
        let arg = format!("--providers={bad}");
        let err = usage_fails(
            &up,
            &[
                "sim",
                "plan",
                "-p",
                "peak",
                "--target",
                "3x",
                "--cheapest",
                &arg,
            ],
        );
        assert!(err.contains("--providers"), "{bad}: {err}");
    }
}

#[test]
fn the_flags_are_sent_in_the_body() {
    let up = plan_up("cheapest-body");
    let v = record(
        &up,
        &[
            "--cheapest",
            "--time=30s",
            "--run-cap=7",
            "--providers=none",
        ],
    );
    assert_eq!(
        v["plan"]["request"]["cheapest"],
        serde_json::json!({ "time_limit_s": 30, "run_cap": 7, "providers": [] }),
        "{v}"
    );
    let v = record(&up, &["--cheapest", "--run-cap=3", "--providers=azure,gcp"]);
    let c = &v["plan"]["request"]["cheapest"];
    assert_eq!(
        (&c["time_limit_s"], &c["run_cap"]),
        (&150.into(), &3.into()),
        "{v}"
    );
    assert_eq!(c["providers"], serde_json::json!(["gcp", "azure"]), "{v}");
}

/// The recipe in the report, run as written, plans the same request.
#[test]
fn the_recipe_in_the_report_runs_the_same_plan() {
    let up = plan_up("cheapest-recipe");
    let report = up.dir.join("plan.md");
    let r = report.to_str().unwrap();
    let first = one_json(&ok(
        &up,
        &[
            "sim",
            "plan",
            "-p",
            "peak",
            "--target",
            "3x",
            "--cheapest",
            "--time",
            "1500ms",
            "--run-cap",
            "4",
            "--providers",
            "none",
            "--seed",
            "3",
            "--report",
            r,
            "--json",
        ],
    ));
    let text = std::fs::read_to_string(&report).unwrap();
    let recipe = text
        .lines()
        .find(|l| l.contains("kaanal sim plan"))
        .unwrap_or_else(|| panic!("no recipe in {text}"));
    assert!(
        recipe.contains("--cheapest --time=2s --run-cap=4 --providers=none"),
        "{recipe}"
    );
    let words: Vec<&str> = recipe
        .trim()
        .trim_matches('`')
        .split_whitespace()
        .skip(1)
        .collect();
    let mut a = words.clone();
    a.push("--json");
    let again = one_json(&ok(&up, &a));
    assert_eq!(
        first["plan"]["request"], again["plan"]["request"],
        "{recipe}"
    );
}

#[test]
fn a_plain_plan_prints_as_before() {
    let up = plan_up("cheapest-plain");
    let out = ok(&up, &["sim", "plan", "-p", "peak", "--target", "3x"]);
    for no in ["Search:", "Comparison", "cheapest", "If it ran on"] {
        assert!(!out.contains(no), "{no}: {out}");
    }
    let v = record(&up, &[]);
    assert!(v["plan"]["request"]["cheapest"].is_null(), "{v}");
    assert!(v["plan"]["search"].is_null(), "{v}");
}
