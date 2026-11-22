//! The plan report's unit tests (the sweep report's are in ../tests.rs): the golden file, every
//! verdict, hostile names, the refusals, the recipe.

use std::collections::BTreeMap;
use std::sync::Arc;

use kaanal_core::Config;
use kaanal_core::cost::CostReport;
use kaanal_core::resize::{Move, MoveKind};
use kaanal_sim::plan::{Candidate, Effect, FamilyAnswer, Plan, PlanVerdict, Step};
use kaanal_sim::sweep::{Limit, Over, Point, Verdict};

use super::*;
use crate::sim::plan_draft::{BudgetCheck, LABEL, PlanAsked, SATURATED_COST, TargetAsked, Verify};
use crate::sim::store::LiveChaos;

const GOLDEN: &str = include_str!("../../../../tests/golden/plan-report.md");

pub(super) fn fx(p99_ms: f64, error_rate: f64, capacity_req_s: f64) -> Effect {
    Effect {
        p99_ms,
        error_rate,
        capacity_req_s: Some(capacity_req_s),
    }
}

pub(super) fn mv(node: &str, kind: MoveKind) -> Move {
    Move {
        node: node.into(),
        kind,
    }
}

pub(super) fn class(from: &str, to: &str) -> MoveKind {
    MoveKind::Class {
        from: from.into(),
        to: to.into(),
    }
}

pub(super) fn cand(m: Move, price: u64, effect: Effect, holds: bool) -> Candidate {
    Candidate {
        mv: m,
        price_micro_usd_hr: Some(price),
        effect,
        holds,
    }
}

/// Two steps: the database to the next class (the plan still over the limit), then an app copy.
pub(super) fn steps() -> Vec<Step> {
    let db = mv("db", class("db.m5.large", "db.m5.2xlarge"));
    let app = mv("app", MoveKind::Copies { from: 2, to: 3 });
    vec![
        Step {
            mv: db.clone(),
            price_micro_usd_hr: Some(1_420_000),
            price_delta_micro_usd_hr: Some(580_000),
            before: fx(4100.0, 0.05, 49.0),
            after: fx(1200.0, 0.002, 118.0),
            menu: vec![
                cand(
                    mv("db", class("db.m5.large", "db.m5.xlarge")),
                    1_130_000,
                    fx(2900.0, 0.015, 83.0),
                    false,
                ),
                cand(db, 1_420_000, fx(1200.0, 0.002, 118.0), false),
                cand(
                    mv("db", MoveKind::Copies { from: 1, to: 2 }),
                    1_090_000,
                    fx(3300.0, 0.02, 77.0),
                    false,
                ),
            ],
            note: None,
        },
        Step {
            mv: app.clone(),
            price_micro_usd_hr: Some(1_620_000),
            price_delta_micro_usd_hr: Some(200_000),
            before: fx(1200.0, 0.002, 118.0),
            after: fx(300.0, 0.0, 251.0),
            menu: vec![
                cand(app, 1_620_000, fx(300.0, 0.0, 251.0), true),
                cand(
                    mv("app", class("c5.large", "c5.xlarge")),
                    1_700_000,
                    fx(800.0, 0.001, 160.0),
                    true,
                ),
            ],
            note: Some("a second copy of app halves its queue".into()),
        },
    ]
}

pub(super) fn config() -> Config {
    Config::from_yaml("version: 1\nnodes:\n  nginx: { type: gateway, proc: 1ms }\n").unwrap()
}

pub(super) fn walked(verdict: PlanVerdict, steps: Vec<Step>, end: Effect, after: u64) -> Plan {
    Plan {
        verdict,
        steps,
        start: fx(4100.0, 0.05, 49.0),
        end,
        price_before: Some(840_000),
        price_after: Some(after),
        unpriced_nodes: Vec::new(),
        config: config(),
        families: Vec::new(),
    }
}

pub(super) fn verify(holds: bool) -> Verify {
    Verify {
        holds,
        p99_ms: 300.0,
        error_rate: 0.0,
        capacity_req_s: Some(251.0),
        cost: CostReport {
            nodes: Vec::new(),
            total_micro_usd_hr: 1_634_500,
            priced: 3,
            unpriced: 1,
            regions: BTreeMap::new(),
            egress: Vec::new(),
        },
        cost_note: None,
    }
}

