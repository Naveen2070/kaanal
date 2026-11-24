//! The walk (Step 9c-1, spec "Design (A)"): check the design at the target, move its bottleneck one
//! step, repeat. Each step tries **every** move [`moves_for`] gives the bottleneck on a clone of the
//! design (the menu) and takes the best by gain per dollar:
//!
//! - gain is the drop of the over-limit margin `max(p99 / limit, errors / limit)` (floor 0);
//! - moves with a gain come first; among them priced moves (the moved node has a price) by gain per
//!   added micro-dollar an hour, then unpriced ones by the smallest step (fewest added cores or
//!   copies); a priced move that costs nothing or saves comes before one that costs (larger gain,
//!   then larger saving first); a tie goes to the cheaper design, then to the lower move label.
//!
//! On an `oom` cause the menu is [`moves_for_oom`] (memory instead of cores). When the errors are
//! over the limit because of a rate limit you set, chaos, or a deadline with no node saturated, the
//! walk stops with `CannotHold` before sizing anything ([`fixed_cause`]).
//!
//! The walk stops when the design holds, when the bottleneck has no move (or every move is refused
//! for size), after two steps in a row without gain, at `max_steps`, or when the next step's price
//! would pass the budget (priced nodes only). At any of those exits the cheapest holding design
//! seen in any menu, if one was, is the plan: a holding design is never hidden. A `NoProgress` plan
//! ends at the last step that gained. Every candidate run uses the same seed (common random numbers)
//! and is reduced to its [`Check`] before the next. The input config is never changed.
//! Prices come from `cost::estimate` with no visits: per-use prices are added by the server's final
//! run. A greedy walk: it finds *a* plan, not the cheapest (that is the 9c-2 search).

use std::cmp::Ordering;

use kaanal_core::resize::{
    Move, MoveKind, copies_only_reason, moves_for, moves_for_oom, why_fixed,
};
use kaanal_core::{Config, EffectiveChaos, Endpoints, Topology};

use super::common::{
    Refusal, State, delta, effect, fail_text, fixed_cause, gain, label, margin, node_priced, price,
    priced_verdict, shown, try_move,
};
use super::types::{Candidate, Effect, Plan, PlanVerdict, Request, Step};
use super::{Check, check};
use crate::sweep::PointFail;
use crate::{Limits, SimConfig};

/// A margin drop at or under this is no gain (float noise, not progress).
const NO_GAIN: f64 = 1e-9;
/// Steps in a row without gain that end the walk.
const STALL_STEPS: u32 = 2;

/// Walks `cfg` toward a design that holds `req.target` under `req.limit` (with the sweep's margin).
/// `endpoints` and `chaos` of the design in force are reused for every candidate: a size move does
/// not change a node id or a route. `on_step` is called after each step taken, in order.
///
/// # Errors
/// A bad target, a design in force that cannot be built or run (too large included), or a candidate
/// run that fails for a reason other than size.
pub fn walk(
    cfg: &Config,
    endpoints: &Endpoints,
    chaos: &EffectiveChaos,
    base: &SimConfig,
    req: &Request,
    limits: &Limits,
    on_step: &mut dyn FnMut(&Step),
) -> Result<Plan, String> {
    let k = req.target.factor(base)?;
    walk_by(
        cfg,
        req,
        &mut |t| check(t, endpoints, chaos, base, k, &req.limit, limits),
        on_step,
    )
}

/// **Private.** The cheapest holding design seen in any menu, with the steps that reach it.
struct Best {
    total: u64,
    steps: Vec<Step>,
    state: State,
}

