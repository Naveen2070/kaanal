---
title: "Simulation"
sidebar:
  order: 3
---

Kaanal can run your system model on a virtual clock: no real requests, no waiting. Given a [traffic profile](/docs/guides/simulate/traffic/), it reports latency percentiles, bottlenecks, queue growth, failures, and causes. Results are a **relative what-if, not a prediction** — the same cost model used by live mocks fed with a smooth Poisson stream.

## Run one

```bash
curl -s -X POST localhost:8080/__kaanal/sim/run \
  -H "Authorization: Bearer $TOKEN" \
  -d '{ "profile": "peak", "duration_s": 60 }'
# 202 {"id":"s1","state":"running"}, then poll the run
curl -s localhost:8080/__kaanal/sim/runs/s1 -H "Authorization: Bearer $TOKEN"
```

The `202` carries a `Location` header with the run's URL (`/__kaanal/sim/runs/s1`).

| Field        | Meaning                                                                                                       |
| ------------ | ------------------------------------------------------------------------------------------------------------- |
| `profile`    | A profile name from `traffic:`, or an inline profile (`{ "rps": 50, "journeys": [...] }`).                    |
| `duration_s` | Simulated seconds (default: the profile's, else `60`; max `3600`).                                            |
| `warmup_s`   | Simulated seconds before results count (default 1/10 of `duration_s`).                                        |
| `seed`       | RNG seed (defaults to `seed:` in config). Same inputs produce the same report.                                |
| `scenario`   | Run under this [scenario](/docs/guides/break/scenarios/)'s chaos alone; otherwise uses active **live chaos**. |
| `source`     | `applied` (default) runs the system in force; `draft` runs the draft text without Apply (`400` otherwise).    |

An unknown profile returns `404` with guidance (`add traffic:` / `-f`, or `name one with -p: peak`). Runs answer `202` immediately; poll `GET /sim/runs/{id}` until `state` is `done` or `failed`. `GET /sim/runs` lists the last eight runs without reports.

## Live and batch

- **Batch run**: simulates a seeded stretch of virtual time at once so two designs compare repeatably.
- **Live simulation** (Studio's default): runs the same engine on the wall clock (`cpu`, slots, queues, retries, breakers, balancing, `detect`, `from_ms`), emitting 250 ms counts and a 1%-precision latency histogram ([Control API](/docs/reference/control-api/live-and-events/#the-live-simulation-get-__kaanallive-and-its-commands)).

## Run on the draft

With `"source": "draft"`, the simulator runs `GET /draft`'s text without Apply (no swap, no counter reset, mock traffic untouched). Live chaos carries over by node/endpoint name, and `scenario` resolves against the draft's scenarios. Invalid drafts return `400` (`{ "error", "diagnostic" }`) without taking the run slot. The run record sets `"source": "draft"` and `draft_rev`.

## Load sweep: where does it break?

A **load sweep** finds how much traffic the design holds before missing a target. It steps **load factors** `k` (`0.25x, 0.35x, 0.5x, ...` up to `max_factor`, `×1.414` per step), checks each point against a **limit** (`p99_ms` and `error_pct`), and bisects between the last passing and first failing points until within `2%`. The last passing `k` is the **knee**.

```bash
curl -s -X POST localhost:8080/__kaanal/sim/sweep \
  -H "Authorization: Bearer $TOKEN" \
  -d '{ "profile": "peak", "p99_ms": 500, "error_pct": 1, "max_factor": 8 }'
# 202 {"id":"s2","state":"running"}
```

The `202` carries a `Location` header with the run's URL (`/__kaanal/sim/runs/s2`).

| Field        | Meaning                                                                                  |
| ------------ | ---------------------------------------------------------------------------------------- |
| `profile`    | Profile name or inline object. Defaults to the first `traffic:` profile (`400` if none). |
| `p99_ms`     | Max passing p99 latency in ms; `> 0` (default `1000`).                                   |
| `error_pct`  | Max passing error share in percent, `0`–`100` (default `1`).                             |
| `max_factor` | Top of the ladder, `1`–`64` (default `8`; `400` outside).                                |
| `seed`       | Same seed at every point so points differ only in load (default `seed:`).                |
| `scenario`   | Sweep under this scenario alone; otherwise uses active live chaos.                       |

Sweeps always run the **applied** system (`source`, `duration_s`, and `warmup_s` return `400`): each point sizes its window to ~20,000 reported requests (at least `10×` the slowest node's mean `proc`). It shares the single run slot (`409` if busy).

### Sweep record fields (`GET /sim/runs/{id}`)

A sweep is a run record with `"kind": "sweep"` (a plain run says `"run"`), under the same single slot: a sweep while a run runs, or the other way round, is `409` with the running id. Poll `GET /sim/runs/{id}`: `sweep_points` grows per point; once `done`, `sweep` holds the result and `sweep_points` is emptied (`sweep.points` has every point):

| Field                | Contents                                                                                                                                                                                                      |
| -------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `verdict`            | `{ "kind": "knee", "k", "req_s" }`, `broken_at_lowest` (failed at `0.25x`), or `holds_to` (no failure up to the last point run).                                                                              |
| `seed`, `max_factor` | Recorded in the sweep record.                                                                                                                                                                                 |
| `fingerprint`        | 12 hex chars over design files.                                                                                                                                                                               |
| `chaos_digest`       | 12 hex chars over compiled chaos.                                                                                                                                                                             |
| `inline_profile`     | Optional.                                                                                                                                                                                                     |
| `live_chaos`         | `{ scenarios, entries, digest }` when no `scenario` was passed.                                                                                                                                               |
| `runtime_scenarios`  | Recorded in the sweep record.                                                                                                                                                                                 |
| `limit`              | `{ "p99_ms", "error_rate" }` (`error_rate` as a fraction, e.g. `0.01`).                                                                                                                                       |
| `points`             | Sorted by `k`, with `req_s`, `requests`, `goodput_req_s`, `error_rate`, `p50_ms`, `p95_ms`, `p99_ms`, `cause`, `over` (`{ "p99", "errors" }`), and per-node `utilization`, `saturated`, `saturates_at_req_s`. |
| `first_to_saturate`  | Node saturating at the lowest `k` with its analytic `saturates_at_req_s` (note: the knee can still come from another limit).                                                                                  |
| `dominant_cause`     | Most common failure cause at the first point over the limit.                                                                                                                                                  |
| `recovers_at`        | A higher `k` that passes again with margin (e.g. retry storm or breaker recovery).                                                                                                                            |
| `stopped_at`         | `k` where the ladder stopped because a point was too large (`holds_to` then means `"held to"` the last point run).                                                                                            |
| `warnings`, `label`  | Sweeps have no `/log` (`409`).                                                                                                                                                                                |

### Sweep CLI

```bash
kaanal sim sweep -p peak --p99 500ms --errors 1% --max-factor 8
```

| Flag                      | Meaning                    | Default |
| ------------------------- | -------------------------- | ------- |
| `--p99`                   | Max passing p99 latency.   | `1s`    |
| `--errors`                | Error limit (`1%` or `1`). | `1%`    |
| `--max-factor`            | Top of the load ladder.    | `8`     |
| `--seed`                  | RNG seed.                  |         |
| `--scenario`              | Sweep under this scenario. |         |
| `-f FILE`                 | Inline profile.            |         |
| `--no-wait`               | Do not wait for the run.   |         |
| `--report FILE [--force]` | Write the Markdown report. |         |

Invalid flags exit `64`; a bad `--p99` or `--errors` is a usage error before anything is sent. `--p99` is a duration (`500ms`, `1s`), `--errors` a percentage from `0` to `100`.

The CLI prints:

- **Verdict**: for example `knee at 94 req/s (1.88x), the last point within the limit; the next point is over: p99 over 500ms`, or where it broke at the lowest point, or how far it held. A cut ladder says `held to N req/s, the last point run`, not `holds to`.
- **Detail**: the first node to saturate with its analytic figure, the dominant cause, the warnings, and the label.
- **Cut-ladder warnings**: `ladder stopped at 5.657x (283 req/s): run too large` when a point was too big to run, and `ladder cut at ...` at the engine's rate cap.
- **Table**: one row per point (`load req/s`, `goodput`, `p99`, `errors`, `ok`). `--no-wait` prints the id.

- `kaanal sim list` shows `kind` (`run` or `sweep`); `kaanal sim show ID` prints `seed`, `fingerprint`, and `chaos in force` (`--json` prints the record).
- `kaanal sim report ID` prints Markdown (`--json` prints the data block; `-o FILE [--force]` writes to disk).
- **Calibrate inputs**: if `proc`, slots, `cpu`, or bandwidth are guesses, the knee is a guess too.

## Sharing a sweep

A finished sweep produces one deterministic **Markdown report** (identical bytes from the server, CLI, and Studio):

| Section | Contents                                                                                                                                                                                                                                                        |
| ------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Label   | _relative what-if, not a prediction_.                                                                                                                                                                                                                           |
| Finding | Verdict, limit, first node to saturate, dominant cause, **chaos in force**, **chaos digest**, and warnings.                                                                                                                                                     |
| Curve   | Table of `load req/s`, `goodput`, `p99 ms`, `errors`, and `ok`.                                                                                                                                                                                                 |
| Re-run  | Exact `kaanal sim sweep ...` command (`-f profile.yaml` for inline profiles), seed, design fingerprint, and Kaanal version.                                                                                                                                     |
| Data    | Fenced JSON block (`report_version`, `label`, `id`, `finding`, `points`, `recipe`). `recipe.chaos` is `none`, `scenario`, or `live`; `recipe.live_chaos`, `recipe.chaos_digest`, and `recipe.runtime_scenarios` are the record's. Never embeds the design file. |

Sweeps under live chaos or runtime-defined scenarios note in **Re-run** that the command alone cannot reproduce them (use the chaos digest to verify setup). A sweep under a runtime scenario keeps `--scenario=NAME` in the command, but no file holds that scenario and after a restart the name is unknown.

### Design fingerprint & chaos digest

- **Design fingerprint**: 12 hex digits of SHA-256 over canonicalized config, `spec:` files, `endpoints:`, `scenarios/*.yaml`, `locales/*.yaml`, and `data:` / `data.yaml` (excludes `site/`). Ignores comments, whitespace, and key order; computed on Apply from the exact texts read (unapplied/live-only edits do not change it).
- **Chaos digest**: 12 hex digits over the compiled chaos in force (scenarios + live entries, independent of node order or live-entry insertion order).
- **Same fingerprint + same chaos digest + same seed = same numbers.**

```bash
kaanal sim report s2                         # Markdown on stdout
kaanal sim report s2 --json                  # JSON data block alone
kaanal sim report s2 -o sweep.md             # write file (--force to overwrite)
kaanal sim sweep -p peak --report sweep.md   # sweep and write report
curl -s localhost:8080/__kaanal/sim/runs/s2/report -H "Authorization: Bearer $TOKEN"
```

Over HTTP, `GET /__kaanal/sim/runs/{id}/report` returns `text/markdown` (or `?format=json`), `404` if unknown/evicted, and `409` while running, failed, or for a plain run. See [Control API](/docs/reference/control-api/).

### Sweeps in Studio

Under Results -> **Find where it breaks**, set p99 (ms) and error (%) limits and click **Start** (disabled in Live mode; switch to Batch). Studio plots a log-log curve, limit line, knee marker, and error strip, and keeps the fetched report for **Copy as Markdown** / **Save as a file** (`kaanal-sweep-<id>.md`). Sweeps from an earlier Apply or server boot are flagged as outdated.

## Sizing a design: what would hold this load?

A **sizing plan** answers _"what do I change so it holds N× traffic, and what does that cost?"_ (a relative what-if, never auto-applied; produces a **proposed draft**).

```bash
kaanal sim plan -p peak --target 3x --report plan.md -o plan.yaml
```

| Part                     | What it does                                                                                                                                                                                                                                                                            |
| ------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Target and margin        | `--target` takes a multiple (`3x`, `0.1`–`64`) or rate (`500` req/s). A design **holds** only with margin: `p99 <= 90%` of `--p99` (default `1s`) and `errors <= 50%` of `--errors` (default `1%`).                                                                                     |
| Walk                     | Runs one check at the target load and steps the first saturating node up one size step (details below).                                                                                                                                                                                 |
| Menu and Cannot hold     | Each step records alternative moves tried without extra runs. If failures come from user-set rate limits, chaos, deadlines without saturation, or unsizable nodes (`external-api`, `storage`, `--keep`), the plan stops with `cannot_hold` (`node` may be `null` when not single-node). |
| Budget (`--budget 2.00`) | Dollars/hour across **priced nodes only** (includes per-use prices on the target re-check; `budget_check` reports if per-use fees pushed a holding plan over budget). In JSON, `micro_usd_hr` is integer millionths of a dollar per hour.                                               |
| Database family what-if  | Separate section testing relational vs `mongodb`/`dynamodb`/`cassandra` on preset ops for databases without custom `ops:` (not kept, assumes app compatibility, not a migration recommendation).                                                                                        |

The **Walk** in detail:

- **Size steps**: `instance` class in the same series of the region's price file (`m5.large` to `m5.xlarge`), `cpu.cores` on un-instanced nodes, `memory` only on `oom` failures, or `replicas`; caches get copies only.
- **Never altered**: node kind, region, wires, rate limits, timeouts, breakers, or retries. Protect nodes with `--keep NODE`.
- **Stops when**: the design holds, the node is unsizable, the plan is over `--budget` (when a priced step adds cost), two steps make no progress, or `--max-steps` (`1`–`100`, default `30`) is reached.

### Cheapest search & provider comparison

`--cheapest` (`cheapest: true` over HTTP) searches for smaller holding sizes instead of walking up (`"cheapest found within the limit, not proven cheapest"`, assumes bigger sizes never hold less). A saving under a cent prints as it is (`$0.0035/h`), not as `$0.00`:

| Flag          | Range or value               | Default                                             |
| ------------- | ---------------------------- | --------------------------------------------------- |
| `--time`      | `1s`–`600s`                  | `150s`                                              |
| `--run-cap`   | `1`–`2000` (runs per search) | `200`                                               |
| `--providers` | `gcp,azure` or `none`        | Other providers when the design uses a cloud class. |

Time bounds the searches only (split half to main, half across provider sections; re-check and verify sweep run after). No `families` section in cheapest mode.

**Provider translation**: compute classes move across AWS, Azure, and GCP. Managed databases only move where the table has a managed class:

| Database                                                     | Behaviour                            |
| ------------------------------------------------------------ | ------------------------------------ |
| `sqlserver`                                                  | Moves between AWS RDS and Azure SQL. |
| `postgres`, `mysql`, `mariadb`, `oracle`, `mongodb`, `neo4j` | Stay on AWS.                         |
| `cassandra`, `dynamodb`                                      | Have no managed class.               |

Kept cloud nodes or cloud caches skip mixed-provider sections.

- **Verification**: every proposed draft is rebuilt from its own text, checked at target, and swept once for its measured knee. Shares the single run slot (`POST /__kaanal/sim/plan`, `GET /__kaanal/sim/runs/{id}/draft`).
- **In Studio**: **Plan** button in the top bar (works in every mode; plans the applied design). Shows live steps, verdicts, menus, provider comparison, and **Load proposed draft** (asks before replacing a dirty draft, with **Copy my draft**).

## Limits

- **One run at a time**: concurrent runs/sweeps/plans return `409` with the active id.
- **Server caps**: capped at ~2 million recorded hops and 5 million simulated hops (returns `400` naming the cap).
- **Generation pinning**: a running job finishes against the generation it started on.
- Per-request records are not retained across runs; only the last eight run reports are kept in memory.

## Reading the report

`cost` reports hourly estimates (`total_micro_usd_hr`, `priced`/`unpriced` counts, per-node `unit_micro_usd_hr`, `copies`, `instance`, `why` if unpriced, `regions`, and `egress`). `summary` holds:

| Key                                  | Contents                                                                                                                                                                                                                                                                                                                         |
| ------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `latency`                            | `p50_ms`, `p95_ms`, `p99_ms`, `max_ms`, and log2 buckets.                                                                                                                                                                                                                                                                        |
| `requests`, `failed`, `errors`       | Counts and failures by kind (`status`, `reset`, `malformed`, `timeout`).                                                                                                                                                                                                                                                         |
| `nodes`                              | Per node and replica: `visits_per_s`, `expected_visits_per_s`, `mean_wait_ms`, `offered_load`, `utilization`, `saturated`, `max_queue`, `limited`, `overflowed`, `timed_out`, `short_circuited`, `capacity_per_s`, `saturates_at_req_s`, `headroom`, and 60-window `series` of `busy`, `queue`, `latency`, each `window_s` wide. |
| `pipes`, `links`                     | Per-node `bandwidth` pipes (one per side with a rate, `send`/`receive`, named by `direction`; no series) and per-edge `bandwidth` links (`transfers`, 60-window `latency` series in the same buckets as `latency.buckets`, `mean_wait_ms`, `offered_load`, `utilization`, `saturated`, `saturates_at_req_s`).                    |
| `bottlenecks`, `streams`, `warnings` | Ranked node ids, per-endpoint percentiles, and noise warnings (`< 1000` requests makes p99 noisy; `< 100` makes p95 noisy).                                                                                                                                                                                                      |

- **Saturation & CPU**: `saturated` triggers at `offered_load >= 1`, `utilization >= 0.95`, `cpu_load >= 1`, or any refusal/timeout. `cpu` adds `cpu_utilization`, `cpu_load`, `cpu_capacity_per_s`, and `saturates_by` (`slots` or `cpu`); see [CPUs](/docs/reference/resources/#cpus-cpu).

## The run log

`GET /__kaanal/sim/runs/{id}/log` pages a **sample** of post-warmup hops (all failures up to `1000`, `500` slowest, and an even spread up to `4000` requests / `20,000` lines).

| Filter       | Notes                                                    |
| ------------ | -------------------------------------------------------- |
| `node`       |                                                          |
| `from`, `to` | Both required for a bandwidth wire.                      |
| `replica`    | `?replica=N` skips failure lines (they have no replica). |
| `endpoint`   | URL-encoded `GET /users/{id}`.                           |
| `level`      | `info`, `warn`, `error`.                                 |
| `limit`      | Default `200`, max `1000`.                               |
| `after`      | The `next` cursor.                                       |

- **Line fields**: `t_ms`, `level`, `request`, `stream`, `node`, `from` (on wire hops), `replica`, `event` (`served`, `queued`, `failed`, `limited`, `overflowed`, `timeout`), `wait_ms`, `service_ms`, `status`, `note`, and `attempt` (`>= 2` on retries). Failure lines have `null` for `replica`, `wait_ms`, and `service_ms`.

## Sample a body & generate a skeleton spec

- **`POST /__kaanal/sample`**: `{ "endpoint": "GET /users/{id}", "seed"?: 7 }` returns the mock body, byte size, and status (`<= 256 KiB`; paged routes return the full schema body).
- **`GET /__kaanal/openapi/skeleton`** (`kaanal skeleton -o specs/app.yaml`): generates an OpenAPI 3.0 file from the **draft's** `endpoints:` (up to `10,000` endpoints, `x-kaanal-skeleton: true`). Paths are written as they are (a catch-all `{*rest}` stays one), and repeated parameters are declared once.

## What the figures leave out

With `hold: own` (default) and no `bandwidth`, slots free during downstream calls and network transfers are free. Studio notes this when both are off; enable `hold: call` or `bandwidth` where appropriate.

## Known limits

| Limit                     | Detail                                                                                                                                       |
| ------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------- |
| Independent arrivals      | Poisson streams have no think time or user sessions; real traffic bursts harder.                                                             |
| Single sample             | One run has no confidence interval; run longer or compare multiple seeds.                                                                    |
| No cancel and ephemeral   | Runs end within CPU caps; only the last eight runs stay in memory while running.                                                             |
| Request and special sizes | `req_bytes` is `0` unless set; paged routes size the full schema; `collection:` and built-in auth routes size as `0` in sim (`HEAD` is `0`). |
| Samples and skeletons     | `sample` data can differ from path-seeded mock responses; skeletons emit empty object schemas.                                               |
| Not built yet             | `traffic/NAME.yaml` files, live load against running mocks, and one-step skeleton attachment.                                                |
| Realistic determinism     | Under `determinism: realistic`, live mocks draw a startup seed so sim draws differ from live mocks.                                          |
| Timeout wait bias         | Requests cancelled while queued are omitted from `mean_wait_ms` and `visits_per_s` (utilization and `saturated` remain unbiased).            |
| Pins                      | `kaanal cycle` pins affect live mocks, not simulator runs (chaos applies to both).                                                           |
