# `kaanal-sim/src/sweep/` (Step 9a, load sweep)

Finds the load factor `k` (a multiple of the configured arrival rate) at which a topology first breaks a limit. `search.rs` is the pure search logic, driven in its unit tests by synthetic curves; `point.rs` runs the engine at one `k`; `sweep()` in `mod.rs` joins them. Integration tests against queueing oracles are in `tests/sweep.rs`.

## `sweep/mod.rs`

`sweep(topo, endpoints, chaos, base, &Options, &Limits, on_point) -> Result<SweepResult, String>`:

- `Options { limit, max_factor (8), target_requests (20 000) }`. A `max_factor` that is not finite or is under 0.25, or a `target_requests` under 1000, is refused before any run.
- **Engine rate cap.** The engine refuses any stream over `MAX_RATE_PER_S` (1e6 req/s, `lib.rs`, public so the server's plan route can refuse a target over it) before it checks its hop caps. A point keeps about `target_requests`, so it seldom nears the hop caps, but a slow node makes the window floor (ten times its mean `proc`) long, and then the hop caps do refuse first (`stopped_at`, "run too large"). `sweep` lowers the ladder top to a millionth under `MAX_RATE_PER_S / fastest stream rate`. When that drops a ladder point, `stopped_at` is the first dropped factor and a warning names the cut; a hop-cap refusal inside the cut ladder takes precedence. If even the lowest factor (0.25) passes the cap, the sweep returns `Err` naming `k = 0.25`.
- `base.streams` give the rates and sizes, `base.seed` is the seed at every point (common random numbers); `base.duration` and `base.warmup` are ignored. `on_point` is called after each point in run order (the server's progress).
- `SweepResult { label, limit, points, verdict, recovers_at, stopped_at, first_to_saturate, dominant_cause, warnings }`. `first_to_saturate` is the node with `saturated` set at the lowest `k` (the summary's flag; for a node with slots: offered load at least 1, utilization at least 0.95, or requests refused by a full queue or a deadline; a node without slots, such as a rate-limited gateway, never saturates), with its analytic `saturates_at_req_s` from the lowest point. `dominant_cause` is the `cause` of the lowest point over the limit. Warnings: no node saturated by the first crossing (naming what crossed it: the cause, or p99 latency), a recovery, a cut ladder (`ladder stopped at Kx (N req/s): run too large` for a hop-cap refusal, `ladder cut at Kx (N req/s): ...` for the rate cap), a point under 1000 reported requests.

The types, all `Serialize`:

- `Limit { p99: Duration, error_rate }`. Default is 1 s and 0.01. `passes` is p99 and error rate both at or under the line. `passes_with_margin` needs p99 at most 0.9 of the limit and error rate at most half, so a graze is not a recovery. It serializes by hand as `{ "p99_ms", "error_rate" }`. `over(&Point) -> Over { p99, errors }` says which limit a point is over (a NaN p99 counts as over); `passes` is neither flag set, and `sweep` stores the result on every `Point` as `over`, so a reader of the record (the CLI) does not repeat the comparison.
- `Point`: one run at one `k` (`req_s`, `requests`, `goodput_req_s`, `error_rate`, `p50_ms`, `p95_ms`, `p99_ms`, `cause`, `nodes`), with `NodePoint` per node.
- `Verdict` (tagged `kind`, snake case): `Knee`, `BrokenAtLowest`, `HoldsTo`. `req_s` is the `req_s` of the point the verdict names.
- `PointFail`: `TooLarge` (over the size caps) or `Other`.

## `sweep/search.rs` (crate private)

`search(&Ladder, &Limit, run) -> Result<Searched, String>`, where `run` returns a `Point` for a `k`.

1. **Ladder.** `lowest * step^i` for every `k <= top * (1 + 1e-9)` (default 0.25 x sqrt(2)^i, 11 points to 8.0; the tolerance keeps the top point through float rounding). Every point runs, so a recovery above the first failure is seen. A failure at the lowest point ends the search at once as `BrokenAtLowest`.
2. **Truncation.** `TooLarge` at the lowest point is an `Err` naming `k`. Above it, the ladder stops and `stopped_at` holds the refused `k`.
3. **Refine.** The first failing ladder point `k_f` and the last pass below it `k_p` are bisected in log space (`k = sqrt(k_p * k_f)`) while `k_f / k_p > resolution` (1.02). Cost: ladder size plus at most 6 runs from a 1.414 step. A `TooLarge` here ends refinement with the current `k_p`.
4. **Verdict.** `Knee` at the final `k_p`. With no failure the verdict is `HoldsTo` at the last point that ran: it means "held to the last point run", and `stopped_at` tells the reader the ladder was cut. `recovers_at` is the first ladder `k` above `k_f` that passes with margin.

`points` come back sorted by `k`, ladder and refinement together.

A ladder that cannot end is refused before any point runs: a field not finite, `lowest <= 0`, `step <= 1`, `lowest > top`, or `resolution <= 1` (bisection between adjacent floats would never stop at 1). A debug assertion checks each point reports the `k` it was asked for.

## `sweep/point.rs` (crate private)

- `floor(topo)`: ten times the largest mean `proc` (`Dist::mean`) of any node. A queue near saturation needs that long to build.
- `point_cfg(base, k, target_requests, floor)`: every stream's rate times `k`, sizes and seed unchanged; window = `target_requests / (total rate x k)` seconds, lengthened to `floor` when shorter (that only adds requests; the caps still apply); warmup = window / 9, so the run is warmup + window. A zero, negative or NaN rate gets a window past the engine's 365-day cap, so the engine names the error rather than `Duration` panicking.
- `run_point(topo, endpoints, chaos, cfg, k, limits)`: `run_with_limits`, then `summarize`; tallies `cause` over the failed requests (the most common, ties by name) before the report is dropped. `error_rate` is `failed / requests` (0 with no requests), `goodput_req_s` is `(requests - failed) / window_s`, and each `NodePoint` copies the summary's `utilization`, `saturated` and `saturates_at_req_s`. An error from the hop caps (`... too large`) is `TooLarge`, anything else `Other`.
