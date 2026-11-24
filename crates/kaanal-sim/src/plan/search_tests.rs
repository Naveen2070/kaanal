//! Unit tests of [`super::search_by`] on scripted runs (a rule on the candidate topology) and an
//! injected clock, and of [`split_time`].

use std::cell::Cell;
use std::time::Duration;

use kaanal_core::resize::{Move, MoveKind};

use super::*;
use crate::plan::{CHEAPEST_NOTE, split_time};
use crate::sweep::{NodePoint, Over, Point};

/// A check that holds (p99 10 ms) or not (p99 5 s), `x` the bottleneck.
fn fake(holds: bool) -> Check {
    Check {
        holds,
        point: Point {
            k: 1.0,
            req_s: 1.0,
            requests: 1000,
            goodput_req_s: 1.0,
            error_rate: 0.0,
            p50_ms: 0.0,
            p95_ms: 0.0,
            p99_ms: if holds { 10.0 } else { 5000.0 },
            cause: None,
            nodes: vec![NodePoint {
                name: "x".into(),
                utilization: Some(1.0),
                saturated: true,
                saturates_at_req_s: Some(100.0),
            }],
            over: Over::default(),
        },
        capacity_req_s: Some(100.0),
        bottleneck: Some("x".into()),
    }
}

/// Node `name` of `t`: (copies, millicores; 1000 when none is written).
fn size(t: &Topology, name: &str) -> (u32, u32) {
    let n = &t.nodes[t.id(name).unwrap()];
    (u32::from(n.replicas), n.cpu.unwrap_or(1000))
}

/// Searches `yaml` with `holds` as the run; the clock advances `tick` per reading. Returns the
/// answer and the steps sent.
fn search_on(
    yaml: &str,
    req: &Request,
    opts: &SearchOptions,
    tick: Duration,
    holds: &dyn Fn(&Topology) -> bool,
) -> (Searched, Vec<Step>) {
    let cfg = Config::from_yaml(yaml).unwrap();
    let clock = Cell::new(Duration::ZERO);
    let mut sent = Vec::new();
    let s = search_by(
        &cfg,
        req,
        opts,
        &mut || {
            let t = clock.get();
            clock.set(t + tick);
            t
        },
        &mut |t| Ok(fake(holds(t))),
        &mut |s| sent.push(s.clone()),
    )
    .unwrap();
    (s, sent)
}

fn plain(yaml: &str, holds: &dyn Fn(&Topology) -> bool) -> (Searched, Vec<Step>) {
    search_on(
        yaml,
        &Request::default(),
        &SearchOptions::default(),
        Duration::ZERO,
        holds,
    )
}

/// One priced class, nothing written: class and copy moves both ways.
const PRICED: &str = "version: 1\nnodes:\n  gw: { type: gateway }\n  x: { type: service, instance: aws/m5.large }\nedges:\n  - { from: gw, to: x }\n";

/// A priced node with copies to spare.
const SPARE: &str = "version: 1\nnodes:\n  gw: { type: gateway }\n  x: { type: service, instance: aws/m5.large, replicas: 8 }\nedges:\n  - { from: gw, to: x }\n";

/// The same node with no class: it has no price, so the search leaves it alone (RULES 142 e).
const UNPRICED: &str = "version: 1\nnodes:\n  gw: { type: gateway }\n  x: { type: service, cpu: { cores: 4 }, replicas: 8 }\nedges:\n  - { from: gw, to: x }\n";

/// Holds with three copies or more, whatever the class.
fn three_copies(t: &Topology) -> bool {
    size(t, "x").0 >= 3
}

