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

| Field        | Meaning                                                                                                                    |
| ------------ | -------------------------------------------------------------------------------------------------------------------------- |
| `profile`    | A profile name from `traffic:`, or a profile written out (`{ "rps": 50, "journeys": [ ... ] }`).                           |
| `duration_s` | Simulated seconds (default: the profile's, else 60; at most 3600).                                                         |
| `warmup_s`   | Simulated seconds before results count (default a tenth of the duration).                                                  |
| `seed`       | Seed of the draws (default the config's `seed:`). The same inputs give the same report.                                    |
| `scenario`   | Run under this [scenario](scenarios.md)'s chaos alone. Without it the run uses the **live chaos** in force when it starts. |

A run is answered at once with `202`; poll `GET /sim/runs/{id}` until `state` is `done` (or `failed`, with an `error`). `GET /sim/runs` lists the last eight runs without their reports.

## Limits

- **One run at a time.** A second `POST` while one runs is `409` with the running id.
- **Server caps.** A run is limited to about 2 million recorded hops and 5 million simulated hops (a couple of seconds of CPU); a bigger request is `400` and says which limit it hit. Lower the rate or the duration.
- **A run keeps the system it began on.** If you Apply a new draft while it runs, the report still describes the old one; its `generation` says which.
- The report is kept, the per-request records are not.

## Reading the report

`summary` holds:

- **`latency`**: `p50_ms`, `p95_ms`, `p99_ms`, `max_ms` and log2 buckets. Percentiles are exact over the run's requests.
- **`requests`, `failed`, `errors`**: reported requests, how many failed, and failures by kind (`status`, `reset`, `malformed`, `timeout`).
- **`nodes`**: per node, with a row per replica: `visits_per_s` (measured) beside `expected_visits_per_s` (what the chains predict), `mean_wait_ms`, `offered_load`, `utilization`, `saturated`, `max_queue`, and refusals (`limited`, `overflowed`, `timed_out`).
- **`links`**: per edge with a `bandwidth`: transfers, wait, load.
- **`bottlenecks`**: node ids, most loaded first (nodes with a `concurrency` or `pool` limit first, since a node with no limit cannot queue).
- **`streams`**: percentiles per endpoint.
- **`warnings`**: read these first.

**Offered load and utilization.** `offered_load` is hop time over capacity and can pass 1; `utilization` is the share of slot time actually busy and tops out at 1. A node is `saturated` when its offered load is 1 or more, its utilization is 0.95 or more, or it already refuses work (a full queue or a fired `timeout:`). Its queue then grows for as long as the run lasts, so its latency is not a steady-state figure.

**Warnings about noise.** With fewer than 1000 reported requests the p99 is noise; with fewer than 100, so is p95. One run is one sample of a random process and has no confidence interval: run longer, or run twice with different seeds and compare.

## Sample a body

`POST /__kaanal/sample` with `{ "endpoint": "GET /users/{id}", "seed": 7 }` returns the body the mock would generate, its size in bytes and the success status, so you can see what a route carries. It is the same generator and seeds the mock uses. A route without a response schema returns `body: null`; a paged list route returns the whole schema body, not one page; a sample over 256 KiB is refused. The size is what a journey's `resp_bytes` defaults to.
