# Simulation

Kaanal can run your system model on a virtual clock: no real requests, no waiting. You give it a [traffic profile](traffic.md) and it tells you how the system would behave: latency percentiles, which node is the bottleneck, where queues grow, how many requests fail and why. The result is a **relative what-if, not a prediction**: it is the same cost model the mocks use, fed with a smooth Poisson stream, so use it to compare designs and find the weak node, not to promise a number.

## Run one

```bash
curl -s -X POST localhost:8080/__kaanal/sim/run \
  -H "Authorization: Bearer $TOKEN" \
  -d '{ "profile": "peak", "duration_s": 60 }'
# 202 {"id":"s1","state":"running"}   Location: /__kaanal/sim/runs/s1
curl -s localhost:8080/__kaanal/sim/runs/s1 -H "Authorization: Bearer $TOKEN"
```

The body:

| Field        | Meaning                                                                                                                      |
| ------------ | ---------------------------------------------------------------------------------------------------------------------------- |
| `profile`    | A profile name from `traffic:`, or a profile written out (`{ "rps": 50, "journeys": [ ... ] }`).                             |
| `duration_s` | Simulated seconds (default: the profile's, else 60; at most 3600).                                                           |
| `warmup_s`   | Simulated seconds before results count (default a tenth of the duration).                                                    |
| `seed`       | Seed of the draws (default the config's `seed:`). The same inputs give the same report.                                      |
| `scenario`   | Run under this [scenario](scenarios.md)'s chaos alone. Without it the run uses the **live chaos** in force when it starts.   |
| `source`     | `applied` (default) runs the system in force; `draft` runs the draft text without Apply (see below). Anything else is `400`. |

A profile name the config does not have is a `404` whose message says what to do: that no profile is defined (add a `traffic:` section or pass `-f`), or the names that are (`name one with -p: peak`); see [traffic](traffic.md).

A run is answered at once with `202`; poll `GET /sim/runs/{id}` until `state` is `done` (or `failed`, with an `error`). `GET /sim/runs` lists the last eight runs without their reports.

## Run on the draft

With `"source": "draft"` the run simulates the text of the draft (`GET /draft`) as it is now, without Apply: nothing is swapped, no counter is reset, mock traffic is untouched. The live chaos is carried onto the draft by name (what the draft no longer has is left out), and a `scenario` resolves against the draft's scenarios. A draft that does not build is `400` with `{ "error", "diagnostic" }` and does not take the run slot. The run record says `"source": "draft"` and a `draft_rev` (the draft's `rev`, which `GET /draft` shows and which counts each change of its text), so a result can be matched to the edit it describes; `generation` is the one in force.

## Limits

- **One run at a time.** A second `POST` while one runs is `409` with the running id.
- **Server caps.** A run is limited to about 2 million recorded hops and 5 million simulated hops (a couple of seconds of CPU); a bigger request is `400` and says which limit it hit. Lower the rate or the duration.
- **A run keeps the system it began on.** If you Apply a new draft while it runs, the report still describes the old one; its `generation` says which.
- The report is kept, the per-request records are not.

## Reading the report

`summary` holds:

- **`latency`**: `p50_ms`, `p95_ms`, `p99_ms`, `max_ms` and log2 buckets. Percentiles are exact over the run's requests.
- **`requests`, `failed`, `errors`**: reported requests, how many failed, and failures by kind (`status`, `reset`, `malformed`, `timeout`).
- **`nodes`**: per node, with a row per replica: `visits_per_s` (measured) beside `expected_visits_per_s` (what the chains predict), `mean_wait_ms`, `offered_load`, `utilization`, `saturated`, `max_queue`, and refusals (`limited`, `overflowed`, `timed_out`) and `short_circuited` (calls a caller's open circuit breaker refused without sending, a 503 with the cause `breaker`); `capacity_per_s` (what the node can take in **visits** per second, not requests per second: slots × replicas ÷ mean service time per visit, absent without a limit or visits), `saturates_at_req_s` (the same figure in **requests** per second for the whole traffic mix: `capacity_per_s` divided by the visits one request makes to the node; absent when there is no capacity or no request reaches the node); `headroom` (`1 - offered_load`, at least 0); and `series`: 60 windows of `busy` (fraction of slots busy), `queue` (the longest queue seen when a request joined, taken per replica, so it misses requests cancelled while queued and can read low under deadlines) and `latency` (counts of the node's own hop durations in the summary's buckets), `window_s` wide each. A run with a warmup under-counts the earliest windows. In the run log the `queued` event fires only when the wait is longer than the service time.
- **`pipes`**: per node `bandwidth` side (`send` and `receive`, only the sides with a rate): `node`, `direction`, `transfers` (bodies for which this pipe was the narrowest), `mean_wait_ms`, `offered_load` (transfer time over the window and the node's copies), `utilization`, `saturated` and `saturates_at_req_s`, as for a wire. No per-window series.
- **`links`**: per edge with a `bandwidth`: `transfers`, `series` (60 windows of `latency`: counts of the time a body spent on the wire, the transfer plus the time lost while other bodies shared the wire, in the same buckets as `latency.buckets`, by the window it joined the wire in; no busy share, because the log's records leave out the requests a full wire spends the run serving), `mean_wait_ms`, `offered_load`, `utilization`, `saturated` (offered load 1 or more, or utilization 0.95 or more) and `saturates_at_req_s` (requests per second for the whole traffic mix before this wire is full: the mix rate divided by `offered_load`; absent when nothing crossed the wire). A full wire slows every body on it, so the nodes behind it can sit idle while its transfers share it.
- **`bottlenecks`**: node ids, most loaded first (nodes with a `concurrency` or `pool` limit first, since a node with no limit cannot queue).
- **`streams`**: percentiles per endpoint.
- **`warnings`**: read these first.

**Offered load and utilization.** `offered_load` is hop time over capacity (for a node with `hold: call`, the time its slots were held, which includes the wait on what it called) and can pass 1; `utilization` is the share of slot time actually busy and tops out at 1. A node is `saturated` when its offered load is 1 or more, its utilization is 0.95 or more, or it already refuses work (a full queue or a fired `timeout:`). Its queue then grows for as long as the run lasts (with a queue limit it fills and the node turns requests away instead), so its latency is not a steady-state figure.

**Warnings about noise.** With fewer than 1000 reported requests the p99 is noise; with fewer than 100, so is p95. One run is one sample of a random process and has no confidence interval: run longer, or run twice with different seeds and compare.

## The run log

`GET /__kaanal/sim/runs/{id}/log` pages a **sample** of the run's requests, one line per hop, in virtual time (`t_ms` is the arrival plus the hop's start). It is not every request: it keeps every failed request (up to 1000), the 500 slowest, and an even spread of the rest, 4000 requests at most and 20 000 lines in all (a run that reaches the cap says so in `note`). Only requests that arrived after the warmup are logged.

Query: `node`, `from` with `to` (one wire: only the hops on the wire from one node to another, so only a wire with a `bandwidth`; one of the two alone is 400), `replica`, `endpoint` (`GET /users/{id}`, URL-encoded), `level` (`info`, `warn`, `error`; anything else is 400), `limit` (default 200, at most 1000), `after` (the `next` of the previous page). Filters combine with AND; an unknown `node` answers 200 with no lines. The answer is `{ lines, next, total, sampled: true, note }`: pass `next` as `after` to continue, `null` means the end; `total` counts the stored lines before filtering.

A line has `t_ms`, `level`, `request` (the endpoint's request counter), `stream` (the endpoint), `node`, `from` (only on a hop on a wire, `link request` or `link response`: the node that called over it, so the wire is `from` to `node`), `replica`, `event`, `wait_ms`, `service_ms`, `status`, `note` and, on a hop that ran in a retry, `attempt` (2 or more; absent in a first attempt, so it is also absent from every run without retries). `event` is `served`, `queued` (the wait was longer than the service: level `warn`), or the failure of a request, on the failing node at the instant it failed (level `error`): `failed` (chaos), `limited` (a rate limit), `overflowed` (a full queue), `timeout` (a deadline). The event comes from the simulator's own record of the cause, so a chaos 503 on a node whose full queue also answers 503 is still `failed`. The simulator does not record which replica a failure happened on, so a failure line has `replica`, `wait_ms` and `service_ms` set to `null`, and `?replica=N` skips failure lines. The log never holds half a request: when the 20 000-line cap is reached, the even spread of requests gets thinner (still across the whole run) and `note` says so.

## Sample a body

`POST /__kaanal/sample` with `{ "endpoint": "GET /users/{id}", "seed": 7 }` returns the body the mock would generate, its size in bytes and the success status, so you can see what a route carries. It is the same generator and seeds the mock uses. A route without a response schema returns `body: null`; a paged list route returns the whole schema body, not one page; a sample over 256 KiB is refused. The size is what a journey's `resp_bytes` defaults to.

## A spec for a system that has none

An `endpoints:` entry cannot create a route by itself: routes belong to a node through its OpenAPI `spec:`. If you have designed a system and its endpoints but have no spec, `GET /__kaanal/openapi/skeleton` (or `kaanal skeleton`) writes one: an OpenAPI 3.0 file with an operation for every key under `endpoints:` in the **draft**, its path parameters (`in: path`, `required: true`), a success response (the entry's `override` status, else 200) with an empty object schema, and `x-kaanal-skeleton: true`. It reads the draft, so it works before the first Apply.

```bash
kaanal skeleton -o specs/app.yaml        # refuses to overwrite without --force
# then set  nodes.app.spec: specs/app.yaml  in the draft and apply it
```

Paths are written as they are (a catch-all `{*rest}` stays one, since the router reads spec paths unchanged), repeated parameters are declared once, and a skeleton covers at most 10000 endpoints. The skeleton lists every key in `endpoints:`, because an entry does not say which node owns it; use it on a node that has no spec of its own. Payload sizes still come from the traffic profile.

## What the figures leave out

Two real limits are off unless you turn them on, so a healthy-looking run can be optimistic. With `hold: own` (the default) a request frees its slot while it waits on what it calls; real servers that block on a call (`hold: call`, Studio's Server style) keep it. And without a `bandwidth` on a node or a wire, bodies cross the network for free. When a run has components that call each other and neither is set, Results says so in a quiet grey note; set the one that fits your system and run again. A real system may saturate earlier than the run shows.

## Known limits

- **A journey is a stream of independent requests**, not a user session: no think time, no state, and Poisson arrivals are smoother than real traffic. Real tails will be worse.
- **One run is one sample.** There is no confidence interval; the report warns when the tail rests on few requests. Run longer or compare seeds.
- **No cancel.** A run is bounded to a couple of seconds of CPU by the server caps, so it ends on its own. Runs are not kept across a restart (the last eight are kept while the server runs).
- **Request bodies are not sized from the spec.** `req_bytes` is 0 unless the profile sets it. A paged list route's default `resp_bytes` is the whole schema body, not one page.
- **Samples** use the mock's generator and seeds, but a sample's data can differ from what the mock returns for the same path.
- **Skeleton specs** have empty object schemas and cover every key of `endpoints:`.
- **Not built yet:** profiles in `traffic/NAME.yaml` files (use the `traffic:` section), live load against the running mocks, and adding the generated skeleton to the draft in one step (save it with `kaanal skeleton -o` and set `spec:`).
- **Seeds.** A run uses the request's `seed`, else the config's `seed:`. Under `determinism: realistic` the live mocks use a seed drawn at startup, so a run is repeatable but its draws do not match what the mocks did.
- **Waits under timeouts are biased low.** A request that a `timeout:` cancels while it still queues never starts its hop, so it is absent from `mean_wait_ms` and `visits_per_s` on a node that sheds load. Utilization and the saturation flag do not have this bias.
- **Pins are not part of a run.** A pinned UI state (`kaanal cycle`) changes what the mocks answer, not what the simulator runs; only the chaos does.
- **Payload sizes follow the mocks, with one difference:** a `HEAD` counts as 0 bytes, as live mode charges it. A `collection:` route and a built-in responder without an `override` also count as 0 here, because a run has no stored items and does not sign a login; the live mocks size a collection route from what it holds right now and the JWKS route from its key set (a login's reply is not sized in either).