#[test]
fn a_greedy_overshoot_is_undone_by_the_search() {
    // 3 vCPU needed: the walk's one step from m5.large (2 vCPU) doubles the price (a class up or a
    // copy); a cheaper class with 4 vCPU (another series) holds too
    let need = |t: &Topology| {
        let (c, m) = size(t, "x");
        c * m >= 3000
    };
    let cfg = Config::from_yaml(PRICED).unwrap();
    let walked = walk_by(
        &cfg,
        &Request::default(),
        &mut |t| Ok(fake(need(t))),
        &mut |_| {},
    )
    .unwrap();
    assert_eq!(walked.verdict, PlanVerdict::Holds);
    let (s, _) = plain(PRICED, &need);
    assert_eq!(s.plan.verdict, PlanVerdict::Holds);
    assert!(
        s.plan.price_after.unwrap() < walked.price_after.unwrap(),
        "{:?} vs {:?}",
        s.plan.price_after,
        walked.price_after
    );
    // the walk's step first, then what the search took off
    assert_eq!(s.plan.steps[0], walked.steps[0]);
    assert!(s.plan.steps.len() > 1);
    assert_eq!(s.info.stop, StopReason::Exhausted);
    assert!(!s.info.design_in_force_holds);
}

#[test]
fn the_time_limit_returns_the_best_holding_design_found() {
    // one second a clock reading (read before each candidate run), a limit of three: the walk's
    // run, then three candidate runs (readings 0, 1 and 2 s); the fourth reading is at the limit
    let opts = SearchOptions {
        time_limit: Duration::from_secs(3),
        run_cap: 200,
    };
    let (s, _) = search_on(
        SPARE,
        &Request::default(),
        &opts,
        Duration::from_secs(1),
        &three_copies,
    );
    assert_eq!(s.info.stop, StopReason::TimeLimit);
    assert_eq!(s.info.runs, 4);
    assert_eq!(s.info.limit_ms, 3000);
    assert!(s.info.elapsed_ms >= 3000);
    assert_eq!(s.plan.verdict, PlanVerdict::Holds);
    assert!(s.plan.price_after < s.plan.price_before);

    // Review focus 4: a limit already passed, on a design the walk cannot make hold: the walk's
    // verdict, its steps only, nothing invented
    let opts = SearchOptions {
        time_limit: Duration::ZERO,
        run_cap: 200,
    };
    let unsizable = "version: 1\nnodes:\n  gw: { type: gateway }\n  app: { type: service }\n  x: { type: external-api }\nedges:\n  - { from: gw, to: app }\n  - { from: app, to: x }\n";
    let (s, sent) = search_on(
        unsizable,
        &Request::default(),
        &opts,
        Duration::from_secs(1),
        &|_| false,
    );
    assert!(
        matches!(s.plan.verdict, PlanVerdict::CannotHold { .. }),
        "{:?}",
        s.plan.verdict
    );
    assert_eq!(s.plan.steps, []);
    assert_eq!(sent, []);
    assert_eq!(s.info.stop, StopReason::Exhausted);
    assert_eq!(s.info.runs, 1);

    // and on a design that holds: the time is up before any candidate, so the design in force
    let (s, sent) = search_on(
        SPARE,
        &Request::default(),
        &opts,
        Duration::from_secs(1),
        &three_copies,
    );
    assert_eq!(s.info.stop, StopReason::TimeLimit);
    assert_eq!(s.info.runs, 1);
    assert_eq!(s.plan.verdict, PlanVerdict::AlreadyHolds);
    assert_eq!(s.plan.steps, []);
    assert_eq!(sent, []);
}

/// One core, one copy: the walk has two moves (cores, then a copy).
const SMALL: &str = "version: 1\nnodes:\n  gw: { type: gateway }\n  x: { type: service, cpu: { cores: 1 } }\nedges:\n  - { from: gw, to: x }\n";

#[test]
fn the_limits_bind_the_walk_too() {
    // a limit already passed, a design that does not hold: the check of the design in force runs,
    // every walk candidate after it is refused; the record names the limit
    let opts = SearchOptions {
        time_limit: Duration::ZERO,
        run_cap: 200,
    };
    let (s, sent) = search_on(
        SMALL,
        &Request::default(),
        &opts,
        Duration::from_secs(1),
        &|_| false,
    );
    assert_eq!(s.info.runs, 1);
    assert_eq!(s.info.stop, StopReason::TimeLimit);
    assert!(
        matches!(s.plan.verdict, PlanVerdict::CannotHold { .. }),
        "{:?}",
        s.plan.verdict
    );
    assert_eq!(s.plan.steps, []);
    assert_eq!(sent, []);

    // the run cap hit inside the walk: the cores candidate holds (run 2), the copy is refused at
    // the cap; the walk's holding candidate is the plan, runs stay at the cap
    let opts = SearchOptions {
        time_limit: Duration::from_secs(150),
        run_cap: 2,
    };
    let (s, _) = search_on(SMALL, &Request::default(), &opts, Duration::ZERO, &|t| {
        size(t, "x").1 >= 2000
    });
    assert_eq!(s.info.runs, 2);
    assert_eq!(s.info.stop, StopReason::RunCap);
    assert_eq!(s.plan.verdict, PlanVerdict::Holds);
    assert!(
        matches!(
            s.plan.steps[..],
            [Step {
                mv: Move {
                    kind: MoveKind::Cores { .. },
                    ..
                },
                ..
            }]
        ),
        "{:?}",
        s.plan.steps
    );
}