/// [`walk`] with the run injected, so the rules can be tested without the engine.
///
/// A step goes to `on_step` once a step at or after it gains, or when the walk ends with it in the
/// plan, so the two steps a `NoProgress` plan drops are never sent. A plan that branches off an
/// earlier menu (its cheapest holding candidate) supersedes the steps sent past the branch point:
/// `Plan.steps` is the answer, the sent steps are progress.
pub(super) fn walk_by(
    cfg: &Config,
    req: &Request,
    run: &mut dyn FnMut(&Topology) -> Result<Check, PointFail>,
    on_step: &mut dyn FnMut(&Step),
) -> Result<Plan, String> {
    let topo = Topology::build(cfg).map_err(|e| e.to_string())?;
    let check = run(&topo).map_err(|e| format!("the design in force: {}", fail_text(&e)))?;
    let mut st = State {
        cfg: cfg.clone(),
        check,
        cost: price(cfg),
    };
    let start = effect(&st.check);
    let price_before = shown(&st.cost);
    if st.check.holds {
        return Ok(plan(
            PlanVerdict::AlreadyHolds,
            Vec::new(),
            st,
            start,
            price_before,
        ));
    }
    let mut steps: Vec<Step> = Vec::new();
    let mut sent = 0;
    let mut stalled = 0;
    let mut before_stall: Option<State> = None;
    let mut best: Option<Best> = None;
    let (verdict, path, end) = loop {
        if steps.len() >= req.max_steps as usize {
            break (PlanVerdict::StepCap, steps, st);
        }
        if let Some((node, reason)) = fixed_cause(&st.cfg, &st.check, &req.limit) {
            break (PlanVerdict::CannotHold { node, reason }, steps, st);
        }
        let Some(node) = st.check.bottleneck.clone() else {
            let by = st.check.point.cause.unwrap_or("p99 latency");
            let reason = format!("no node saturates at the target: the limit is crossed by {by}");
            break (PlanVerdict::CannotHold { node: None, reason }, steps, st);
        };
        let mut m = match Menu::of(&st, &node, req, run)? {
            Ok(m) => m,
            Err(reason) => {
                let node = Some(node);
                break (PlanVerdict::CannotHold { node, reason }, steps, st);
            }
        };
        m.keep_cheapest_holding(&mut best, &steps, &st.check);
        let pick = (0..m.keys.len())
            .min_by(|&a, &b| rank(&m.keys[a], &m.keys[b]))
            .expect("a menu has at least one candidate");
        // a step that adds nothing to the price never trips the budget (a design in force already
        // over it can still be sized by free moves)
        if let Some(budget) = req.budget_micro_usd_hr
            && m.keys[pick].total > budget
            && m.keys[pick].total > st.cost.total_micro_usd_hr
        {
            // the next step needed passes the budget; a holding design priced in any menu replaces this below
            let verdict = PlanVerdict::OverBudget {
                over_by_micro_usd_hr: m.keys[pick].total - budget,
                holds: false,
            };
            break (verdict, steps, st);
        }
        steps.push(m.step(pick, &st.check));
        let next = m.tried.swap_remove(pick).1;
        if m.keys[pick].gain > NO_GAIN {
            stalled = 0;
            for s in &steps[sent..] {
                on_step(s);
            }
            sent = steps.len();
        } else {
            stalled += 1;
            if stalled == 1 {
                before_stall = Some(st);
            }
        }
        st = next;
        if st.check.holds {
            break (PlanVerdict::Holds, steps, st);
        }
        if stalled >= STALL_STEPS {
            // the steps that gained nothing are reported by the verdict, not proposed
            steps.truncate(steps.len() - STALL_STEPS as usize);
            let at = before_stall
                .take()
                .expect("a stall starts with a saved state");
            break (PlanVerdict::NoProgress { node }, steps, at);
        }
    };
    let (verdict, path, end, shared) = settle(verdict, path, end, best, req.budget_micro_usd_hr);
    // send the rest of the plan unless it branches off before a step already sent
    if shared >= sent {
        for s in &path[sent..] {
            on_step(s);
        }
    }
    Ok(plan(verdict, path, end, start, price_before))
}

/// **Private.** Where the walk ends, with how many of its steps the steps already sent share.
fn settle(
    verdict: PlanVerdict,
    path: Vec<Step>,
    end: State,
    best: Option<Best>,
    budget: Option<u64>,
) -> (PlanVerdict, Vec<Step>, State, usize) {
    // a holding design priced in any menu is never hidden: it beats a failure, and a cheaper one
    // beats the pick that held
    match best {
        Some(b) if verdict != PlanVerdict::Holds || b.total < end.cost.total_micro_usd_hr => {
            let shared = b.steps.len() - 1;
            let v = priced_verdict(b.total, budget);
            (v, b.steps, b.state, shared)
        }
        _ => {
            let shared = path.len();
            // a holding pick reached by free steps on a design already over the budget is over it too
            let verdict = match verdict {
                PlanVerdict::Holds => priced_verdict(end.cost.total_micro_usd_hr, budget),
                v => v,
            };
            (verdict, path, end, shared)
        }
    }
}

/// **Private.** The answer for a walk that ends on `st` after `steps`.
fn plan(
    verdict: PlanVerdict,
    steps: Vec<Step>,
    st: State,
    start: Effect,
    price_before: Option<u64>,
) -> Plan {
    Plan {
        verdict,
        steps,
        start,
        end: effect(&st.check),
        price_before,
        price_after: shown(&st.cost),
        unpriced_nodes: st
            .cost
            .nodes
            .iter()
            .filter(|n| n.total_micro_usd_hr.is_none())
            .map(|n| n.name.clone())
            .collect(),
        config: st.cfg,
        families: Vec::new(),
    }
}

/// **Private.** Every move tried for one node at one step.
struct Menu {
    tried: Vec<(Move, State)>,
    keys: Vec<Key>,
    cands: Vec<Candidate>,
    note: Option<String>,
}