/// The walk on the changed database: its own start, and a step that begins where that start ends.
fn family_plan() -> Plan {
    let start = fx(1500.0, 0.01, 150.0);
    let step = Step {
        mv: mv("app", MoveKind::Copies { from: 2, to: 3 }),
        price_micro_usd_hr: Some(1_200_000),
        price_delta_micro_usd_hr: Some(500_000),
        before: start.clone(),
        after: fx(280.0, 0.0, 300.0),
        menu: Vec::new(),
        note: None,
    };
    Plan {
        start,
        price_before: Some(700_000),
        ..walked(
            PlanVerdict::Holds,
            vec![step],
            fx(280.0, 0.0, 300.0),
            1_200_000,
        )
    }
}

pub(super) fn doc() -> PlanDoc {
    PlanDoc {
        label: LABEL,
        request: PlanAsked {
            target: TargetAsked {
                multiple: Some(2.5),
                req_s: None,
            },
            factor: 2.5,
            req_s: 250.0,
            limit: Limit::default(),
            budget_micro_usd_hr: Some(2_000_000),
            keep: vec!["cache".into()],
            max_steps: 30,
        },
        verdict: PlanVerdict::Holds,
        budget_check: Some(BudgetCheck {
            budget_micro_usd_hr: 2_000_000,
            cost_micro_usd_hr: 1_634_500,
            rejudged: false,
        }),
        steps: steps(),
        start: fx(4100.0, 0.05, 49.0),
        end: fx(300.0, 0.0, 251.0),
        price_before: Some(840_000),
        price_after: Some(1_620_000),
        unpriced_nodes: vec!["cdn".into()],
        families: vec![
            FamilySection::Answer(FamilyAnswer {
                node: "db".into(),
                flavor: "dynamodb".into(),
                plan: Box::new(family_plan()),
                notes: vec![
                    "the table is priced as provisioned throughput".into(),
                    "assumes your application can use this family; preset estimates".into(),
                ],
            }),
            FamilySection::Skipped {
                node: "db".into(),
                flavor: Some("mongodb".into()),
                reason: "no preset instance table for this flavor".into(),
            },
        ],
        verify: verify(true),
        measured_knee: Some(Verdict::Knee {
            k: 2.62,
            req_s: 262.0,
        }),
        sweep_error: None,
    }
}

/// A finished plan record.
pub(super) fn rec() -> RunRecord {
    RunRecord {
        id: "s4".into(),
        kind: "plan",
        state: "done",
        generation: 2,
        profile: "peak".into(),
        scenario: None,
        source: "applied",
        draft_rev: None,
        started_ms: 1,
        finished_ms: Some(2),
        summary: None,
        cost: None,
        error: None,
        sweep_points: Vec::new(),
        sweep: None,
        plan_steps: steps(),
        plan: Some(doc()),
        proposed_draft: Some("version: 1\n".into()),
        seed: Some(7),
        max_factor: None,
        fingerprint: Some("0123456789ab".into()),
        inline_profile: None,
        live_chaos: Some(LiveChaos {
            scenarios: Vec::new(),
            entries: 0,
            digest: "e3b0c44298fc".into(),
        }),
        chaos_digest: Some("9f86d081884c".into()),
        runtime_scenarios: Vec::new(),
        log: Arc::default(),
    }
}

pub(super) fn with(f: impl FnOnce(&mut PlanDoc)) -> RunRecord {
    let mut r = rec();
    f(r.plan.as_mut().unwrap());
    r
}

/// The text of the first fenced block opened by `fence`.
pub(super) fn block<'a>(md: &'a str, fence: &str) -> &'a str {
    let start = md.find(fence).expect("the block") + fence.len();
    let end = start + md[start..].find("\n```\n").expect("closed");
    &md[start..end]
}

pub(super) fn md(r: &RunRecord) -> String {
    plan_report(r, "9.9.9").unwrap().markdown
}

