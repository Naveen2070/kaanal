# `kaanal-sim/src/plan/` (Step 9c-1, sizing planner)

The check a candidate plan must pass, and the walk. `plan/mod.rs` re-exports; `plan/check.rs`, `plan/types.rs` and `plan/walk.rs` hold the code. Tests are in `tests/plan_check.rs` (M/M/1: capacity estimate within 5 percent of mu) and `tests/plan_walk.rs` (M/M/c: the copies the walk picks are the fewest Erlang C allows, capacity after the walk within 5 percent of c x mu), plus unit tests of the ranking and of the walk's rules on scripted runs in `plan/walk_tests.rs` (a `#[cfg(test)]` module of `walk.rs`, split out to keep it under the size seam).

## `plan/check.rs`

- `Target { Multiple(f64), PerSecond(f64) }` and `factor(&base) -> Result<f64, String>`: a multiple is the load factor, a rate divides by the sum of the base stream rates. A value that is not finite or not above zero, or streams whose rates do not sum above zero (including no streams), is an error.
- `check(topo, endpoints, chaos, base, k, &Limit, &Limits) -> Result<Check, PointFail>`: one `run_point` at `k` with `base.seed`, the window sized by `point_cfg` (20 000 reported requests) and `floor(topo)`, recomputed per call because a candidate topology has its own floor. `Endpoints` and chaos of the original are reused for a candidate (tested).
- `Check { holds, point, capacity_req_s, bottleneck }`: `holds` is `Limit::passes_with_margin`. `capacity_req_s` is the lowest analytic `saturates_at_req_s` over the nodes (an estimate, not a measurement). `bottleneck` is the node with that lowest value, else the saturated node of highest utilisation, else `None`.

## `plan/types.rs`

- `Request { target, limit, budget_micro_usd_hr, keep, max_steps }` (`Default`: 1x, the sweep's limit, no budget, nothing kept, 30 steps). `Debug, Clone` only: `Target` is not `Serialize`.
- `Effect { p99_ms, error_rate, capacity_req_s }`, `Candidate { mv, price_micro_usd_hr, effect, holds }` (a menu entry), `Step { mv, price_micro_usd_hr, price_delta_micro_usd_hr, before, after, menu, note }`.
- `PlanVerdict { AlreadyHolds, Holds, CannotHold { node: Option<String>, reason }, OverBudget { over_by_micro_usd_hr }, NoProgress { node }, StepCap }` (serialised with a `kind` tag).
- `Plan { verdict, steps, start, end, price_before, price_after, unpriced_nodes, config, families }`: `config` (the plan's design) is not serialised; `families` (`FamilyAnswer { node, flavor, plan, notes }`) is filled by the server, empty from `walk`.
- Prices are micro-USD an hour of the priced nodes only (`None` when the moved node, or for the plan no node, is priced), estimates.

## `plan/walk.rs` (tests in `plan/walk_tests.rs`)

- `walk(cfg, endpoints, chaos, base, &Request, &Limits, on_step) -> Result<Plan, String>`: check at `target.factor(base)`; while it does not hold, try every `moves_for` move (`moves_for_oom` when the point's cause is `oom`: memory instead of cores) of the check's `bottleneck` on a clone (apply, `Topology::build`, `check`): that is the menu. Endpoints and chaos are reused. Prices from `cost::estimate` with no visits.
- Choice: gain is the drop of `max(p99 / limit, errors / limit)` (floor 0; a NaN figure or a figure over a zero limit is infinitely over). Moves with a gain first; priced moves by gain per added micro-dollar, a move that costs nothing or saves before one that costs (larger gain, then larger saving first), then unpriced by the smallest step (added cores or copies; a class or memory step is 1); ties to the cheaper design, then the lower move label.
- Stops: `Holds`; `CannotHold` before any menu when `fixed_cause` finds the errors over the plain limit and the most common cause is `rate_limit` (the node named when exactly one node has a `rate_limit`), `chaos`, or `deadline` with no node saturated (node `None`); `CannotHold` when the bottleneck has no move (`why_fixed`), every move is refused, or no node saturates (`node: None`, serialised `null`; the reason names the crossed limit); `NoProgress` after two steps in a row with a gain at or under 1e-9 (the plan ends at the last step that gained: the two steps, their design and price are dropped); `StepCap` at `max_steps`; when the chosen move's design passes the budget, the cheapest holding candidate seen in any menu (with the steps to it) is the plan (`Holds` if within budget, else `OverBudget` by its excess), else the walk's last state, over by the chosen move's price. The budget trips only on a pick whose design costs more than the budget **and** more than the design it stands on (a free step on a design already over the budget goes on); a holding pick reached that way is `OverBudget { holds: true }`, not `Holds`.
- A holding design is never hidden: the cheapest holding candidate seen in any menu (with the steps to it) is the plan at every exit after the first step (`CannotHold`, `NoProgress`, `StepCap`, and `Holds` when it is cheaper than the pick that held), as `Holds`, or `OverBudget` by its excess over a budget.
- A candidate refused for size (`PointFail::TooLarge`), or one `apply`/`Topology::build` refuses, is dropped from the menu and named in the step's `note`; a node that grows by copies only says why there too. Any other run error ends the walk with an error. `on_step` gets a step once a step at or after it gains, or at the end when it is in the plan, so the steps a `NoProgress` plan drops are never sent and a budget stop sends the plan's last step (sent steps equal `Plan.steps`). Only a plan that branches off an earlier menu (its cheapest holding candidate) supersedes steps already sent past the branch point; `Plan.steps` is then the answer.

Reports built on a `Check` carry `kaanal_sim::summary::LABEL` ("relative what-if, not a prediction").

`sweep::point` became `pub(crate)` so the check shares the sweep's point runner.