impl Menu {
    /// The menu for `node` on `st`; the inner `Err` is why the node cannot be sized (no move, or
    /// every move refused), the outer one a run that failed for another reason.
    fn of(
        st: &State,
        node: &str,
        req: &Request,
        run: &mut dyn FnMut(&Topology) -> Result<Check, PointFail>,
    ) -> Result<Result<Menu, String>, String> {
        // spec: memory moves only when the cause is `oom`
        let oom = st.check.point.cause == Some("oom");
        let moves = if oom {
            moves_for_oom(&st.cfg, node, &req.keep)
        } else {
            moves_for(&st.cfg, node, &req.keep)
        };
        if moves.is_empty() {
            let why = if oom {
                "no memory or copy move (the cause is oom)"
            } else {
                why_fixed(&st.cfg, node, &req.keep).unwrap_or("no size move")
            };
            return Ok(Err(why.into()));
        }
        let mut notes = Vec::new();
        let mut tried = Vec::new();
        for mv in moves {
            match try_move(&st.cfg, &mv, run) {
                Ok(s) => tried.push((mv, s)),
                Err(Refusal::Size(e)) => notes.push(format!("{} refused: {e}", label(&mv))),
                Err(Refusal::Fatal(e)) => return Err(format!("{}: {e}", label(&mv))),
            }
        }
        if tried.is_empty() {
            return Ok(Err(notes.join("; ")));
        }
        if let Some(why) = copies_only_reason(&st.cfg, node, &req.keep) {
            notes.push(format!("'{node}' grows by copies only: {why}"));
        }
        let m0 = margin(&req.limit, &st.check.point);
        let keys: Vec<Key> = tried
            .iter()
            .map(|(mv, s)| Key {
                gain: gain(m0, margin(&req.limit, &s.check.point)),
                delta: node_priced(&s.cost, node).then(|| delta(&s.cost, &st.cost)),
                size: size(&mv.kind),
                total: s.cost.total_micro_usd_hr,
                label: label(mv),
            })
            .collect();
        let cands = tried
            .iter()
            .zip(&keys)
            .map(|((mv, s), k)| Candidate {
                mv: mv.clone(),
                price_micro_usd_hr: k.delta.map(|_| k.total),
                effect: effect(&s.check),
                holds: s.check.holds,
            })
            .collect();
        Ok(Ok(Menu {
            tried,
            keys,
            cands,
            note: (!notes.is_empty()).then(|| notes.join("; ")),
        }))
    }

    /// Records in `best` a holding candidate cheaper than it, reached by `steps` then that move.
    fn keep_cheapest_holding(&self, best: &mut Option<Best>, steps: &[Step], before: &Check) {
        for i in (0..self.tried.len()).filter(|&i| self.cands[i].holds) {
            if best.as_ref().is_none_or(|b| self.keys[i].total < b.total) {
                let mut path = steps.to_vec();
                path.push(self.step(i, before));
                *best = Some(Best {
                    total: self.keys[i].total,
                    steps: path,
                    state: self.tried[i].1.clone(),
                });
            }
        }
    }

    /// Candidate `i` as a step from `before`.
    fn step(&self, i: usize, before: &Check) -> Step {
        Step {
            mv: self.cands[i].mv.clone(),
            price_micro_usd_hr: self.cands[i].price_micro_usd_hr,
            price_delta_micro_usd_hr: self.keys[i].delta,
            before: effect(before),
            after: self.cands[i].effect.clone(),
            menu: self.cands.clone(),
            note: self.note.clone(),
        }
    }
}

/// **Private.** What a move adds: cores or copies; a class or memory step counts as one.
fn size(k: &MoveKind) -> f64 {
    match k {
        MoveKind::Cores { from, to } => to - from,
        MoveKind::Copies { from, to } => f64::from(to.saturating_sub(*from)),
        MoveKind::Class { .. } | MoveKind::Memory { .. } => 1.0,
    }
}

/// **Private.** What the choice of a move looks at.
#[derive(Debug, Clone)]
struct Key {
    gain: f64,
    /// Added micro-USD an hour, when the moved node is priced.
    delta: Option<i64>,
    size: f64,
    total: u64,
    label: String,
}

/// **Private.** `Less` when `a` is the better move (the order in the module doc).
fn rank(a: &Key, b: &Key) -> Ordering {
    #[expect(
        clippy::cast_precision_loss,
        reason = "hourly prices are far below 2^52"
    )]
    let per_dollar = |k: &Key, d: i64| k.gain / d as f64;
    (b.gain > NO_GAIN)
        .cmp(&(a.gain > NO_GAIN))
        .then(b.delta.is_some().cmp(&a.delta.is_some()))
        .then_with(|| match (a.delta, b.delta) {
            // a move that costs nothing or saves beats one that costs; among those, the larger
            // gain, then the larger saving
            (Some(da), Some(db)) => (da > 0).cmp(&(db > 0)).then_with(|| {
                if da > 0 {
                    per_dollar(b, db).total_cmp(&per_dollar(a, da))
                } else {
                    b.gain.total_cmp(&a.gain).then(da.cmp(&db))
                }
            }),
            _ => a.size.total_cmp(&b.size),
        })
        .then(a.total.cmp(&b.total))
        .then_with(|| a.label.cmp(&b.label))
}

#[cfg(test)]
#[path = "walk_tests.rs"]
mod tests;