#[test]
fn plan_report_matches_the_golden_file() {
    let rep = plan_report(&rec(), "9.9.9").unwrap();
    assert_eq!(rep.markdown, GOLDEN);
}

#[test]
fn the_json_block_parses_back_to_the_served_json() {
    let rep = plan_report(&rec(), "9.9.9").unwrap();
    let text = block(&rep.markdown, "```json\n");
    assert_eq!(text, serde_json::to_string_pretty(&rep.json).unwrap());
    let parsed: serde_json::Value = serde_json::from_str(text).unwrap();
    assert_eq!(parsed, rep.json);
    assert_eq!(rep.json["report_version"], 1);
    assert_eq!(rep.json["label"], LABEL);
    assert_eq!(rep.json["plan"]["steps"].as_array().unwrap().len(), 2);
    assert_eq!(rep.json["plan"]["verdict"]["kind"], "holds");
    assert_eq!(rep.json["plan"]["families"][0]["kind"], "answer");
    assert_eq!(rep.json["plan"]["families"][1]["kind"], "skipped");
    assert_eq!(rep.json["recipe"]["seed"], 7);
    assert_eq!(rep.json["recipe"]["kaanal_version"], "9.9.9");
}

#[test]
fn every_report_says_it_is_not_a_prediction() {
    for verdict in all_verdicts() {
        let m = md(&with(|p| p.verdict = verdict));
        assert!(m.contains("This plan is not a prediction."), "{m}");
        assert!(m.contains("not a prediction; prices are estimates"), "{m}");
    }
}

fn all_verdicts() -> Vec<PlanVerdict> {
    vec![
        PlanVerdict::AlreadyHolds,
        PlanVerdict::Holds,
        PlanVerdict::CannotHold {
            node: Some("db".into()),
            reason: "its class is the largest of the series".into(),
        },
        PlanVerdict::OverBudget {
            over_by_micro_usd_hr: 250_000,
            holds: false,
        },
        PlanVerdict::NoProgress { node: "app".into() },
        PlanVerdict::StepCap,
    ]
}

/// The `## Verdict` paragraph of a report for `p`.
fn verdict_of(r: &RunRecord) -> String {
    let m = md(r);
    let at = m.find("## Verdict\n\n").expect("a verdict") + "## Verdict\n\n".len();
    m[at..].split("\n\n").next().unwrap().to_string()
}

#[test]
fn every_verdict_has_honest_words() {
    let v = |verdict, holds| {
        verdict_of(&with(|p| {
            p.verdict = verdict;
            p.verify.holds = holds;
        }))
    };
    assert_eq!(
        v(PlanVerdict::AlreadyHolds, true),
        "The design in force already holds the target: nothing to change."
    );
    assert_eq!(v(PlanVerdict::Holds, true), "The plan holds the target.");
    let cannot = v(
        PlanVerdict::CannotHold {
            node: Some("db".into()),
            reason: "its class is the largest of the series.".into(),
        },
        false,
    );
    assert_eq!(
        cannot,
        "The plan cannot hold the target by sizing at 'db': its class is the largest of the series. The design after the steps taken does not hold the target."
    );
    // no node saturates: the reason names the crossed limit and no node is invented
    let none = v(
        PlanVerdict::CannotHold {
            node: None,
            reason: "no node saturates at the target: the limit is crossed by p99 latency".into(),
        },
        false,
    );
    assert!(
        none.starts_with("The plan cannot hold the target by sizing: no node saturates"),
        "{none}"
    );
    assert!(!none.contains(" at '"), "{none}");
    // over budget, holding: the cheapest design found is dear; says by how much
    let dear = v(
        PlanVerdict::OverBudget {
            over_by_micro_usd_hr: 250_000,
            holds: true,
        },
        true,
    );
    assert_eq!(
        dear,
        "The plan holds the target but is over budget by $0.25/h: the cheapest holding design found costs $2.25/h against a budget of $2.00/h."
    );
    // over budget, not holding: the plan is where the walk stopped and does not hold
    let stopped = v(
        PlanVerdict::OverBudget {
            over_by_micro_usd_hr: 250_000,
            holds: false,
        },
        false,
    );
    assert_eq!(
        stopped,
        "No holding design was found within the budget. The plan shown costs $1.62/h without per-use prices and does not hold the target; the next step needed would cost $2.25/h, over the budget of $2.00/h by $0.25/h, so the plan stops here."
    );
    // the two no-gain steps can be on different nodes: only the second is named, and no claim
    // is made about sizing 'app' further
    let stall = v(PlanVerdict::NoProgress { node: "app".into() }, false);
    assert_eq!(
        stall,
        "The plan stops: the last two steps the walk tried gained nothing (the second on 'app'), and they are not in the plan. The design where it stops does not hold the target."
    );
    assert!(!stall.contains("more size"), "{stall}");
    assert_eq!(
        v(PlanVerdict::StepCap, false),
        "The plan stops after 30 steps, the step cap, without holding the target."
    );
}

