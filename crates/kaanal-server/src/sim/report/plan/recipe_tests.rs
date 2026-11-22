//! The plan report's recipe, refusal, dispatch and hostile-name tests (split from tests.rs for the
//! file-size rule; the fixtures are there).

use std::time::Duration;

use kaanal_sim::plan::{FamilyAnswer, PlanVerdict};
use kaanal_sim::sweep::Limit;

use super::super::report;
use super::tests::{block, class, fx, md, rec, walked, with};
use super::*;
use crate::sim::plan_draft::{SATURATED_COST, TargetAsked};
use crate::sim::store::LiveChaos;

fn command_of(r: &RunRecord) -> String {
    plan_report(r, "v").unwrap().json["recipe"]["command"]
        .as_str()
        .unwrap()
        .to_string()
}

#[test]
fn the_recipe_repeats_the_request_with_the_equals_forms() {
    assert_eq!(
        command_of(&rec()),
        "kaanal sim plan --profile=peak --target=2.5x --p99 1s --errors 1% --budget=2.00 --keep=cache --max-steps 30 --seed 7"
    );
    let r = RunRecord {
        profile: "-p".into(),
        scenario: Some("--help".into()),
        ..with(|p| {
            p.request.target = TargetAsked {
                multiple: None,
                req_s: Some(500.0),
            };
            p.request.budget_micro_usd_hr = None;
            p.request.keep = vec!["-db".into(), "a b".into()];
            p.request.limit = Limit {
                p99: Duration::from_millis(1500),
                error_rate: 0.005,
            };
        })
    };
    assert_eq!(
        command_of(&r),
        "kaanal sim plan --profile=-p --target=500 --p99 '1s 500ms' --errors 0.5% --keep=-db --keep='a b' --max-steps 30 --seed 7 --scenario=--help"
    );
    let inline = RunRecord {
        inline_profile: Some(serde_json::json!({ "rps": 5 })),
        ..rec()
    };
    assert!(command_of(&inline).starts_with("kaanal sim plan -f profile.yaml --target="));
    assert!(md(&inline).contains("profile.yaml: recipe.inline_profile"));
}

#[test]
fn the_recipe_names_the_seed_the_fingerprint_and_the_chaos_digest() {
    let m = md(&rec());
    let recipe = block(&m, "```bash\n");
    assert!(recipe.contains("# seed 7: every candidate draws the same random numbers, so the same seed on the same design repeats the plan"), "{recipe}");
    assert!(
        recipe.contains("# fingerprint 0123456789ab: SHA-256 of the design files the build read"),
        "{recipe}"
    );
    assert!(
        recipe.contains("the fingerprint alone does not"),
        "{recipe}"
    );
    assert!(recipe.contains("chaos digest 9f86d081884c"), "{recipe}");
    assert!(recipe.ends_with("# kaanal 9.9.9"), "{recipe}");
}

#[test]
fn live_chaos_and_a_scenario_are_named_like_the_sweeps() {
    let live = RunRecord {
        live_chaos: Some(LiveChaos {
            scenarios: vec!["slow".into()],
            entries: 2,
            digest: "abcdef012345".into(),
        }),
        ..rec()
    };
    let m = md(&live);
    assert!(m.contains("live chaos in force when this ran"), "{m}");
    assert_eq!(
        plan_report(&live, "v").unwrap().json["recipe"]["chaos"],
        "live"
    );
    let sc = RunRecord {
        scenario: Some("storm".into()),
        live_chaos: None,
        ..rec()
    };
    assert!(command_of(&sc).ends_with(" --scenario=storm"));
    assert_eq!(
        plan_report(&sc, "v").unwrap().json["recipe"]["chaos"],
        "scenario"
    );
}

#[test]
fn a_running_failed_or_other_record_is_refused_and_a_broken_plan_is_internal() {
    let running = RunRecord {
        state: "running",
        plan: None,
        ..rec()
    };
    assert_eq!(
        plan_report(&running, "v").unwrap_err(),
        ReportError::Running
    );
    let failed = RunRecord {
        state: "failed",
        plan: None,
        error: Some("internal: boom".into()),
        ..rec()
    };
    assert_eq!(
        plan_report(&failed, "v").unwrap_err(),
        ReportError::Failed("internal: boom".into())
    );
    let sweep = RunRecord {
        kind: "sweep",
        ..rec()
    };
    assert_eq!(
        plan_report(&sweep, "v").unwrap_err(),
        ReportError::NotAReport
    );
    for broken in [
        RunRecord {
            plan: None,
            ..rec()
        },
        RunRecord {
            seed: None,
            ..rec()
        },
        RunRecord {
            fingerprint: None,
            ..rec()
        },
        RunRecord {
            live_chaos: None,
            ..rec()
        },
        RunRecord {
            chaos_digest: None,
            ..rec()
        },
        with(|p| {
            p.request.target = TargetAsked {
                multiple: None,
                req_s: None,
            };
        }),
    ] {
        assert!(matches!(
            plan_report(&broken, "v").unwrap_err(),
            ReportError::Internal(_)
        ));
    }
}

