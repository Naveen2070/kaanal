//! The cheapest-plan part of the plan report (Step 9c-2): the golden file, the comparison and its
//! winner, no winner when nothing holds, the words for each stop reason, a skipped provider, the
//! unchanged plain report, the JSON, and the hostile and tiny-money cases. The fixtures are in
//! tests.rs.

use kaanal_core::cost::CostReport;
use kaanal_core::resize::MoveKind;
use kaanal_sim::plan::{CHEAPEST_NOTE, Effect, Plan, PlanVerdict, SearchInfo, Step, StopReason};

use super::tests::{block, class, doc, fx, md, mv, rec, verify, walked};
use super::*;
use crate::sim::plan_draft::{BudgetCheck, SATURATED_COST};
use crate::sim::plan_search::{CheapestAsked, PROVIDER_NOTE, ProviderSection};

const GOLDEN: &str = include_str!("../../../../tests/golden/plan-report-cheapest.md");
const PLAIN: &str = include_str!("../../../../tests/golden/plan-report.md");

fn info(stop: StopReason, runs: u32, elapsed_ms: u64, limit_ms: u64) -> SearchInfo {
    SearchInfo {
        stop,
        runs,
        elapsed_ms,
        limit_ms,
        run_cap: 200,
        design_in_force_holds: true,
        notes: Vec::new(),
    }
}

/// A merged change the search took, with its own price effect and the note the search writes.
fn taken(node: &str, kind: MoveKind, delta: i64, note: &str, after: Effect) -> Step {
    Step {
        mv: mv(node, kind),
        price_micro_usd_hr: None,
        price_delta_micro_usd_hr: Some(delta),
        before: fx(300.0, 0.0, 251.0),
        after,
        menu: Vec::new(),
        note: Some(note.into()),
    }
}

fn cost(total: u64) -> CostReport {
    CostReport {
        total_micro_usd_hr: total,
        ..verify(true).cost
    }
}

/// The GCP what-if: the design translated, a copy count taken off, within the budget.
fn gcp(price_after: u64, verdict: &PlanVerdict) -> ProviderSection {
    let step = taken(
        "app",
        MoveKind::Copies { from: 5, to: 3 },
        -600_000,
        "cheaper by $0.60 an hour and still holds",
        fx(540.0, 0.001, 250.0),
    );
    ProviderSection::Answer {
        provider: "gcp".into(),
        plan: Box::new(Plan {
            start: fx(300.0, 0.0, 251.0),
            price_before: Some(1_480_000),
            ..walked(
                verdict.clone(),
                vec![step],
                fx(540.0, 0.001, 250.0),
                price_after,
            )
        }),
        cost: cost(price_after + 12_000),
        cost_note: None,
        budget_check: Some(BudgetCheck {
            budget_micro_usd_hr: 2_000_000,
            cost_micro_usd_hr: price_after + 12_000,
            rejudged: false,
        }),
        // a design that holds was found with the run cap; one that does not, by running out
        info: if *verdict == PlanVerdict::Holds {
            info(StopReason::RunCap, 200, 22_400, 37_500)
        } else {
            info(StopReason::Exhausted, 31, 9_800, 37_500)
        },
        notes: vec![
            "app: aws/m5.large to gcp/n2-standard-2".into(),
            "nginx: kept as written, not compared (no class)".into(),
            PROVIDER_NOTE.into(),
            CHEAPEST_NOTE.into(),
        ],
    }
}

fn azure_skipped() -> ProviderSection {
    ProviderSection::Skipped {
        provider: "azure".into(),
        reason: "db: no managed postgres class for azure in the price table".into(),
    }
}

