# Concepts

What Kaanal actually simulates, in plain terms.

## Topology

Your system is a graph of **nodes** (`gateway`, `service`, `database`) connected by **edges** (dependencies). The graph must be acyclic and have exactly one gateway; anything Kaanal can't simulate is rejected when the file loads, with a message naming the problem — never silently ignored.

Phase one walks `gateway → service → database`. More node kinds (cache, external API, auth) are on the roadmap.

## Call chains

A request to a mocked endpoint doesn't return instantly. Kaanal walks the path _behind_ that endpoint and adds up the simulated cost of each hop:

```
GET /users/42
  nginx.proc   1.0 ms    gateway processing
  net          1.0 ms    gateway → service
  app.proc     5.0 ms    service processing
  net          1.0 ms    service → database
  db.read      4.2 ms    database read (sampled from your p50/p99)
  ─────────────────────
  total       12.2 ms    ← the client really waits this long
```

The breakdown comes back in the `X-Kaanal-Trace` response header, and the total is what your frontend experiences — so loading states, spinners and timeouts behave like they will in production.

Costs can be fixed (`5ms`), long-tailed (`{ p50: 4ms, p99: 20ms }` — most reads are ~4 ms, one in a hundred is ~20 ms, capped at 10×p99) or exponential (`{ mean: 5ms }`, as in queueing theory, capped at 10×mean).
A node may also set `concurrency: N` (how many requests it serves at once, the rest queue). The simulator honours it; live mode warns and ignores it until M1.

## Operations

Each database hop is one of three operations, inferred from the endpoint: `GET /users/{id}` → **read** (one row), `GET /users` → **scan** (a list), everything else → **write**. Each has its own cost in the node's `ops`.

## Chaos

Chaos is live, per-node runtime state — never part of the config file:

- **latency**: added to every visit of the node.
- **error rate + status**: each visit fails with that probability and HTTP status.
- **down**: every visit fails 503.

A failure stops the chain at that node and its status becomes the response — a database failure surfaces as a 5xx at the gateway, after the time already spent. That's how cascading failures look in real systems.

## Determinism

Given the same `seed` in `kaanal.yaml`:

- The same URL always returns the same generated body (`GET /users/42` is always the same Priya).
- A sequence of requests sent one after another always sees the same timings and the same failure pattern.

This makes tests reproducible: commit the config, and every CI run gets the same backend. (Under _concurrent_ requests, arrival order decides which specific request fails; only the failure _rate_ is guaranteed.)

## Generated data

Bodies come from your OpenAPI response schemas, chosen by field meaning: `email` fields get emails, `pincode` gets pincodes, `price` gets plausible prices, `createdAt` gets recent timestamps, and fields in one object agree with each other (city, state and pincode always describe the same place). `example` values in the spec are used verbatim. Static `routes:` overrides beat generation entirely.

All data is synthetic; emails use `example.com`.

## What Kaanal is not (yet)

- It does not speak real database protocols — the database is a behavioural model (timing, failures), not Postgres. Nothing outside Kaanal connects to it.
- Timings are modelled from your configured numbers, not measured from a real system. Treat them as realistic estimates, not predictions.
