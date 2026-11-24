//! Unit tests of [`super::walk_by`] with scripted runs, and of the ranking (split from `walk.rs`
//! to keep it under the size seam).

use std::collections::BTreeMap;

use kaanal_core::cost::estimate;

use super::*;
use crate::sweep::{Limit, NodePoint, Over, Point};

fn key(gain: f64, delta: Option<i64>, size: f64, total: u64, label: &str) -> Key {
    Key {
        gain,
        delta,
        size,
        total,
        label: label.into(),
    }
}

fn best(keys: &[Key]) -> &str {
    &keys.iter().min_by(|a, b| rank(a, b)).unwrap().label
}

#[test]
fn gain_per_dollar_prefers_the_cheaper_equal_gain() {
    // the same gain for half the price wins
    let k = [
        key(2.0, Some(20), 1.0, 120, "dear"),
        key(2.0, Some(10), 1.0, 110, "cheap"),
    ];
    assert_eq!(best(&k), "cheap");
    // more gain per dollar beats a cheaper step
    let k = [
        key(1.0, Some(10), 1.0, 110, "cheap"),
        key(3.0, Some(20), 1.0, 120, "worth"),
    ];
    assert_eq!(best(&k), "worth");
    // an equal ratio goes to the cheaper design, then the lower label
    let k = [
        key(2.0, Some(20), 1.0, 120, "a"),
        key(1.0, Some(10), 1.0, 110, "b"),
    ];
    assert_eq!(best(&k), "b");
    let k = [
        key(1.0, Some(10), 1.0, 110, "b"),
        key(1.0, Some(10), 1.0, 110, "a"),
    ];
    assert_eq!(best(&k), "a");
    // priced before unpriced, a gain before none, then the smallest unpriced step
    let k = [
        key(9.0, None, 1.0, 100, "unpriced"),
        key(0.1, Some(50), 1.0, 150, "priced"),
    ];
    assert_eq!(best(&k), "priced");
    let k = [
        key(0.0, Some(1), 1.0, 101, "flat"),
        key(0.5, None, 4.0, 100, "gains"),
    ];
    assert_eq!(best(&k), "gains");
    let k = [
        key(1.0, None, 4.0, 0, "four"),
        key(0.2, None, 1.0, 0, "one"),
    ];
    assert_eq!(best(&k), "one");
}

#[test]
fn the_margin_is_the_worse_of_the_two_limits() {
    let l = Limit::default(); // 1000 ms, 1 percent
    assert!((margin(&l, &pt(500.0, 0.02)) - 2.0).abs() < 1e-12);
    assert!((margin(&l, &pt(3000.0, 0.0)) - 3.0).abs() < 1e-12);
    assert!(margin(&l, &pt(f64::NAN, 0.0)).is_infinite());
    assert_eq!(gain(f64::INFINITY, f64::INFINITY), 0.0);
    assert_eq!(gain(1.0, 2.0), 0.0);
}

fn pt(p99_ms: f64, error_rate: f64) -> Point {
    Point {
        k: 1.0,
        req_s: 1.0,
        requests: 1000,
        goodput_req_s: 1.0,
        error_rate,
        p50_ms: 0.0,
        p95_ms: 0.0,
        p99_ms,
        cause: None,
        nodes: vec![NodePoint {
            name: "x".into(),
            utilization: Some(1.0),
            saturated: true,
            saturates_at_req_s: Some(100.0),
        }],
        over: Over::default(),
    }
}

fn fake(holds: bool) -> Check {
    Check {
        holds,
        point: pt(if holds { 10.0 } else { 5000.0 }, 0.0),
        capacity_req_s: Some(100.0),
        bottleneck: Some("x".into()),
    }
}

const SERVICE: &str = "version: 1\nnodes:\n  gw: { type: gateway }\n  x: { type: service, cpu: { cores: 1 } }\nedges:\n  - { from: gw, to: x }\n";

/// Copies only (`cpu` next to the instance) and priced.
const PRICED: &str = "version: 1\nnodes:\n  gw: { type: gateway }\n  x: { type: service, instance: aws/m5.large, cpu: { cores: 2 } }\nedges:\n  - { from: gw, to: x }\n";

/// Walks `yaml` with `req`; `script` answers the runs in order. Returns the plan and the steps sent.
fn walk_on(
    yaml: &str,
    req: &Request,
    script: Vec<Result<Check, PointFail>>,
) -> (Result<Plan, String>, Vec<Step>) {
    let cfg = Config::from_yaml(yaml).unwrap();
    let mut answers = script.into_iter();
    let mut sent = Vec::new();
    let p = walk_by(
        &cfg,
        req,
        &mut |_| answers.next().expect("a scripted answer for each run"),
        &mut |s| sent.push(s.clone()),
    );
    (p, sent)
}