/// A cheapest plan: the design in force holds and the search took three things off it, one for
/// less than a cent an hour; GCP is cheaper still; Azure was skipped.
fn cheapest_doc() -> PlanDoc {
    let mut p = doc();
    p.request.cheapest = Some(CheapestAsked {
        time_limit_s: 150,
        run_cap: 200,
        providers: vec!["gcp".into(), "azure".into()],
    });
    p.verdict = PlanVerdict::Holds;
    p.steps = vec![
        taken(
            "app",
            MoveKind::Copies { from: 5, to: 3 },
            -400_000,
            "cheaper by $0.40 an hour and still holds",
            fx(520.0, 0.001, 255.0),
        ),
        taken(
            "db",
            class("db.m5.2xlarge", "db.m5.xlarge"),
            -290_000,
            "cheaper by $0.29 an hour and still holds",
            fx(520.0, 0.001, 255.0),
        ),
        taken(
            "worker",
            class("c5.xlarge", "c5.large"),
            -3_500,
            "cheaper by $0.0035 an hour and still holds",
            fx(520.0, 0.001, 255.0),
        ),
    ];
    p.start = fx(300.0, 0.0, 251.0);
    p.end = fx(520.0, 0.001, 255.0);
    p.price_before = Some(1_620_000);
    p.price_after = Some(926_500);
    p.families = Vec::new();
    // the re-check of the proposed draft is the search's end, exactly
    p.verify.p99_ms = p.end.p99_ms;
    p.verify.error_rate = p.end.error_rate;
    p.verify.capacity_req_s = p.end.capacity_req_s;
    p.verify.cost.total_micro_usd_hr = 940_000;
    p.budget_check = Some(BudgetCheck {
        budget_micro_usd_hr: 2_000_000,
        cost_micro_usd_hr: 940_000,
        rejudged: false,
    });
    p.cheapest_note = Some(CHEAPEST_NOTE);
    p.search = Some(info(StopReason::Exhausted, 64, 31_800, 75_000));
    p.providers = Some(vec![gcp(880_000, &PlanVerdict::Holds), azure_skipped()]);
    p
}

fn crec(f: impl FnOnce(&mut PlanDoc)) -> RunRecord {
    let mut r = rec();
    let mut p = cheapest_doc();
    f(&mut p);
    r.plan = Some(p);
    r
}

/// The answer of the first provider section.
fn first(p: &mut PlanDoc) -> &mut ProviderSection {
    &mut p.providers.as_mut().expect("sections")[0]
}

#[test]
fn the_cheapest_report_matches_the_golden_file() {
    assert_eq!(md(&crec(|_| {})), GOLDEN);
}

#[test]
fn a_plain_plan_report_is_unchanged() {
    let m = md(&rec());
    assert_eq!(m, PLAIN);
    for gone in [
        "## Search",
        "## Comparison",
        "If it ran on",
        "--cheapest",
        CHEAPEST_NOTE,
    ] {
        assert!(!m.contains(gone), "{gone}");
    }
}

fn rank(m: &str, name: &str) -> String {
    m.lines()
        .find(|l| l.starts_with('|') && l.contains(name))
        .unwrap()
        .split('|')
        .nth(1)
        .unwrap()
        .trim()
        .to_string()
}

#[test]
fn the_winner_is_the_cheapest_design_that_holds() {
    let m = md(&crec(|_| {}));
    assert!(
        m.contains("Winner: the design translated to GCP, $0.892/h. It is the cheapest design that holds the target found within the limit, not proven cheapest."),
        "{m}"
    );
    assert_eq!(rank(&m, "translated to GCP"), "1");
    assert_eq!(rank(&m, "design in force"), "2");
    // dearer on GCP: the design in force wins
    let m = md(&crec(|p| {
        p.providers = Some(vec![gcp(1_300_000, &PlanVerdict::Holds)]);
    }));
    assert!(
        m.contains("Winner: the design in force, with the changes above, $0.94/h."),
        "{m}"
    );
    // a design that does not hold ranks after every holder, however cheap
    let m = md(&crec(|p| {
        p.providers = Some(vec![gcp(
            100_000,
            &PlanVerdict::CannotHold {
                node: Some("db".into()),
                reason: "its class is the largest of the series".into(),
            },
        )]);
    }));
    assert!(m.contains("Winner: the design in force"), "{m}");
    assert_eq!(rank(&m, "translated to GCP"), "-", "{m}");
    assert!(m.contains("cannot hold by sizing, at 'db'"), "{m}");
    // equal prices are said to be tied
    let m = md(&crec(|p| {
        p.providers = Some(vec![gcp(928_000, &PlanVerdict::Holds)]);
    }));
    assert!(
        m.contains("(tied with the design translated to GCP)"),
        "{m}"
    );
}