#[test]
fn unpriced_nodes_are_named_and_a_fully_priced_design_says_so() {
    assert!(
        md(&rec()).contains("Not priced: cdn. They are left out of every price and of the budget.")
    );
    let all = md(&with(|p| p.unpriced_nodes.clear()));
    assert!(all.contains("Every node is priced."), "{all}");
    assert!(!all.contains("Not priced:"), "{all}");
}

#[test]
fn a_family_answer_says_what_it_assumes_and_a_skip_says_why() {
    let m = md(&rec());
    assert!(
        m.contains("Every answer assumes your application can use this family; preset estimates."),
        "{m}"
    );
    assert!(
        m.contains(
            "- db as dynamodb:\n  - The plan holds the target.\n  - 1 step taken, ending at:"
        ),
        "{m}"
    );
    assert!(
        m.contains("  - assumes your application can use this family; preset estimates\n"),
        "each answer carries it too: {m}"
    );
    assert!(
        m.contains("- db as mongodb: not walked: no preset instance table for this flavor\n"),
        "{m}"
    );
    // a whole database skipped has no flavor
    let m = md(&with(|p| {
        p.families = vec![FamilySection::Skipped {
            node: "db".into(),
            flavor: None,
            reason: "it is kept".into(),
        }];
    }));
    assert!(m.contains("- db: not walked: it is kept\n"), "{m}");
    let none = md(&with(|p| p.families.clear()));
    assert!(
        none.contains("The design has no database to change."),
        "{none}"
    );
}

#[test]
fn a_family_that_already_holds_does_not_call_itself_the_design_in_force() {
    let m = md(&with(|p| {
        p.families = vec![FamilySection::Answer(FamilyAnswer {
            node: "db".into(),
            flavor: "dynamodb".into(),
            plan: Box::new(walked(
                PlanVerdict::AlreadyHolds,
                Vec::new(),
                fx(300.0, 0.0, 251.0),
                700_000,
            )),
            notes: Vec::new(),
        })];
    }));
    let fam = m
        .split("## If you changed")
        .nth(1)
        .unwrap()
        .split("## Verdict")
        .next()
        .unwrap();
    assert!(
        fam.contains("  - The changed design already holds the target: no step needed."),
        "{fam}"
    );
    assert!(!fam.contains("in force"), "{fam}");
}

#[test]
fn the_fixtures_family_plan_is_self_consistent() {
    let FamilySection::Answer(a) = &doc().families[0] else {
        panic!("an answer");
    };
    assert_eq!(
        a.plan.steps[0].before, a.plan.start,
        "its step starts at its start"
    );
    assert_eq!(a.plan.steps[0].after, a.plan.end);
}

#[test]
fn a_family_answer_over_budget_says_what_its_own_walk_found() {
    let m = |holds: bool| {
        md(&with(|p| {
            p.families = vec![FamilySection::Answer(FamilyAnswer {
                node: "db".into(),
                flavor: "dynamodb".into(),
                plan: Box::new(walked(
                    PlanVerdict::OverBudget {
                        over_by_micro_usd_hr: 10_000,
                        holds,
                    },
                    Vec::new(),
                    fx(300.0, 0.0, 251.0),
                    900_000,
                )),
                notes: Vec::new(),
            })];
        }))
    };
    assert!(m(true).contains("The plan holds the target but is over budget by $0.01/h"));
    assert!(m(false).contains("does not hold the target; the next step needed would cost $2.01/h, over the budget of $2.00/h by $0.01/h"));
    assert!(
        !m(false).contains("(within the budget"),
        "never claims the shown plan is within"
    );
}