#[test]
fn the_run_cap_stops_the_search_and_counts_runs() {
    let opts = SearchOptions {
        time_limit: Duration::from_secs(150),
        run_cap: 3,
    };
    let (s, _) = search_on(
        SPARE,
        &Request::default(),
        &opts,
        Duration::ZERO,
        &three_copies,
    );
    assert_eq!(s.info.stop, StopReason::RunCap);
    assert_eq!(s.info.runs, 3);
    assert_eq!(s.info.run_cap, 3);
    assert_eq!(s.plan.verdict, PlanVerdict::Holds, "the best found holds");
    let d = SearchOptions::default();
    assert_eq!((d.time_limit, d.run_cap), (Duration::from_secs(150), 200));
}

#[test]
fn a_kept_node_is_never_shrunk() {
    // Review focus 2: x and y both have spare copies and cores; x is kept
    let yaml = "version: 1\nnodes:\n  gw: { type: gateway }\n  x: { type: service, instance: aws/m5.large, replicas: 8 }\n  y: { type: service, instance: aws/m5.large, replicas: 8 }\nedges:\n  - { from: gw, to: x }\n  - { from: x, to: y }\n";
    let req = Request {
        keep: vec!["x".into()],
        ..Request::default()
    };
    let seen_x = Cell::new(true);
    let first = std::cell::OnceCell::new();
    let holds = |t: &Topology| {
        // the first run is the design in force: every later one must size x the same
        if *first.get_or_init(|| size(t, "x")) != size(t, "x") {
            seen_x.set(false);
        }
        true
    };
    let (s, sent) = search_on(
        yaml,
        &req,
        &SearchOptions::default(),
        Duration::ZERO,
        &holds,
    );
    assert!(seen_x.get(), "no candidate changed x");
    assert_eq!(
        format!("{:?}", s.plan.config.nodes["x"]),
        format!("{:?}", Config::from_yaml(yaml).unwrap().nodes["x"])
    );
    assert!(s.plan.steps.iter().chain(&sent).all(|st| st.mv.node == "y"));
    assert_eq!(s.plan.config.nodes["y"].replicas(), 1, "y shrinks");
    assert!(s.info.design_in_force_holds);
}

#[test]
fn a_knob_that_moves_twice_collapses_into_one_step_with_the_summed_saving() {
    // x needs half of y's copies: pass 1 takes x to 4 (y is still 8) and y to 1, pass 2 takes x to 1
    let yaml = "version: 1\nnodes:\n  gw: { type: gateway }\n  x: { type: service, instance: aws/m5.large, replicas: 8 }\n  y: { type: service, instance: aws/m5.large, replicas: 8 }\nedges:\n  - { from: gw, to: x }\n  - { from: x, to: y }\n";
    let (s, sent) = plain(yaml, &|t| 2 * size(t, "x").0 >= size(t, "y").0);
    let copies = |node: &str| {
        s.plan
            .steps
            .iter()
            .filter(|st| st.mv.node == node && matches!(st.mv.kind, MoveKind::Copies { .. }))
            .collect::<Vec<_>>()
    };
    let x = copies("x");
    assert_eq!(x.len(), 1, "{:?}", s.plan.steps);
    assert_eq!(x[0].mv.kind, MoveKind::Copies { from: 8, to: 1 });
    assert_eq!(copies("y")[0].mv.kind, MoveKind::Copies { from: 8, to: 1 });
    // x moved twice, so one more move was sent than steps were kept
    assert_eq!(sent.len(), s.plan.steps.len() + 1, "{sent:?}");
    // the kept steps' savings add up to the whole change in price
    let sum: i64 = s
        .plan
        .steps
        .iter()
        .map(|st| st.price_delta_micro_usd_hr.unwrap())
        .sum();
    let (before, after) = (s.plan.price_before.unwrap(), s.plan.price_after.unwrap());
    assert_eq!(
        sum,
        i64::try_from(after).unwrap() - i64::try_from(before).unwrap()
    );
}