/// Review Focus 5: no design holds at any size tried, so no winner, and the sections say so.
#[test]
fn no_holding_design_means_no_winner() {
    let cannot = |node: &str| PlanVerdict::CannotHold {
        node: Some(node.into()),
        reason: "its class is the largest of the series".into(),
    };
    let m = md(&crec(|p| {
        p.verdict = cannot("db");
        p.steps.clear();
        p.search = Some(SearchInfo {
            design_in_force_holds: false,
            ..info(StopReason::Exhausted, 12, 5_000, 75_000)
        });
        p.providers = Some(vec![gcp(1_700_000, &cannot("db")), azure_skipped()]);
    }));
    assert!(
        m.contains("No design holds at any size tried, so there is no winner."),
        "{m}"
    );
    assert!(!m.contains("Winner"), "{m}");
    assert!(!m.contains("could cost less"), "{m}");
    for l in m.lines().filter(|l| l.contains("| no |")) {
        assert!(
            l.starts_with("| - |"),
            "no rank for a design that does not hold: {l}"
        );
    }
    let section = &m[m.find("## If it ran on GCP").unwrap()..];
    assert!(section.contains(CHEAPEST_NOTE), "{section}");
    assert!(
        section.contains("The plan cannot hold the target by sizing at 'db'"),
        "{section}"
    );
    // nothing says a prediction or a proven minimum
    for claim in ["will cost", "guarantee", "is the minimum", "proven minimum"] {
        assert!(!m.contains(claim), "{claim}: {m}");
    }
}

/// A limit cut says "none found within the limit", never "cannot hold" (a cut walk's reason is the
/// limit's), and each stop has its own words, runs, elapsed time and limit.
#[test]
fn every_stop_reason_has_honest_words() {
    let cannot = PlanVerdict::CannotHold {
        node: None,
        reason: "search limit reached".into(),
    };
    for (stop, what, why) in [
        (StopReason::Exhausted, "", "it ran out of candidates to try"),
        (
            StopReason::RunCap,
            "run cap",
            "it reached the run cap, so cheaper designs may be left untried",
        ),
        (
            StopReason::TimeLimit,
            "time limit",
            "its time limit passed, so cheaper designs may be left untried",
        ),
    ] {
        let m = md(&crec(|p| {
            p.verdict = cannot.clone();
            p.search = Some(SearchInfo {
                design_in_force_holds: false,
                ..info(stop, 64, 31_800, 75_000)
            });
        }));
        let search = &m[m.find("## Search\n\n").unwrap()..];
        let search = search.split("\n\n").nth(1).unwrap();
        assert!(
            search.contains(&format!("- Stopped because {why}.")),
            "{search}"
        );
        assert!(search.contains("- Runs: 64 of 200 allowed."), "{search}");
        assert!(
            search.contains(
                "- Time: 31s 800ms used of 1m 15s, this search's share of the 150 s time limit"
            ),
            "{search}"
        );
        let v = m[m.find("## Verdict\n\n").unwrap()..]
            .split("\n\n")
            .nth(1)
            .unwrap();
        if what.is_empty() {
            assert!(
                v.starts_with("The plan cannot hold the target by sizing"),
                "{v}"
            );
        } else {
            assert!(
                v.starts_with("No design that holds the target was found within the limit")
                    && v.contains(what)
                    && v.contains("does not show that none exists"),
                "{v}"
            );
            assert!(!v.contains("cannot hold"), "{v}");
        }
    }
    // the design in force holds and nothing cheaper was found: the cut says one may exist
    let m = md(&crec(|p| {
        p.verdict = PlanVerdict::AlreadyHolds;
        p.steps.clear();
        p.price_after = p.price_before;
        p.search = Some(info(StopReason::TimeLimit, 9, 75_100, 75_000));
    }));
    assert!(
        m.contains("No cheaper design that holds was found within the limit: the search stopped at the time limit before it ran out of candidates, so one may exist."),
        "{m}"
    );
    assert!(m.contains("No changes."), "{m}");
    assert!(!m.contains("could cost less"), "{m}");
}

