//! Helpers the walk (`walk.rs`) and the cheapest search share: the design a step stands on, its price
//! and effect, the margin and gain figures, one move tried, and the stop causes. Moved out of
//! `walk.rs` unchanged; `pub(super)` to the `plan` module only.

use std::collections::BTreeMap;

use kaanal_core::cost::{CostReport, estimate};
use kaanal_core::resize::{Move, MoveKind, apply};
use kaanal_core::{Config, Topology};

use super::Check;
use super::types::{Effect, PlanVerdict};
use crate::sweep::{Limit, Point, PointFail};

/// What a run refused by the search's limits says (the walk drops it as a refused candidate).
pub(super) const LIMIT_REACHED: &str = "search limit reached";

/// One design the walk stands on or tried.
#[derive(Clone)]
pub(super) struct State {
    pub(super) cfg: Config,
    pub(super) check: Check,
    pub(super) cost: CostReport,
}

pub(super) fn price(cfg: &Config) -> CostReport {
    estimate(cfg, &BTreeMap::new())
}

/// The priced total, `None` when no node is priced.
pub(super) fn shown(c: &CostReport) -> Option<u64> {
    (c.priced > 0).then_some(c.total_micro_usd_hr)
}

pub(super) fn node_priced(c: &CostReport, node: &str) -> bool {
    c.nodes
        .iter()
        .any(|n| n.name == node && n.total_micro_usd_hr.is_some())
}

#[expect(
    clippy::cast_possible_wrap,
    reason = "hourly micro-USD totals are far below 2^63"
)]
pub(super) fn delta(after: &CostReport, before: &CostReport) -> i64 {
    after.total_micro_usd_hr as i64 - before.total_micro_usd_hr as i64
}

pub(super) fn effect(c: &Check) -> Effect {
    Effect {
        p99_ms: c.point.p99_ms,
        error_rate: c.point.error_rate,
        capacity_req_s: c.capacity_req_s,
    }
}

/// How far over the line a point is: the larger of p99 / limit and errors / limit (1 is
/// on the line). A NaN figure, or one over a zero limit, is infinitely over.
pub(super) fn margin(l: &Limit, p: &Point) -> f64 {
    let frac = |x: f64, lim: f64| {
        if x.is_nan() {
            f64::INFINITY
        } else if lim > 0.0 {
            x / lim
        } else if x > 0.0 {
            f64::INFINITY
        } else {
            0.0
        }
    };
    frac(p.p99_ms, l.p99.as_secs_f64() * 1000.0).max(frac(p.error_rate, l.error_rate))
}

/// The margin drop, at least 0 (infinity minus infinity is no gain).
pub(super) fn gain(before: f64, after: f64) -> f64 {
    let g = before - after;
    if g.is_nan() { 0.0 } else { g.max(0.0) }
}

pub(super) fn label(mv: &Move) -> String {
    let what = match &mv.kind {
        MoveKind::Class { from, to } => format!("class {from} -> {to}"),
        MoveKind::Cores { from, to } => format!("cores {from} -> {to}"),
        MoveKind::Memory { from, to } => format!("memory {from} -> {to} bytes"),
        MoveKind::Copies { from, to } => format!("copies {from} -> {to}"),
    };
    format!("'{}' {what}", mv.node)
}

/// `mv` applied to a clone of `cfg`, built and run.
pub(super) fn try_move(
    cfg: &Config,
    mv: &Move,
    run: &mut dyn FnMut(&Topology) -> Result<Check, PointFail>,
) -> Result<State, Refusal> {
    let mut c = cfg.clone();
    apply(&mut c, mv).map_err(Refusal::Size)?;
    let topo = Topology::build(&c).map_err(|e| Refusal::Size(e.to_string()))?;
    let check = run(&topo).map_err(|e| match e {
        // a limit cut is not a size refusal: no "too large to run" prefix
        PointFail::TooLarge(e) if e == LIMIT_REACHED => Refusal::Size(e),
        PointFail::TooLarge(e) => Refusal::Size(format!("too large to run: {e}")),
        PointFail::Other(e) => Refusal::Fatal(e),
    })?;
    let cost = price(&c);
    Ok(State {
        cfg: c,
        check,
        cost,
    })
}

/// Why a candidate has no run.
pub(super) enum Refusal {
    /// Dropped from the menu with a note: too large to run, or a move the design refuses.
    Size(String),
    /// Ends the walk.
    Fatal(String),
}

pub(super) fn fail_text(e: &PointFail) -> &str {
    match e {
        PointFail::TooLarge(s) | PointFail::Other(s) => s,
    }
}

/// `Holds` for a holding design within `budget` (or with none), else `OverBudget`.
pub(super) fn priced_verdict(total: u64, budget: Option<u64>) -> PlanVerdict {
    match budget {
        Some(b) if total > b => PlanVerdict::OverBudget {
            over_by_micro_usd_hr: total - b,
            holds: true,
        },
        _ => PlanVerdict::Holds,
    }
}

/// A cause more size cannot fix (spec: a limit the developer set, or chaos, ends the
/// plan instead of sizing an unrelated node). Looked at only when the errors are over the plain limit
/// and the most common failure cause is a rate limit (named when one node has one), chaos, or a
/// deadline while no node saturates (a deadline with a saturated node is a queue a size step can
/// shorten). The node, when known, and the reason.
pub(super) fn fixed_cause(
    cfg: &Config,
    c: &Check,
    limit: &Limit,
) -> Option<(Option<String>, String)> {
    if !limit.over(&c.point).errors {
        return None;
    }
    match c.point.cause? {
        "rate_limit" => {
            let mut limited = cfg.nodes.iter().filter(|(_, n)| n.rate_limit().is_some());
            let node = match (limited.next(), limited.next()) {
                (Some((name, _)), None) => Some(name.clone()),
                _ => None,
            };
            Some((node, "most failed requests at the target are turned away by a rate limit you set; a plan never raises it, and more size cannot".into()))
        }
        "chaos" => Some((None, "most failed requests at the target fail from the chaos in force; more size cannot remove it".into())),
        "deadline" if !c.point.nodes.iter().any(|n| n.saturated) => Some((None, "most failed requests at the target run out a deadline you set while no node saturates; more size cannot lengthen it".into())),
        _ => None,
    }
}
