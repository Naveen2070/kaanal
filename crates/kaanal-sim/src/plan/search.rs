//! The cheapest search (Step 9c-2, spec "Design (A)" of 2026-11-22): from the 9c-1 walk's holding
//! design, take off what is not needed, and stop at the cheapest holding design found within the
//! limits. Every answer carries [`super::CHEAPEST_NOTE`].
//!
//! 1. The walk ([`walk_by`]) from the design in force; its first run is the check of the design in
//!    force (`AlreadyHolds` when it holds, so no run is spent twice). A walk that ends without a
//!    holding design is the answer as it is (`Exhausted`): the search invents no step.
//!    The limits below bind the walk too: once one fires, every further run is refused as
//!    "search limit reached" (the walk drops it as a refused candidate and ends on what it has
//!    seen, its cheapest holding candidate when it saw one), and the walk's plan is the answer with
//!    the limit as the stop reason.
//! 2. **Descent** (coordinate descent with a bisection per knob): passes over the priced
//!    nodes, most expensive first (ties by name; an unpriced node is never shrunk, RULES 142 e); for each node
//!    and knob (class, copies) the [`cheaper_moves`] of that knob, in its own order, are
//!    bisected for the first candidate that holds (Knuth, *TAOCP* vol. 3, 6.2.1: binary search for the
//!    least index with a monotone predicate, ceil(log2(n + 1)) runs for n candidates). A knob's list
//!    is ascending in the node's size: by price (classes on the Pareto front by
//!    price, copies by count). The one assumption, the one [`super::CHEAPEST_NOTE`] names: "holds" is monotone along
//!    that list (a larger node never holds less). 9a showed it can fail (retries, breakers), so a
//!    holding design is only ever replaced by another that holds: a wrong guess costs a cheaper
//!    design missed, never a plan that fails. Stops when a whole pass takes nothing (`Exhausted`),
//!    at `run_cap` runs (the walk's included, `RunCap`), or when the injected clock reads at or
//!    past `time_limit` before a run (`TimeLimit`; every run but the first, so the overshoot is at
//!    most the one run in flight and `runs` never passes `run_cap`); a holding candidate found by a bisection
//!    the stop cuts short is still taken.
//! 3. The plan: the walk's steps, then the search's moves **collapsed per (node, knob)** (`from` the
//!    value before the first, `to` after the last, `before` of the first, `after` of the last, no
//!    menu, a note saying what it saves), the verdict by the 9c-1 budget rule. `on_step` gets the
//!    walk's steps as the walk sends them, then each move taken, uncollapsed, as progress.
//!
//! A node in `req.keep` has no candidate ([`cheaper_moves`]), so it is never changed. A candidate
//! refused for size counts as not holding and is named in `SearchInfo.notes`. Same request, seed and
//! design give the same plan when the limits do not stop it (a fixed order, one seed for every run).

use std::cell::Cell;
use std::time::Duration;

use kaanal_core::cost::CostReport;
use kaanal_core::resize::{Move, MoveKind, cheaper_moves};
use kaanal_core::{Config, EffectiveChaos, Endpoints, Topology};

use super::common::{
    LIMIT_REACHED, Refusal, State, delta, effect, label, price, priced_verdict, shown, try_move,
};
use super::search_types::{SearchInfo, SearchOptions, Searched, StopReason};
use super::types::{Effect, Plan, PlanVerdict, Request, Step};
use super::walk::walk_by;
use super::{Check, check};
use crate::sweep::PointFail;
use crate::{Limits, SimConfig};

/// Searches for the cheapest design that holds `req.target` (module doc). `now` is the time since
/// the search began (the server passes an `Instant` closure); it is read before each run of the
/// descent. Arguments otherwise as [`super::walk`].
///
/// # Errors
/// As [`super::walk`]: a bad target, a design in force that cannot be built or run, or a run that
/// fails for a reason other than size.
#[expect(
    clippy::too_many_arguments,
    reason = "the walk's inputs plus the search's bounds and clock"
)]
pub fn search(
    cfg: &Config,
    endpoints: &Endpoints,
    chaos: &EffectiveChaos,
    base: &SimConfig,
    req: &Request,
    opts: &SearchOptions,
    limits: &Limits,
    now: &mut dyn FnMut() -> Duration,
    on_step: &mut dyn FnMut(&Step),
) -> Result<Searched, String> {
    let k = req.target.factor(base)?;
    search_by(
        cfg,
        req,
        opts,
        now,
        &mut |t| check(t, endpoints, chaos, base, k, &req.limit, limits),
        on_step,
    )
}

/// **Private.** The design the descent stands on: no `Check` is needed past its effect.
struct At {
    cfg: Config,
    cost: CostReport,
    end: Effect,
}