#[test]
fn a_skipped_provider_says_why() {
    let m = md(&crec(|_| {}));
    assert!(
        m.contains(
            "Not compared:\n\n- Azure: db: no managed postgres class for azure in the price table\n"
        ),
        "{m}"
    );
    assert!(!m.contains("## If it ran on Azure"), "no section for it");
    // prominent: under the heading, before the table and the winner
    let cmp = &m[m.find("## Comparison").unwrap()..];
    let (skip, table) = (
        cmp.find("Not compared:").unwrap(),
        cmp.find("| rank |").unwrap(),
    );
    assert!(skip < table, "{cmp}");
    // only skipped providers: still a comparison, the design in force alone
    let m = md(&crec(|p| p.providers = Some(vec![azure_skipped()])));
    assert!(
        m.contains("No provider was compared, so no winner is named."),
        "{m}"
    );
    assert!(!m.contains("Winner:"), "{m}");
    assert!(
        m.contains("- Azure: db: no managed postgres class for azure in the price table"),
        "{m}"
    );
}

#[test]
fn a_provider_section_carries_the_caveat_the_label_and_its_budget_verdict() {
    let m = md(&crec(|_| {}));
    let section = &m[m.find("## If it ran on GCP").unwrap()..m.find("## Re-run").unwrap()];
    assert!(
        section.contains(&format!("This what-if {PROVIDER_NOTE}.")),
        "{section}"
    );
    assert!(section.contains(&format!("*{CHEAPEST_NOTE}*")), "{section}");
    assert!(
        section.contains("within the budget of $2.00/h"),
        "{section}"
    );
    assert!(section.contains("The translation:"), "{section}");
    assert!(
        section.contains("- nginx: kept as written, not compared (no class)"),
        "{section}"
    );
    assert_eq!(
        section.matches(PROVIDER_NOTE).count(),
        1,
        "said once: {section}"
    );
    // over budget with per-use prices, and a saturated design's note, are said in the section
    let m = md(&crec(|p| {
        let ProviderSection::Answer {
            budget_check,
            cost_note,
            ..
        } = first(p)
        else {
            unreachable!()
        };
        *budget_check = Some(BudgetCheck {
            budget_micro_usd_hr: 800_000,
            cost_micro_usd_hr: 892_000,
            rejudged: true,
        });
        *cost_note = Some(SATURATED_COST);
    }));
    assert!(m.contains("over the budget of $0.80/h by $0.092/h"), "{m}");
    assert!(
        m.contains("Note: the design does not hold the target"),
        "{m}"
    );
    // no budget asked: the per-use cost is stated instead
    let m = md(&crec(|p| {
        let ProviderSection::Answer { budget_check, .. } = first(p) else {
            unreachable!()
        };
        *budget_check = None;
    }));
    assert!(
        m.contains("the design costs $0.892/h (3 nodes priced, 1 not)."),
        "{m}"
    );
}

/// A saving under a cent is printed as it is, never as $0.00, in the table and in the note.
#[test]
fn a_tiny_saving_is_not_printed_as_zero() {
    let m = md(&crec(|_| {}));
    assert!(
        m.contains("| -$0.0035/h | cheaper by $0.0035 an hour and still holds |"),
        "{m}"
    );
    assert!(!m.contains("$0.00 ") && !m.contains("$0.00/h"), "{m}");
}

#[test]
fn the_design_in_force_could_cost_less_says_both_prices() {
    let m = md(&crec(|_| {}));
    assert!(
        m.contains("The design in force already holds the target and could cost less: $1.62/h now, $0.9265/h with the changes below, $0.6935/h less."),
        "{m}"
    );
    // a design in force that did not hold is not "could cost less"
    let m = md(&crec(|p| {
        p.search = Some(SearchInfo {
            design_in_force_holds: false,
            ..info(StopReason::Exhausted, 64, 31_800, 75_000)
        });
    }));
    assert!(!m.contains("could cost less"), "{m}");
}

