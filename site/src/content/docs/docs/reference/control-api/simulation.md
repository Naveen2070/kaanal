---
title: "Simulation"
sidebar:
  order: 6
---

Run the simulator on the applied system and read the report. See [Simulation](/docs/guides/simulate/simulation/) for the body, limits, and how to read a summary, and [Traffic](/docs/guides/simulate/traffic/) for profiles.

## `POST /__kaanal/sim/run`

Starts a simulation run.

- **Body**: `source` is `applied` (default) or `draft` (the draft text without Apply; its own `scenarios:` count).
- **202**: `{ "id", "state": "running" }` with `Location: /__kaanal/sim/runs/{id}`.
- **400**: bad body, a profile naming an unknown endpoint, a request over the server caps (the message names the cap), or a draft that does not build (`{ "error", "diagnostic" }`).
- **404**: unknown profile or scenario.
- **409**: `{ "error", "id" }` while another run is in progress, or no system loaded (blank start; not for `source: "draft"`).
- **429**: while two draft builds, samples, or skeletons are in progress.

## `POST /__kaanal/sim/sweep`

Starts a load sweep of the system in force. See [Simulation](/docs/guides/simulate/simulation/#load-sweep-where-does-it-break).

- **Body**: `{ "profile"?, "seed"?, "scenario"?, "p99_ms"?, "error_pct"?, "max_factor"? }`
  - `profile`: defaults to the first `traffic:` profile (`400` when there is none).
  - `p99_ms`: above `0` (default `1000`).
  - `error_pct`: `0` to `100` (default `1`).
  - `max_factor`: `1` to `64` (default `8`).
  - `source`, `duration_s`, and `warmup_s` are refused (`400`, unknown field).
- **202**: `{ "id", "state": "running" }` with `Location: /__kaanal/sim/runs/{id}`.
- **400**: bad body or limit.
- **404**: unknown profile or scenario.
- **409**: `{ "error", "id" }` while a run or sweep is in progress, or no system loaded.

## `POST /__kaanal/sim/plan`

Starts a sizing plan of the system in force.

- **Body**: `{ "target", "profile"?, "seed"?, "scenario"?, "p99_ms"?, "error_pct"?, "budget_per_hour"?, "keep"?, "max_steps"?, "cheapest"?, "time_limit_s"?, "run_cap"?, "providers"? }`
  - `target`: `{ "multiple": 0.1 to 64 }` or `{ "req_s" }`.
  - `budget_per_hour`: dollars an hour of the priced nodes. Counts per-use prices at the visits of the target run; the verdict is re-judged from the re-check's cost, and `budget_check` (`budget_micro_usd_hr`, `cost_micro_usd_hr`, `rejudged`) reports whether those prices turned `holds` into `over_budget`.
  - `keep`: node names the plan must not move.
  - `max_steps`: `1` to `100` (default `30`). A database-family walk takes at most `10` steps.
- **Cheapest mode (`"cheapest": true`)**: searches for the cheapest holding design found (may shrink nodes; never moves `keep` nodes) instead of the walk, and adds a provider section per other provider:
  - `time_limit_s`: `1` to `600` (default `150`). Bounds the searches only, split between them (half to the main search when a section runs, the rest equally, checked between runs); the re-check, section pricing runs, and sweep run after it.
  - `run_cap`: `1` to `2000` (default `200` runs per search).
  - `providers`: list of `aws`, `gcp`, `azure`, or `["none"]` (defaults to every other provider when the design has a provider class).
  - Without `cheapest: true`, `time_limit_s`, `run_cap`, and `providers` return `400` (`"needs cheapest"`), as do out-of-range bounds, unknown/repeated providers, or a provider the design already uses.
  - Extra `plan` fields when `cheapest` runs: `cheapest_note`, `search` (`stop`: `exhausted` | `run_cap` | `time_limit`, `runs`, `elapsed_ms`, `limit_ms`, `run_cap`, `design_in_force_holds`, `notes`), `comparison`, `providers` (`answer` or `skipped`), and `request.cheapest` (`families` is `[]`; `proposed_draft` is the main search's design).
- **202**: `{ "id", "state": "running" }` with `Location: /__kaanal/sim/runs/{id}`. Record has `kind: "plan"`, live `plan_steps`, and once `done`, `plan` and `proposed_draft` (both stripped from `GET /__kaanal/sim/runs`).
- **400**: bad body, target, limit, budget, `max_steps`, or `keep` name.
- **404**: unknown profile or scenario.
- **409 / 429** (checked in order): `409 { "error", "id" }` while a run, sweep, or plan is in progress; `429` while two draft builds, samples, or skeletons are in progress; `409` with no applied text or when applied text is not the design in force (Apply first); `409` with no system.

## `GET /__kaanal/sim/runs`

Returns `{ "runs": [...] }`: the last eight runs (oldest first) without `summary`, `sweep_points`, `sweep`, `plan_steps`, `plan`, or `proposed_draft`.

## `GET /__kaanal/sim/runs/{id}`

Returns one run record (`404` once pushed out of the last eight).

- **Common fields**: `kind` (`run`, `sweep`, or `plan`), `state` (`running`, `done`, `failed`), `generation`, `source`, `draft_rev` (for a draft run, else `null`), `profile`, `scenario`, `started_ms`, `finished_ms`, and `summary` or `error`.
- **Sweeps (`kind: "sweep"`):** omits `summary`/`cost`. Includes `sweep_points` (grows while running, cleared when `done`), `seed`, `max_factor`, `fingerprint`, `chaos_digest`, optional `live_chaos` / `runtime_scenarios` / `inline_profile`. Once `done`, `sweep` holds `label`, `limit`, `points`, `verdict`, `recovers_at`, `stopped_at`, `first_to_saturate`, `dominant_cause`, and `warnings`.
- **Plans (`kind: "plan"`)**: no `summary`. Includes `plan_steps` (omitted while empty), `seed`, `fingerprint`, `chaos_digest`, and once `done`, `proposed_draft` and `plan` (`request`, `start`, `steps` with `menu`, `end`, `verdict`, `families`, `unpriced_nodes`, `verify`, `measured_knee`, `price_before`, `price_after`, and `words`).
  - `verdict.kind`: `holds`, `already_holds`, `cannot_hold` (`node` nullable + `reason`), `over_budget` (`over_by_micro_usd_hr` + `holds` boolean), `no_progress` (`node`), or `step_cap`.
  - Money values are in `micro_usd_hr` (millionths of a dollar an hour).
  - `409` on the report or draft of a failed plan.

## `GET /__kaanal/sim/runs/{id}/log`

Returns a page of the run's log sample: `{ "lines", "next", "total", "sampled": true, "note" }`.

- **Query params**: `node`, `replica`, `endpoint` (`METHOD /path`), `level` (`info`, `warn`, `error`), `request` (per-stream counter; pair with `endpoint`), `limit` (default `200`, max `1000`), `after` (index of last seen line; `next` is `null` at the end).
- **Line shape**: `t_ms`, `level`, `request`, `stream`, `node`, `replica`, `event`, `wait_ms`, `service_ms`, `status`, `note`. (`replica`, `wait_ms`, and `service_ms` are `null` on failure lines, so `replica` filters skip them.) `total` counts stored lines before filters.
- **Errors**: `400` bad `level` or `endpoint`; `404` unknown run; `409` while running, after failure, or on a sweep/plan.

## `GET /__kaanal/sim/runs/{id}/report`

Returns a finished sweep's or plan's report (deterministic bytes on every fetch).

- **Formats**: `200 text/markdown; charset=utf-8` (default, ending in a re-run recipe and a `## Data` JSON block) or `?format=json` (`200 application/json`).
- **Plan reports**: state first that they are not predictions, then target, design in force, steps, per-step menu, database-family what-ifs, plain-text verdict, proposed-draft re-check, and measured knee. Cheapest plans add `"cheapest found within the limit, not proven cheapest"`, `## Search`, `## Comparison`, per-change price effects, and per-provider sections.
- **Errors**: `400` bad `format` or extra query key; `404` unknown/evicted id; `409 { "error" }` while running, failed, or for a plain run; `500 { "error" }` if a done record is missing a required sweep/plan field.

## `GET /__kaanal/sim/runs/{id}/draft`

Returns a finished plan's proposed `kaanal.yaml` draft as `200 text/plain; charset=utf-8` (comments and layout preserved; never applied by Kaanal).

- **Errors**: `404` unknown/evicted id; `409 { "error" }` if not a plan, still running, or failed.

## `POST /__kaanal/sample`

Generates one response body using the mock generator: `{ "endpoint": "GET /users/{id}", "seed"?: 7 }` → `{ endpoint, status, body, bytes, label }`.

- **Errors**: `400` over 256 KiB; `404` unknown route; `409` no system loaded.

## `GET /__kaanal/openapi/skeleton`

Returns `{ "text", "endpoints" }`: a generated OpenAPI 3.0 file with one operation per key of the **draft's** `endpoints:` (inline or external file inside the project).

- **Errors**: `400` draft does not parse or endpoints file is outside the project/unreadable; `409` no endpoints, over 10,000 endpoints, invalid `METHOD /path` key, or malformed `{parameter}`.