/// [`walk_on`] on a service with `cpu`: two moves, cores then a copy.
fn walk_script(script: Vec<Result<Check, PointFail>>) -> Plan {
    walk_on(SERVICE, &Request::default(), script).0.unwrap()
}

/// A failing check at `p99_ms` (bottleneck `x`), as a scripted run answer.
#[expect(
    clippy::unnecessary_wraps,
    reason = "it is one entry of a script of results"
)]
fn at(p99_ms: f64) -> Result<Check, PointFail> {
    let mut c = fake(false);
    c.point.p99_ms = p99_ms;
    Ok(c)
}

#[test]
fn a_refused_candidate_is_dropped_with_a_note() {
    let too_large = || Err(PointFail::TooLarge("workload too large".into()));
    let p = walk_script(vec![Ok(fake(false)), too_large(), Ok(fake(true))]);
    assert_eq!(p.verdict, PlanVerdict::Holds);
    let s = &p.steps[0];
    assert!(matches!(s.mv.kind, MoveKind::Copies { from: 1, to: 2 }));
    assert_eq!(s.menu.len(), 1);
    let note = s.note.as_deref().unwrap();
    assert!(
        note.contains("'x' cores 1 -> 2 refused: too large to run"),
        "{note}"
    );

    // every candidate refused: the walk cannot hold, and says why
    let p = walk_script(vec![Ok(fake(false)), too_large(), too_large()]);
    match p.verdict {
        PlanVerdict::CannotHold { node, reason } => {
            assert_eq!(node.as_deref(), Some("x"));
            assert!(reason.contains("too large to run"), "{reason}");
        }
        v => panic!("{v:?}"),
    }
    assert_eq!(p.steps, []);
}

#[test]
fn a_holding_design_seen_in_a_menu_is_never_hidden() {
    // menu 1: cores holds, the copy does not but wins the tie (label); the walk takes the copy,
    // then its bottleneck has no move: the plan is the holding cores step, not a failure
    let mut stuck = fake(false);
    stuck.point.p99_ms = 2000.0;
    stuck.bottleneck = Some("not-a-node".into());
    let p = walk_script(vec![Ok(fake(false)), Ok(fake(true)), Ok(stuck)]);
    assert_eq!(p.verdict, PlanVerdict::Holds);
    assert_eq!(p.steps.len(), 1);
    assert!(
        matches!(p.steps[0].mv.kind, MoveKind::Cores { .. }),
        "{:?}",
        p.steps[0].mv
    );
    assert!(p.end.p99_ms < 100.0);
}

#[test]
fn the_stall_count_resets_after_a_step_that_gains() {
    // menu 1 gains nothing, menu 2 gains, menus 3 and 4 gain nothing: four steps walked, the
    // last two dropped; without the reset the walk would stop after menu 3 with one step kept
    let script = vec![
        at(5000.0),
        at(5000.0),
        at(5000.0),
        at(3000.0),
        at(3000.0),
        at(3000.0),
        at(3000.0),
        at(3000.0),
        at(3000.0),
    ];
    let (p, sent) = walk_on(SERVICE, &Request::default(), script);
    let p = p.unwrap();
    assert_eq!(p.verdict, PlanVerdict::NoProgress { node: "x".into() });
    assert_eq!(p.steps.len(), 2);
    assert_eq!(sent, p.steps, "the dropped steps are never sent");
    // the plan ends at the last step that gained: two copies, its design and figures
    assert_eq!(p.config.nodes["x"].replicas(), 3);
    assert!((p.end.p99_ms - 3000.0).abs() < 1e-9);
}

#[test]
fn a_no_progress_plan_drops_the_steps_that_gained_nothing() {
    let script = vec![at(5000.0), at(5000.0), at(5000.0), at(5000.0), at(5000.0)];
    let (p, sent) = walk_on(SERVICE, &Request::default(), script);
    let p = p.unwrap();
    assert_eq!(p.verdict, PlanVerdict::NoProgress { node: "x".into() });
    assert_eq!(p.steps, []);
    assert_eq!(sent, []);
    assert_eq!(p.config.nodes["x"].replicas(), 1);
    assert_eq!(p.end, p.start);
}

