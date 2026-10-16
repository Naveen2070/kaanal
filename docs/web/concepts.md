# Concepts

What Kaanal actually simulates, in plain terms.

## Topology

Your system is a graph of **nodes** (`gateway`, `service`, `monolith`, `database`, `cache`, `external-api`, `auth`) connected by **edges** (dependencies). The graph must be acyclic and have at most one gateway and one auth node; anything Kaanal can't simulate is rejected when the file loads, with a message naming the problem — never silently ignored.

Requests enter at the gateway, or at the service that owns the route when there is none, then follow the endpoint's chain (by default every node reachable from the owner); `POST /login` and the JWKS enter at the auth node (after the gateway, if any). A config with only an auth node is valid. See the [endpoints reference](endpoints.md) to change it.

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

Calls inside a `parallel` block run side by side, so the block costs as much as its slowest branch; a cache hit skips the rest of the sequence it is in. The breakdown comes back in the `X-Kaanal-Trace` response header, and the total is what your frontend experiences — so loading states, spinners and timeouts behave like they will in production.

Costs can be fixed (`5ms`), long-tailed (`{ p50: 4ms, p99: 20ms }` — most reads are ~4 ms, one in a hundred is ~20 ms, capped at 10×p99) or exponential (`{ mean: 5ms }`, as in queueing theory, capped at 10×mean).
A node may also set `concurrency: N` (how many requests it serves at once, the rest queue). See [capacity.md](capacity.md) for queues, pools, rate limits and 429s. Live mode queues requests the same way the simulator does: while a node is full, a request waits and its trace shows `node.wait=`.

## Operations

Each database hop is one of three operations, inferred from the endpoint: `GET /users/{id}` → **read** (one row), `GET /users` → **scan** (a list), everything else → **write**. Each has its own cost in the node's `ops`. The operation flows down the chain: an endpoint's `op:` applies to every database step below it, `op:` on a single step changes just that step, and `via:` uses the op of the endpoint it points at.

## Chaos

Chaos is live runtime state (per node, per endpoint, or per node-at-endpoint). You set it by hand, or switch on a named [scenario](scenarios.md); neither is part of a node's configuration:

- **latency**: added to every visit of the node.
- **error rate + status**: each visit fails with that probability and HTTP status.
- **down**: every visit fails 503.
- **kind**: how an error shows up. `status` (default) answers with the HTTP status; `reset` sends the start of the response and then drops the connection; `malformed` sends the right status with a body that is not valid JSON; `hang` never answers and ends as a 504 after the node's `timeout` (60 s without one).

A failure stops the chain at that node and its status becomes the response — a database failure surfaces as a 5xx at the gateway, after the time already spent. That's how cascading failures look in real systems.

**Where the failure happens matters.** At the gateway or the service that owns the route, `reset` and `malformed` reach your client as they are. Deeper (a database, a cache call, an external API) they become a 502, and a timeout a 504, unless the endpoint's `error_map` says otherwise. `error_map` keys can be a node name, a kind (`reset`, `malformed`, `timeout`), a status or a class like `5xx`, tried in that order. A cache never fails a request: a failing cache is a miss.

**Timeouts.** Any node can have `timeout: 1s`. It is a deadline for calls to that node, covering its queue wait, its own work and everything it calls. When it passes, the caller gets a 504 (the error body has `"kind":"timeout"`) at that moment — but the work that was already running keeps its slot until it ends, as a real database does. A slow dependency therefore turns into a pile of timeouts, not a clean recovery.

**Chaos on one route.** Chaos can be set on a node, on an endpoint (`GET /orders`, once at its front door) or on a node only while it serves one endpoint. Entries that match a visit **stack**: latencies add (clamped to 5 minutes per entry key: the node, endpoint and node-at-endpoint entries each clamp on their own), `down` is an OR, and error rates combine as independent chances. Each failure keeps its own kind and status, which belong to the entry's own `error_rate` (a kind with no rate in the same entry does nothing and is reported as a warning). Two failing entries are drawn from one roll, so 30% `reset` plus 30% `503` gives about 30% resets and 21% 503s. Entries come in layers, bottom to top: the active scenarios in the order you activated them, then the chaos you set by hand, always on top. **A higher layer wins over a lower one** for the same field, whatever the key; inside one layer the node-at-endpoint entry goes before the node's. A hand-set entry with `mode: override` replaces everything below it for that key instead (see the [control API](control-api.md)). An endpoint entry is separate: it is rolled once at the endpoint's front door (its latency is a leading delay on the entry node, its failure belongs to the entry node) and is not merged into the node visits behind it. See [chaos internals](../source/chaos.md). Stacking differs from service meshes, where the most specific rule replaces the others; to get "only this", clear the broader entry, or use `mode: override`.

**Scenarios.** A scenario is a named bundle of chaos entries from `kaanal.yaml` or `scenarios/NAME.yaml`: "payments is down", "slow database". Switch them on and off live, stack several, save what you built by hand, and reset everything between tests. See [scenarios.md](scenarios.md).

## Determinism

Given the same `seed` in `kaanal.yaml`:

- The same URL always returns the same generated body (`GET /users/42` is always the same Priya).
- A sequence of requests sent one after another always sees the same timings and the same failure pattern.

This makes tests reproducible: commit the config, and every CI run gets the same backend. (Under _concurrent_ requests, arrival order decides which specific request fails; only the failure _rate_ is guaranteed.)

By default (`determinism: controlled`) this holds on every run. With `determinism: realistic`, timings and failures vary from run to run: `kaanal up` draws a fresh seed at startup and prints it (`Seed:  N (determinism: realistic; replay its timings and failures with determinism: controlled and seed: N; generated data then follows that seed too)`). To replay that run's timings and failures, set `determinism: controlled` and `seed: N`. Generated bodies follow `seed:` in both modes, so replaying with a new `seed:` also changes the generated data, while the chain's timings and failures repeat exactly.

## Generated data

Bodies come from your OpenAPI response schemas, chosen by field meaning: `email` fields get emails, `pincode` gets pincodes, `price` gets plausible prices, `createdAt` gets recent timestamps, and fields in one object agree with each other (city, state and pincode always describe the same place). `example` values in the spec are used verbatim. An `override:` in `endpoints:` beats generation entirely.

All data is synthetic; emails use `example.com`.

## What Kaanal is not (yet)

- It does not speak real database protocols — the database is a behavioural model (timing, failures), not Postgres. Nothing outside Kaanal connects to it.
- Timings are modelled from your configured numbers, not measured from a real system. Treat them as realistic estimates, not predictions.
