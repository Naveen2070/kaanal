# `kaanal-sim/src/plan/` (Step 9c-1, sizing planner)

The check a candidate plan must pass. `plan/mod.rs` re-exports; `plan/check.rs` holds the code. Tests are in `tests/plan_check.rs`, against the M/M/1 closed form (capacity estimate within 5 percent of mu).

## `plan/check.rs`

- `Target { Multiple(f64), PerSecond(f64) }` and `factor(&base) -> Result<f64, String>`: a multiple is the load factor, a rate divides by the sum of the base stream rates. A value that is not finite or not above zero, or streams whose rates do not sum above zero (including no streams), is an error.
- `check(topo, endpoints, chaos, base, k, &Limit, &Limits) -> Result<Check, PointFail>`: one `run_point` at `k` with `base.seed`, the window sized by `point_cfg` (20 000 reported requests) and `floor(topo)`, recomputed per call because a candidate topology has its own floor. `Endpoints` and chaos of the original are reused for a candidate (tested).
- `Check { holds, point, capacity_req_s, bottleneck }`: `holds` is `Limit::passes_with_margin`. `capacity_req_s` is the lowest analytic `saturates_at_req_s` over the nodes (an estimate, not a measurement). `bottleneck` is the node with that lowest value, else the saturated node of highest utilisation, else `None`.

Reports built on a `Check` carry `kaanal_sim::summary::LABEL` ("relative what-if, not a prediction").

`sweep::point` became `pub(crate)` so the check shares the sweep's point runner.