#[test]
fn a_collapsed_plan_does_not_narrate_a_price_sequence() {
    let m = md(&crec(|_| {}));
    let plan = &m[m.find("## Plan\n").unwrap()..m.find("## Checked").unwrap()];
    assert!(!plan.contains("in all"), "{plan}");
    assert!(
        !plan.contains(" to 251"),
        "no before-to-after columns: {plan}"
    );
    assert!(
        plan.contains("The design in force before any change: p99 300 ms"),
        "{plan}"
    );
    assert!(plan.contains("After the changes: p99 520 ms"), "{plan}");
    assert!(
        !plan.contains("## Menu"),
        "collapsed steps have no menu: {plan}"
    );
}

#[test]
fn the_json_block_parses_back_to_the_served_json() {
    let rep = plan_report(&crec(|_| {}), "9.9.9").unwrap();
    let text = block(&rep.markdown, "```json\n");
    assert_eq!(text, serde_json::to_string_pretty(&rep.json).unwrap());
    let parsed: serde_json::Value = serde_json::from_str(text).unwrap();
    assert_eq!(parsed, rep.json);
    let p = &rep.json["plan"];
    assert_eq!(p["cheapest_note"], CHEAPEST_NOTE);
    assert_eq!(p["search"]["stop"], "exhausted");
    assert_eq!(p["search"]["runs"], 64);
    assert_eq!(p["providers"][0]["kind"], "answer");
    assert_eq!(p["providers"][0]["info"]["stop"], "run_cap");
    assert_eq!(p["providers"][1]["kind"], "skipped");
    assert_eq!(
        p["request"]["cheapest"],
        serde_json::json!({ "time_limit_s": 150, "run_cap": 200, "providers": ["gcp", "azure"] })
    );
    assert_eq!(
        rep.json["recipe"]["command"],
        "kaanal sim plan --profile=peak --target=2.5x --p99 1s --errors 1% --budget=2.00 --keep=cache --max-steps 30 --cheapest --time=150s --run-cap=200 --providers=gcp,azure --seed 7"
    );
}

#[test]
fn the_recipe_names_none_when_no_section_ran() {
    let r = crec(|p| {
        p.request.cheapest = Some(CheapestAsked {
            time_limit_s: 60,
            run_cap: 15,
            providers: Vec::new(),
        });
        p.providers = Some(Vec::new());
    });
    let rep = plan_report(&r, "9.9.9").unwrap();
    let command = rep.json["recipe"]["command"].as_str().unwrap();
    assert!(
        command.contains("--cheapest --time=60s --run-cap=15 --providers=none --seed 7"),
        "{command}"
    );
    let m = &rep.markdown;
    assert!(
        !m.contains("## Comparison") && !m.contains("If it ran on"),
        "{m}"
    );
    assert!(
        m.contains(", the time limit."),
        "the whole limit is this search's: {m}"
    );
}

#[test]
fn hostile_names_keep_the_tables_whole() {
    let evil = "a|b\n```x";
    let m = md(&crec(|p| {
        p.steps[0].mv.node = evil.into();
        p.search
            .as_mut()
            .unwrap()
            .notes
            .push(format!("{evil}: refused"));
        let ProviderSection::Answer { plan, notes, .. } = first(p) else {
            unreachable!()
        };
        plan.steps[0].mv.node = evil.into();
        notes.push(format!("{evil}: kept as written"));
    }));
    // the report's text, not the data block (which holds the names as JSON)
    let m = &m[..m.find("## Data").unwrap()];
    assert!(!m.contains("a|b"), "{m}");
    assert_eq!(
        m.matches("```").count(),
        2,
        "only the re-run block's fences: {m}"
    );
    for row in m
        .lines()
        .filter(|l| l.starts_with("| 1 |") && l.contains("copies 5 to 3"))
    {
        assert_eq!(
            row.matches('|').count() - row.matches("\\|").count(),
            6,
            "{row}"
        );
    }
}