#[test]
fn no_saturated_node_cannot_hold_and_names_the_cause() {
    let mut c = fake(false);
    c.bottleneck = None;
    c.point.cause = Some("timeout");
    let p = walk_script(vec![Ok(c)]);
    match p.verdict {
        PlanVerdict::CannotHold { node, reason } => {
            assert_eq!(node, None, "no node to name");
            assert!(reason.contains("timeout"), "{reason}");
        }
        v => panic!("{v:?}"),
    }
}

#[test]
fn a_run_error_other_than_size_ends_the_walk_with_an_error() {
    let other = Err(PointFail::Other("engine broke".into()));
    let (p, _) = walk_on(SERVICE, &Request::default(), vec![Ok(fake(false)), other]);
    let e = p.unwrap_err();
    assert!(
        e.contains("'x' cores 1 -> 2") && e.contains("engine broke"),
        "{e}"
    );
}

#[test]
fn a_budget_stop_sends_the_plan_s_last_step() {
    let cfg = Config::from_yaml(PRICED).unwrap();
    let u = estimate(&cfg, &BTreeMap::new()).total_micro_usd_hr;
    let req = Request {
        budget_micro_usd_hr: Some(u),
        ..Request::default()
    };
    // the one copy holds but doubles the price: over by one unit
    let (p, sent) = walk_on(PRICED, &req, vec![Ok(fake(false)), Ok(fake(true))]);
    let p = p.unwrap();
    assert_eq!(
        p.verdict,
        PlanVerdict::OverBudget {
            over_by_micro_usd_hr: u,
            holds: true
        }
    );
    assert_eq!(p.steps.len(), 1);
    assert_eq!(sent, p.steps);
}

#[test]
fn a_budget_stop_with_no_holding_design_says_the_plan_does_not_hold() {
    let cfg = Config::from_yaml(PRICED).unwrap();
    let u = estimate(&cfg, &BTreeMap::new()).total_micro_usd_hr;
    let req = Request {
        budget_micro_usd_hr: Some(u),
        ..Request::default()
    };
    // the one copy would cost 2u and still not hold: the plan stays where it is, within the budget
    let (p, _) = walk_on(PRICED, &req, vec![Ok(fake(false)), Ok(fake(false))]);
    let p = p.unwrap();
    assert_eq!(
        p.verdict,
        PlanVerdict::OverBudget {
            over_by_micro_usd_hr: u,
            holds: false
        }
    );
    assert_eq!(p.steps.len(), 0);
    assert_eq!(
        p.price_after,
        Some(u),
        "the plan shown is within the budget"
    );
}

/// Menu 1: the copy gains a little, the cores move holds; the copy wins the tie on its label.
fn holding_cores_passed_over() -> Vec<Result<Check, PointFail>> {
    vec![at(5000.0), Ok(fake(true)), at(4000.0)]
}

#[test]
fn a_holding_design_beats_a_no_progress_end() {
    let mut script = holding_cores_passed_over();
    script.extend([at(4000.0), at(4000.0), at(4000.0), at(4000.0)]);
    let p = walk_script(script);
    assert_eq!(p.verdict, PlanVerdict::Holds);
    assert!(matches!(
        p.steps[..],
        [Step {
            mv: Move {
                kind: MoveKind::Cores { .. },
                ..
            },
            ..
        }]
    ));
}

#[test]
fn a_holding_design_beats_a_step_cap_end() {
    let req = Request {
        max_steps: 1,
        ..Request::default()
    };
    let (p, sent) = walk_on(SERVICE, &req, holding_cores_passed_over());
    let p = p.unwrap();
    assert_eq!(p.verdict, PlanVerdict::Holds);
    assert!(matches!(
        p.steps[..],
        [Step {
            mv: Move {
                kind: MoveKind::Cores { .. },
                ..
            },
            ..
        }]
    ));
    // the copy was sent as progress; the plan branches off before it and is the answer
    assert!(matches!(
        sent[..],
        [Step {
            mv: Move {
                kind: MoveKind::Copies { .. },
                ..
            },
            ..
        }]
    ));
}

#[test]
fn a_move_that_saves_ranks_by_its_saving_on_equal_gain() {
    let k = [
        key(1.0, Some(-5), 1.0, 95, "saves5"),
        key(1.0, Some(-20), 1.0, 80, "saves20"),
    ];
    assert_eq!(best(&k), "saves20");
    // a saving beats a cost whatever the ratio, and a larger gain beats a larger saving
    let k = [
        key(9.0, Some(1), 1.0, 101, "costs"),
        key(0.5, Some(-1), 1.0, 99, "saves"),
    ];
    assert_eq!(best(&k), "saves");
    let k = [
        key(1.0, Some(-20), 1.0, 80, "less"),
        key(2.0, Some(0), 1.0, 100, "more"),
    ];
    assert_eq!(best(&k), "more");
}