/// **Private.** [`search`] with the run injected, as [`walk_by`].
fn search_by(
    cfg: &Config,
    req: &Request,
    opts: &SearchOptions,
    now: &mut dyn FnMut() -> Duration,
    run: &mut dyn FnMut(&Topology) -> Result<Check, PointFail>,
    on_step: &mut dyn FnMut(&Step),
) -> Result<Searched, String> {
    let runs = Cell::new(0u32);
    // the limit that cut the search short; checked before every run but the first (the design in
    // force), walk included, so the overshoot is at most the one run in flight
    let cut = Cell::new(None);
    let mut counted = |t: &Topology| {
        if runs.get() > 0 && cut.get().is_none() {
            if runs.get() >= opts.run_cap {
                cut.set(Some(StopReason::RunCap));
            } else if now() >= opts.time_limit {
                cut.set(Some(StopReason::TimeLimit));
            }
        }
        if cut.get().is_some() {
            // the walk drops it as a refused candidate and ends on what it has seen
            return Err(PointFail::TooLarge(LIMIT_REACHED.into()));
        }
        runs.set(runs.get() + 1);
        run(t)
    };
    let walked = walk_by(cfg, req, &mut counted, on_step)?;
    let design_in_force_holds = walked.verdict == PlanVerdict::AlreadyHolds;
    let mut notes = Vec::new();
    let holds = matches!(
        walked.verdict,
        PlanVerdict::AlreadyHolds
            | PlanVerdict::Holds
            | PlanVerdict::OverBudget { holds: true, .. }
    );
    if !holds || cut.get().is_some() {
        let stop = cut.get().unwrap_or(StopReason::Exhausted);
        return Ok(Searched {
            info: info(stop, runs.get(), now(), opts, design_in_force_holds, notes),
            plan: walked,
        });
    }
    let mut at = At {
        cost: price(&walked.config),
        cfg: walked.config.clone(),
        end: walked.end.clone(),
    };
    let mut taken: Vec<Step> = Vec::new();
    let stop = descend(
        req,
        &mut at,
        &mut counted,
        &cut,
        &mut taken,
        &mut notes,
        on_step,
    )?;
    let plan = finish(walked, at, taken, req.budget_micro_usd_hr);
    Ok(Searched {
        info: info(stop, runs.get(), now(), opts, design_in_force_holds, notes),
        plan,
    })
}

/// **Private.** The passes of the descent (module doc, step 2); `taken` gets the collapsed steps.
fn descend(
    req: &Request,
    at: &mut At,
    run: &mut dyn FnMut(&Topology) -> Result<Check, PointFail>,
    cut: &Cell<Option<StopReason>>,
    taken: &mut Vec<Step>,
    notes: &mut Vec<String>,
    on_step: &mut dyn FnMut(&Step),
) -> Result<StopReason, String> {
    loop {
        let mut moved = false;
        for node in order(&at.cost) {
            for knob in Knob::ALL {
                let list: Vec<Move> = cheaper_moves(&at.cfg, &node, &req.keep)
                    .into_iter()
                    .filter(|m| Knob::of(&m.kind) == Some(knob))
                    .collect();
                let (found, stop) = bisect(&at.cfg, &list, run, cut, notes)?;
                if let Some((mv, st)) = found {
                    let s = Step {
                        // every node of a pass is priced (see `order`)
                        price_micro_usd_hr: Some(st.cost.total_micro_usd_hr),
                        price_delta_micro_usd_hr: Some(delta(&st.cost, &at.cost)),
                        before: at.end.clone(),
                        after: effect(&st.check),
                        menu: Vec::new(),
                        note: None,
                        mv,
                    };
                    on_step(&s);
                    merge(taken, s);
                    *at = At {
                        end: effect(&st.check),
                        cfg: st.cfg,
                        cost: st.cost,
                    };
                    moved = true;
                }
                if let Some(stop) = stop {
                    return Ok(stop);
                }
            }
        }
        if !moved {
            return Ok(StopReason::Exhausted);
        }
    }
}

/// **Private.** The knobs the descent turns, in order.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Knob {
    Class,
    Copies,
}

impl Knob {
    const ALL: [Knob; 2] = [Knob::Class, Knob::Copies];

    fn of(k: &MoveKind) -> Option<Knob> {
        match k {
            MoveKind::Class { .. } => Some(Knob::Class),
            MoveKind::Copies { .. } => Some(Knob::Copies),
            MoveKind::Cores { .. } | MoveKind::Memory { .. } => None,
        }
    }
}

/// **Private.** The nodes of a pass: the priced ones, most expensive first (ties by name). An
/// unpriced node is never shrunk (RULES 142 e): it would spend runs and cost redundancy for no money.
fn order(cost: &CostReport) -> Vec<String> {
    let mut priced: Vec<(&str, u64)> = cost
        .nodes
        .iter()
        .filter_map(|n| Some((n.name.as_str(), n.total_micro_usd_hr?)))
        .collect();
    priced.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
    priced.into_iter().map(|(n, _)| n.to_string()).collect()
}