/// Review: the fixture's re-check is the search's end, and the report says one thing about it in
/// both places (the Plan block and the Checked block).
#[test]
fn the_plan_and_the_checked_block_give_the_same_figures() {
    let p = cheapest_doc();
    assert_eq!(
        crate::sim::plan_draft::round_trip(&p.end, &p.verify),
        Ok(())
    );
    let m = md(&crec(|_| {}));
    let figures = "p99 520 ms, errors 0.1%, capacity est. 255 req/s";
    assert!(m.contains(&format!("After the changes: {figures};")), "{m}");
    assert!(
        m.contains(&format!("checked again at the target: {figures}.")),
        "{m}"
    );
}

/// Review: one price per design. The comparison ranks on the cost with per-use prices, the figure
/// the section states, and counts the nodes left out.
#[test]
fn the_comparison_and_the_section_state_the_same_price() {
    let m = md(&crec(|_| {}));
    let row = m
        .lines()
        .find(|l| l.contains("translated to GCP |"))
        .unwrap();
    assert!(row.contains("| $0.892/h | 1 |"), "{row}");
    let main = m
        .lines()
        .find(|l| l.contains("| the design in force, with"))
        .unwrap();
    assert!(main.contains("| $0.94/h | 1 |"), "{main}");
    let section = &m[m.find("## If it ran on GCP").unwrap()..];
    assert!(
        section.contains("the plan shown costs $0.892/h"),
        "{section}"
    );
    assert!(!row.contains("$0.88/h"), "{row}");
}

/// Review: a design that is cheaper only because it leaves nodes out of its price does not win.
#[test]
fn designs_with_different_unpriced_nodes_are_not_ranked() {
    let m = md(&crec(|p| {
        let ProviderSection::Answer { cost, .. } = first(p) else {
            unreachable!()
        };
        cost.unpriced = 0;
        cost.priced = 4;
    }));
    assert!(!m.contains("Winner"), "{m}");
    assert!(
        m.contains("leave different nodes out of their prices, so the prices cannot be ranked against each other and no winner is named."),
        "{m}"
    );
    assert_eq!(rank(&m, "translated to GCP"), "not comparable", "{m}");
    assert_eq!(rank(&m, "design in force"), "not comparable", "{m}");
    // a design that does not hold is not part of the question
    let m = md(&crec(|p| {
        p.providers = Some(vec![gcp(
            100_000,
            &PlanVerdict::CannotHold {
                node: None,
                reason: "x".into(),
            },
        )]);
        let ProviderSection::Answer { cost, .. } = first(p) else {
            unreachable!()
        };
        cost.unpriced = 0;
    }));
    assert!(m.contains("Winner: the design in force"), "{m}");
}

/// Review minor: a winner over the budget says so.
#[test]
fn a_winner_over_the_budget_says_so() {
    let m = md(&crec(|p| {
        p.verdict = PlanVerdict::OverBudget {
            over_by_micro_usd_hr: 50_000,
            holds: true,
        };
        // dearer than the design in force, so the design in force wins
        p.providers = Some(vec![gcp(880_000, &PlanVerdict::Holds)]);
        let ProviderSection::Answer { cost, .. } = first(p) else {
            unreachable!()
        };
        cost.total_micro_usd_hr = 2_000_000;
    }));
    assert!(
        m.contains("not proven cheapest. It is over the budget by $0.05/h."),
        "{m}"
    );
}

/// Review minor: an over-budget design that does not hold, after a limit cut, is "none found within
/// the limit", not a claim about the budget.
#[test]
fn an_over_budget_design_that_does_not_hold_after_a_cut_says_none_found() {
    let m = md(&crec(|p| {
        p.verdict = PlanVerdict::OverBudget {
            over_by_micro_usd_hr: 50_000,
            holds: false,
        };
        p.search = Some(SearchInfo {
            design_in_force_holds: false,
            ..info(StopReason::RunCap, 200, 9_000, 75_000)
        });
    }));
    let v = m[m.find("## Verdict\n\n").unwrap()..]
        .split("\n\n")
        .nth(1)
        .unwrap();
    assert!(
        v.starts_with("No design that holds the target was found within the limit"),
        "{v}"
    );
    assert!(
        m.contains("| no | no holding design found within the limit |"),
        "{m}"
    );
    assert!(!m.contains("no holding design within the budget"), "{m}");
}