/// True when `e` is clear of `l` with the margin a plan needs: p99 at most 0.9 of the limit and
/// errors at most half of it (the rule of `Limit::passes_with_margin`, which takes a whole point).
pub(super) fn holds_with_margin(l: &Limit, e: &Effect) -> bool {
    e.p99_ms <= 0.9 * l.p99.as_secs_f64() * 1000.0 && e.error_rate <= 0.5 * l.error_rate
}

#[test]
fn the_margin_rule_agrees_with_the_sweeps() {
    let l = Limit::default();
    let point = |p99_ms: f64, error_rate: f64| Point {
        k: 1.0,
        req_s: 1.0,
        requests: 1,
        goodput_req_s: 1.0,
        error_rate,
        p50_ms: 0.0,
        p95_ms: 0.0,
        p99_ms,
        cause: None,
        nodes: Vec::new(),
        over: Over::default(),
    };
    for (p99, errors) in [
        (900.0, 0.005),
        (900.1, 0.005),
        (900.0, 0.005_000_1),
        (0.0, 0.0),
        (1000.0, 0.01),
    ] {
        assert_eq!(
            holds_with_margin(&l, &fx(p99, errors, 1.0)),
            l.passes_with_margin(&point(p99, errors)),
            "{p99} {errors}"
        );
    }
}

#[test]
fn a_design_that_does_not_hold_says_so_and_why_its_cost_is_low() {
    let m = md(&with(|p| {
        p.verify.holds = false;
        p.verify.cost_note = Some(SATURATED_COST);
    }));
    assert!(
        m.contains("It does not hold the target with the margin."),
        "{m}"
    );
    assert!(
        m.contains("per-use prices (counted at its visits) are understated"),
        "{m}"
    );
    assert!(md(&rec()).contains("It holds the target with the margin."));
}

#[test]
fn the_knee_is_a_search_result_on_the_plain_limit_not_the_margin() {
    let m = md(&rec());
    assert!(
        m.contains("finds its knee near 262 req/s (2.62x the profile's rates), to the sweep's search resolution: the last point within the plain limit (no margin) before the first one over it."),
        "{m}"
    );
}

#[test]
fn the_measured_knee_and_a_sweep_that_did_not_run_are_both_reported() {
    let k = |v: Option<Verdict>, e: Option<&str>| {
        md(&with(|p| {
            p.measured_knee = v;
            p.sweep_error = e.map(String::from);
        }))
    };
    assert!(k(
        Some(Verdict::HoldsTo { k: 5.0, req_s: 500.0 }),
        None
    )
    .contains("no point over the limit up to 500 req/s (5.00x the profile's rates), the highest it ran"));
    assert!(
        k(
            Some(Verdict::BrokenAtLowest {
                k: 0.5,
                req_s: 50.0
            }),
            None
        )
        .contains("over the limit already at its lowest point, 50 req/s (0.50x")
    );
    let failed = k(None, Some("a point is too large."));
    assert!(
        failed
            .contains("The load sweep of the proposed design did not run: a point is too large.\n"),
        "{failed}"
    );
}

#[test]
fn a_plan_without_steps_says_so() {
    let m = md(&with(|p| {
        p.verdict = PlanVerdict::AlreadyHolds;
        p.steps.clear();
        p.end = p.start.clone();
        p.price_after = p.price_before;
    }));
    assert!(m.contains("No steps.\n"), "{m}");
    assert!(m.contains("No step, so no menu."), "{m}");
    assert!(!m.contains("| step |"), "{m}");
}

#[test]
fn the_menu_marks_the_move_taken() {
    let m = md(&rec());
    assert_eq!(m.matches("(taken)").count(), 2, "{m}");
    assert!(
        m.contains("- instance class db.m5.large to db.m5.xlarge: $1.13/h;"),
        "{m}"
    );
    assert!(
        m.contains("- instance class db.m5.large to db.m5.2xlarge (taken): $1.42/h;"),
        "{m}"
    );
}