/// A check whose failed requests come mostly from `cause`, at an error rate over the plain limit.
#[expect(
    clippy::unnecessary_wraps,
    reason = "it is one entry of a script of results"
)]
fn failing_by(cause: &'static str, saturated: bool) -> Result<Check, PointFail> {
    let mut c = fake(false);
    c.point.error_rate = 0.2;
    c.point.cause = Some(cause);
    c.point.nodes[0].saturated = saturated;
    Ok(c)
}

#[test]
fn chaos_or_a_deadline_with_no_saturated_node_cannot_hold_without_a_step() {
    // the size knobs exist (cores, a copy), but chaos is not a size problem: no candidate runs
    let p = walk_script(vec![failing_by("chaos", true)]);
    match p.verdict {
        PlanVerdict::CannotHold { node, reason } => {
            assert_eq!(node, None);
            assert!(reason.contains("chaos"), "{reason}");
        }
        v => panic!("{v:?}"),
    }
    assert_eq!(p.steps, []);
    let p = walk_script(vec![failing_by("deadline", false)]);
    match p.verdict {
        PlanVerdict::CannotHold { node: None, reason } => {
            assert!(reason.contains("deadline"), "{reason}");
        }
        v => panic!("{v:?}"),
    }
    // a deadline with a saturated node is a size problem: the walk sizes it
    let p = walk_script(vec![
        failing_by("deadline", true),
        Ok(fake(true)),
        Ok(fake(true)),
    ]);
    assert_eq!(p.verdict, PlanVerdict::Holds);
}

#[test]
fn a_rate_limit_cause_names_the_node_that_has_one() {
    let yaml = "version: 1\nnodes:\n  gw: { type: gateway, rate_limit: { rps: 5, burst: 5 } }\n  x: { type: service, cpu: { cores: 1 } }\nedges:\n  - { from: gw, to: x }\n";
    let (p, _) = walk_on(
        yaml,
        &Request::default(),
        vec![failing_by("rate_limit", true)],
    );
    match p.unwrap().verdict {
        PlanVerdict::CannotHold { node, reason } => {
            assert_eq!(node.as_deref(), Some("gw"));
            assert!(reason.contains("rate limit you set"), "{reason}");
        }
        v => panic!("{v:?}"),
    }
}

#[test]
fn an_oom_cause_offers_memory_moves() {
    let yaml = "version: 1\nnodes:\n  gw: { type: gateway }\n  x: { type: service, memory: { limit: 512MiB, per_request: 1MiB } }\nedges:\n  - { from: gw, to: x }\n";
    let mut oom = fake(false);
    oom.point.cause = Some("oom");
    let (p, _) = walk_on(
        yaml,
        &Request::default(),
        vec![Ok(oom), Ok(fake(true)), Ok(fake(true))],
    );
    let p = p.unwrap();
    let menu: Vec<&MoveKind> = p.steps[0].menu.iter().map(|c| &c.mv.kind).collect();
    assert!(
        menu.iter().any(|k| matches!(
            k,
            MoveKind::Memory {
                from: 536_870_912,
                to: 1_073_741_824
            }
        )),
        "{menu:?}"
    );
}

/// A design in force already over the budget: a step that adds nothing to the price (here unpriced
/// moves) does not trip the budget, so the walk goes on and finds the holding design.
#[test]
fn a_step_that_adds_no_price_does_not_trip_the_budget() {
    let yaml = "version: 1\nnodes:\n  gw: { type: gateway }\n  y: { type: service, instance: aws/m5.large, cpu: { cores: 2 } }\n  x: { type: service, cpu: { cores: 1 } }\nedges:\n  - { from: gw, to: y }\n  - { from: y, to: x }\n";
    let cfg = Config::from_yaml(yaml).unwrap();
    assert!(price(&cfg).total_micro_usd_hr > 1, "y is priced");
    let r = Request {
        budget_micro_usd_hr: Some(1),
        ..Request::default()
    };
    let (p, _) = walk_on(
        yaml,
        &r,
        vec![
            Ok(fake(false)),
            at(4000.0),
            at(3000.0),
            Ok(fake(true)),
            Ok(fake(true)),
        ],
    );
    let p = p.unwrap();
    assert!(
        matches!(p.verdict, PlanVerdict::OverBudget { holds: true, .. }),
        "{:?}",
        p.verdict
    );
    assert_eq!(p.steps.len(), 2);
}