/// **Private.** The first move of `list` whose design holds, by bisection (module doc); the design
/// it stands on holds, so the answer lies in `0..=len` (`len`: none). A refused candidate counts as
/// not holding. The second value is the limit that cut it short.
#[expect(
    clippy::type_complexity,
    reason = "a found move and why the search stops, read once"
)]
fn bisect(
    cfg: &Config,
    list: &[Move],
    run: &mut dyn FnMut(&Topology) -> Result<Check, PointFail>,
    cut: &Cell<Option<StopReason>>,
    notes: &mut Vec<String>,
) -> Result<(Option<(Move, State)>, Option<StopReason>), String> {
    let (mut lo, mut hi) = (0, list.len());
    let mut found = None;
    while lo < hi {
        let mid = lo + (hi - lo) / 2;
        let tried = try_move(cfg, &list[mid], run);
        if let Some(stop) = cut.get() {
            return Ok((found, Some(stop)));
        }
        match tried {
            Ok(st) if st.check.holds => {
                hi = mid;
                found = Some((list[mid].clone(), st));
            }
            Ok(_) => lo = mid + 1,
            Err(Refusal::Size(e)) => {
                notes.push(format!("{} refused: {e}", label(&list[mid])));
                lo = mid + 1;
            }
            Err(Refusal::Fatal(e)) => return Err(format!("{}: {e}", label(&list[mid]))),
        }
    }
    Ok((found, None))
}

/// **Private.** Folds `s` into the step of the same node and knob in `taken`, else appends it.
fn merge(taken: &mut Vec<Step>, s: Step) {
    let same =
        |t: &&mut Step| t.mv.node == s.mv.node && Knob::of(&t.mv.kind) == Knob::of(&s.mv.kind);
    let Some(t) = taken.iter_mut().find(same) else {
        taken.push(s);
        return;
    };
    match (&mut t.mv.kind, s.mv.kind) {
        (MoveKind::Class { to, .. }, MoveKind::Class { to: new, .. }) => *to = new,
        (MoveKind::Copies { to, .. }, MoveKind::Copies { to: new, .. }) => *to = new,
        _ => unreachable!("the same knob"),
    }
    t.price_micro_usd_hr = s.price_micro_usd_hr;
    t.price_delta_micro_usd_hr = t
        .price_delta_micro_usd_hr
        .zip(s.price_delta_micro_usd_hr)
        .map(|(a, b)| a + b);
    t.after = s.after;
}

/// **Private.** The plan of a search that held: the walk's steps, then the collapsed ones.
fn finish(walked: Plan, at: At, mut taken: Vec<Step>, budget: Option<u64>) -> Plan {
    for t in &mut taken {
        // a taken move is strictly cheaper (a class from the front, or fewer copies)
        let d = t.price_delta_micro_usd_hr.unwrap_or_default();
        t.note = Some(format!(
            "cheaper by {} an hour and still holds",
            usd(d.unsigned_abs())
        ));
    }
    let verdict = if taken.is_empty() {
        walked.verdict
    } else {
        priced_verdict(at.cost.total_micro_usd_hr, budget)
    };
    let mut steps = walked.steps;
    steps.append(&mut taken);
    Plan {
        verdict,
        steps,
        start: walked.start,
        end: at.end,
        price_before: walked.price_before,
        price_after: shown(&at.cost),
        unpriced_nodes: at
            .cost
            .nodes
            .iter()
            .filter(|n| n.total_micro_usd_hr.is_none())
            .map(|n| n.name.clone())
            .collect(),
        config: at.cfg,
        families: Vec::new(),
    }
}

/// **Private.** Micro-USD as exact dollars, at least two decimals and no float: `840_000` is
/// `$0.84`, `3_500` is `$0.0035` (rounding to the cent would print a real saving as `$0.00`).
fn usd(micro: u64) -> String {
    let mut frac = format!("{:06}", micro % 1_000_000);
    while frac.len() > 2 && frac.ends_with('0') {
        frac.pop();
    }
    format!("${}.{frac}", micro / 1_000_000)
}

fn info(
    stop: StopReason,
    runs: u32,
    elapsed: Duration,
    opts: &SearchOptions,
    design_in_force_holds: bool,
    notes: Vec<String>,
) -> SearchInfo {
    let ms = |d: Duration| u64::try_from(d.as_millis()).unwrap_or(u64::MAX);
    SearchInfo {
        stop,
        runs,
        elapsed_ms: ms(elapsed),
        limit_ms: ms(opts.time_limit),
        run_cap: opts.run_cap,
        design_in_force_holds,
        notes,
    }
}

#[cfg(test)]
#[path = "search_tests.rs"]
mod tests;