#[test]
fn an_unpriced_node_is_never_shrunk() {
    // RULES 142 e: copies and cores to spare, no price, so no run is spent on it
    let (s, sent) = plain(UNPRICED, &|_| true);
    assert_eq!(s.plan.steps, []);
    assert_eq!(sent, []);
    assert_eq!(s.info.runs, 1, "only the check of the design in force");
    assert_eq!(s.plan.config.nodes["x"].replicas(), 8);
}

#[test]
fn the_same_inputs_give_the_same_plan() {
    let a = plain(SPARE, &three_copies);
    let b = plain(SPARE, &three_copies);
    assert_eq!(
        serde_json::to_string(&a.0).unwrap(),
        serde_json::to_string(&b.0).unwrap()
    );
    assert_eq!(a.1, b.1);
}

#[test]
fn the_collapsed_steps_name_the_original_value_as_from() {
    let (s, sent) = plain(SPARE, &three_copies);
    // the class moves (any class holds), then copies 8 -> 3; the second pass takes nothing
    let kinds: Vec<&MoveKind> = s.plan.steps.iter().map(|s| &s.mv.kind).collect();
    assert!(
        matches!(
            kinds[..],
            [MoveKind::Class { .. }, MoveKind::Copies { from: 8, to: 3 }]
        ),
        "{kinds:?}"
    );
    assert_eq!(sent.len(), 2, "{sent:?}");
    // before of the first move, after of the last; no menu
    assert_eq!(s.plan.steps[0].before, sent[0].before);
    assert_eq!(s.plan.steps[0].after, sent[0].after);
    assert!(s.plan.steps.iter().all(|s| s.menu.is_empty()));
    assert_eq!(s.plan.verdict, PlanVerdict::Holds);
    assert!(s.info.design_in_force_holds);
    assert_eq!(s.info.stop, StopReason::Exhausted);
    assert!(CHEAPEST_NOTE.contains("not proven cheapest"));

    // a priced shrink says how much cheaper
    let yaml = PRICED.replace(
        "aws/m5.large }",
        "aws/m5.large, cpu: { cores: 2 }, replicas: 4 }",
    );
    let (s, _) = plain(&yaml, &|t| size(t, "x").0 >= 2);
    assert!(
        matches!(
            s.plan.steps[..],
            [Step {
                mv: Move {
                    kind: MoveKind::Copies { from: 4, to: 2 },
                    ..
                },
                ..
            }]
        ),
        "{:?}",
        s.plan.steps
    );
    let note = s.plan.steps[0].note.as_deref().unwrap();
    assert!(
        note.starts_with("cheaper by $") && note.ends_with("and still holds"),
        "{note}"
    );
    assert!(s.plan.price_after < s.plan.price_before);
}

/// A saving under a cent is still printed (exact dollars, never `$0.00`).
#[test]
fn money_is_exact_dollars() {
    assert_eq!(usd(840_000), "$0.84");
    assert_eq!(usd(3_500), "$0.0035");
    assert_eq!(usd(1), "$0.000001");
    assert_eq!(usd(1_234_567), "$1.234567");
    assert_eq!(usd(2_000_000), "$2.00");
}

#[test]
fn split_time_gives_half_to_the_main_search_and_splits_the_rest() {
    let l = Duration::from_secs(150);
    assert_eq!(split_time(l, 0), (l, vec![]));
    assert_eq!(
        split_time(l, 2),
        (
            Duration::from_secs(75),
            vec![Duration::from_millis(37_500); 2]
        )
    );
    assert_eq!(
        split_time(l, 1),
        (Duration::from_secs(75), vec![Duration::from_secs(75)])
    );
}