#[test]
fn a_priceless_step_and_an_unknown_capacity_read_plainly() {
    let m = md(&with(|p| {
        p.steps[0].price_micro_usd_hr = None;
        p.steps[0].price_delta_micro_usd_hr = None;
        p.steps[0].before.capacity_req_s = None;
        p.steps[0].menu[0].price_micro_usd_hr = None;
        p.price_before = None;
        p.start.capacity_req_s = None;
    }));
    assert!(
        m.contains(
            "| db | instance class db.m5.large to db.m5.2xlarge | not priced | no estimate to 118 |"
        ),
        "{m}"
    );
    assert!(
        m.contains("capacity est. no estimate. It costs not priced (priced nodes only)"),
        "{m}"
    );
    assert!(m.contains("db.m5.xlarge: not priced;"), "{m}");
}

#[test]
fn money_is_exact_and_never_a_float() {
    assert_eq!(dollars(0), "0.00");
    assert_eq!(dollars(840_000), "0.84");
    assert_eq!(dollars(3_500), "0.0035");
    assert_eq!(dollars(1_234_567), "1.234567");
    assert_eq!(dollars(2_000_000), "2.00");
    assert_eq!(dollars(u64::MAX), "18446744073709.551615");
    assert_eq!(price(Some(840_000)), "$0.84/h");
    assert_eq!(price(None), "not priced");
    // a saving is a minus
    let m = md(&with(|p| {
        p.steps[1].price_delta_micro_usd_hr = Some(-100_000);
    }));
    assert!(m.contains("-$0.10/h; $1.62/h in all"), "{m}");
}

#[test]
fn memory_and_cores_moves_are_worded() {
    assert_eq!(bytes(2 << 30), "2 GiB");
    assert_eq!(bytes(512 << 20), "512 MiB");
    assert_eq!(bytes(1000), "1000 bytes");
    assert_eq!(
        change(&mv(
            "x",
            MoveKind::Memory {
                from: 1 << 30,
                to: 2 << 30
            }
        )),
        "memory limit 1 GiB to 2 GiB"
    );
    assert_eq!(
        change(&mv("x", MoveKind::Cores { from: 2.0, to: 4.0 })),
        "cores 2 to 4"
    );
}

/// The budget line says what the plan shown costs with per-use prices, within or over, and says so
/// when those prices turned a holding plan into an over-budget one.
#[test]
fn the_budget_check_says_what_the_plan_shown_costs_with_per_use_prices() {
    let m = |cost: u64, rejudged: bool| {
        md(&with(|p| {
            p.budget_check = Some(BudgetCheck {
                budget_micro_usd_hr: 2_000_000,
                cost_micro_usd_hr: cost,
                rejudged,
            });
        }))
    };
    assert!(m(1_700_000, false).contains("Budget check, with per-use prices counted at the target run's visits: the plan shown costs $1.70/h, within the budget of $2.00/h.\n"));
    // the design in force already over the budget: no claim of "within" anywhere
    let over = m(2_300_000, false);
    assert!(
        over.contains("the plan shown costs $2.30/h, over the budget of $2.00/h by $0.30/h.\n"),
        "{over}"
    );
    let re = m(2_300_000, true);
    assert!(re.contains("by $0.30/h. The walk prices without per-use prices and found the plan within the budget; with them it is over, so the verdict is over budget.\n"), "{re}");
    let none = md(&with(|p| p.budget_check = None));
    assert!(!none.contains("Budget check"), "no budget, no line");
}

#[test]
fn a_family_walk_at_its_step_cap_names_its_own_cap() {
    let r = md(&with(|p| {
        p.families = vec![FamilySection::Answer(FamilyAnswer {
            node: "db".into(),
            flavor: "dynamodb".into(),
            plan: Box::new(walked(
                PlanVerdict::StepCap,
                Vec::new(),
                fx(3000.0, 0.0, 51.0),
                900_000,
            )),
            notes: Vec::new(),
        })];
    }));
    assert!(r.contains("stops after 10 steps, the step cap"), "{r}");
}