#[test]
fn the_dispatch_picks_the_maker_by_kind() {
    assert!(
        report(&rec(), "v")
            .unwrap()
            .markdown
            .starts_with("# Sizing plan s4")
    );
    assert_eq!(
        report(
            &RunRecord {
                kind: "run",
                ..rec()
            },
            "v"
        )
        .unwrap_err(),
        ReportError::NotAReport
    );
}

#[test]
fn names_with_markdown_and_format_characters_cannot_break_the_report() {
    let hostile = "x`|y\n```\u{7}\u{202E}z\u{2028}w\u{2029}v";
    let r = RunRecord {
        profile: "my peak|`\r\n#".into(),
        scenario: Some(hostile.into()),
        live_chaos: None,
        ..with(|p| {
            p.request.keep = vec![hostile.into()];
            p.unpriced_nodes = vec![hostile.into()];
            p.steps[0].mv.node = hostile.into();
            p.steps[0].mv.kind = class(hostile, "to|```");
            p.steps[0].menu[0].mv.node = hostile.into();
            p.steps[0].note = Some(format!("# {hostile}"));
            p.verdict = PlanVerdict::CannotHold {
                node: Some(hostile.into()),
                reason: format!("> {hostile}"),
            };
            p.families = vec![
                FamilySection::Skipped {
                    node: hostile.into(),
                    flavor: Some(hostile.into()),
                    reason: format!("- {hostile}"),
                },
                FamilySection::Answer(FamilyAnswer {
                    node: hostile.into(),
                    flavor: hostile.into(),
                    plan: Box::new(walked(
                        PlanVerdict::NoProgress {
                            node: hostile.into(),
                        },
                        Vec::new(),
                        fx(1.0, 0.0, 1.0),
                        1,
                    )),
                    notes: vec![format!("+ {hostile}")],
                }),
            ];
            p.sweep_error = Some(hostile.into());
            p.measured_knee = None;
            p.verify.cost_note = Some(SATURATED_COST);
        })
    };
    let rep = plan_report(&r, "v").unwrap();
    let m = &rep.markdown;
    assert!(m.chars().all(|c| c == '\n' || !c.is_control()), "{m:?}");
    for bad in ['\u{202E}', '\u{2028}', '\u{2029}'] {
        assert!(!m.contains(bad), "{bad:?} in {m:?}");
    }
    // every table line has the header's seven columns: eight unescaped pipes
    let pipes = |l: &str| {
        l.match_indices('|')
            .filter(|(i, _)| !l[..*i].ends_with('\\'))
            .count()
    };
    for l in m.lines().filter(|l| l.starts_with('|')) {
        assert_eq!(pipes(l), 8, "{l}");
    }
    // two fences each for the recipe and the data, and no line of the prose opens one
    assert_eq!(
        m.lines()
            .filter(|l| l.trim_start().starts_with("```"))
            .count(),
        4,
        "{m}"
    );
    let parsed: serde_json::Value = serde_json::from_str(block(m, "```json\n")).unwrap();
    assert_eq!(parsed, rep.json);
    assert_eq!(parsed["recipe"]["scenario"], hostile);
    assert!(
        block(m, "```json\n").contains("\\u202e"),
        "escaped in the data"
    );
    let cmd = block(m, "```bash\n").lines().next().unwrap();
    assert!(cmd.contains("--profile='my peak|`#'"), "{cmd}");
    assert!(cmd.contains("--keep='x`|y```zwv'"), "{cmd}");
    assert!(cmd.ends_with("--scenario='x`|y```zwv'"), "{cmd}");
    // a name that starts a line of the prose cannot open a block there
    assert!(
        m.lines().any(|l| l.starts_with(r"Note on step 1: \# x")),
        "{m}"
    );
    assert!(m.contains("not walked: \\- x"), "{m}");
    assert!(m.contains("  - \\+ x"), "{m}");
}
