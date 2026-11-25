---
title: "How Kaanal works"
sidebar:
  order: 1
---

What Kaanal actually simulates, in plain terms.

## Topology

Your system is a graph of **nodes** connected by **edges** (dependencies):

- **Node kinds:** `gateway`, `service`, `monolith`, `database`, `cache`, `external-api`, `auth`, `load-balancer`, `storage`.
- **Graph rules:** acyclic, with at most one `gateway` and one `auth` node. Invalid graphs are rejected on load with a clear error — never silently ignored.

Requests enter at the `gateway` (or at the route's owning service if there is no gateway) and follow the endpoint's chain. By default, that chain includes every node reachable from the owner; see the [endpoints reference](/docs/guides/mock/endpoints/) to customize it.

`POST /login` and JWKS requests enter at the `auth` node (after the gateway, if one exists). A config with only an `auth` node is valid.

## Call chains

A request to a mocked endpoint doesn't return instantly. Kaanal walks the path _behind_ that endpoint and sums each hop's simulated cost:

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

- **Parallel & cache hops:** calls inside a `parallel` block run side by side, costing as much as the slowest branch. A cache hit skips the rest of its sequence.
- **Trace & client timing:** the hop breakdown returns in `X-Kaanal-Trace`, and the client waits for the full total — so spinners, loading states, and timeouts behave like production.

### Cost models & concurrency

- **Fixed:** e.g. `5ms`.
- **Long-tailed:** `{ p50: 4ms, p99: 20ms }` — most calls take ~4 ms, 1% take ~20 ms, capped at `10 × p99`.
- **Exponential:** `{ mean: 5ms }` — queueing-theory exponential distribution, capped at `10 × mean`.
- **Concurrency limits:** `concurrency: N` caps simultaneous requests per node; extras wait in a queue (trace shows `node.wait=`). With `hold: call`, the slot stays held while waiting on downstream calls. See [Capacity](/docs/guides/simulate/capacity/) for pools, rate limits, and 429s.

## Operations

Each database hop is one of three operations, inferred from the endpoint:

- `GET /users/{id}` → **read** (one row)
- `GET /users` → **scan** (a list)
- Everything else → **write**

Each operation uses its own cost in the node's `ops`. The operation flows down the chain: an endpoint's `op:` sets all database steps below it, `op:` on a single step overrides just that step, and `via:` inherits the target endpoint's op.

## Chaos

Chaos is live runtime state (per node, per endpoint, or per node-at-endpoint). Set it manually or switch on a named [scenario](/docs/guides/break/scenarios/) — neither is baked into node config:

- **latency:** added to every visit of the node.
- **error rate + status:** each visit fails with that probability and HTTP status.
- **down:** every visit fails with `503`.
- **kind:** how an error appears:
  - `status` (default): returns the HTTP status.
  - `reset`: sends the response start, then drops the connection.
  - `malformed`: returns the status with invalid JSON.
  - `hang`: never answers; fails as `504` after the node's `timeout` (60 s default).

A failure stops the chain at that node, and its status becomes the response — a database failure surfaces as a 5xx at the gateway after the time already spent.

### Where the failure happens matters

- **Entry nodes:** at the `gateway` or route-owning `service`, `reset` and `malformed` reach the client as-is.
- **Deeper nodes:** failures from a database, cache call, or external API become `502` (or `504` on timeout), unless the endpoint's `error_map` overrides them.
- **`error_map` lookup order:** node name → kind (`reset`, `malformed`, `timeout`) → status → class (e.g. `5xx`).
- **Caches:** a failing cache is always a miss, never a failed request. Because a cache hit ends the chain, database chaos won't affect cached reads — break the cache too (or lower its hit ratio) to see DB faults on `GET`.

### Unseen faults & timeouts

- **Unseen chaos:** faults only trigger when requests reach that node. A warm cache or an uncalled node means a run can show zero failures even when the node is down. Studio flags this in Controls and Results; check `X-Kaanal-Trace` to see exact hops.
- **Timeouts:** `timeout: 1s` sets a deadline covering queue wait, own work, and downstream calls. On expiry, the caller gets `504` (`"kind":"timeout"`), while the running work keeps its slot until it finishes — turning slow dependencies into a pile of timeouts.

### Layering & chaos on one route

Chaos can target a **node**, an **endpoint** (`GET /orders`), or a **node-at-endpoint**:

- **Stacking within a visit:** latencies add (clamped to 5 minutes per entry key), `down` is ORed, and error rates combine as independent chances.
- **Single-roll error draws:** each failure keeps its own `kind` and `status` tied to its entry's `error_rate` (a `kind` without an `error_rate` warns and does nothing). Two entries are drawn from one roll, so 30% `reset` + 30% `503` yields ~30% resets and ~21% 503s.
- **Layer order (bottom to top):** active scenarios (in activation order), then manual chaos on top. **Higher layers win** per field; within one layer, node-at-endpoint applies before node-level.
- **Overriding & endpoint entries:** `mode: override` replaces everything below it for that key (see the [control API](/docs/reference/control-api/)). An endpoint entry rolls once at the front door (its latency is a leading delay; its failure belongs to the entry node). See [chaos internals](https://github.com/Naveen2070/kaanal/blob/main/docs/source/chaos.md).

### Scenarios

A scenario is a named bundle of chaos entries from `kaanal.yaml` or `scenarios/NAME.yaml` (e.g. "payments is down", "slow database"). Toggle them live, stack several, save manual chaos into one, and reset between tests. See [Scenarios](/docs/guides/break/scenarios/).

## Determinism

Given the same `seed` in `kaanal.yaml`:

- The same URL always returns the same generated body (`GET /users/42` is always the same Priya).
- Sequential requests always see the same timings and failure pattern (under _concurrent_ requests, arrival order decides which specific request fails, while the _rate_ holds).

- **`determinism: controlled` (default):** timings, failures, and generated bodies repeat identically on every run.
- **`determinism: realistic`:** `kaanal up` picks a fresh seed at startup and prints it (`Seed: N ...`). To replay those exact timings and failures later, set `determinism: controlled` and `seed: N` (generated bodies follow `seed:` in both modes).

## Generated data

- **Schema-aware:** `email` gets emails (`example.com`), `pincode` gets pincodes, `price` gets plausible prices, `createdAt` gets recent timestamps, and related fields (`city`, `state`, `pincode`) stay consistent.
- **Overrides:** OpenAPI `example` values are used verbatim; an `override:` in `endpoints:` bypasses generation completely.
- **Locales & dictionaries:** names, places, phones, and currency come from the node's locale pack and an editable dictionary. See [Data and locales](/docs/guides/mock/requests-and-data/#data-and-locales) and [Data packs](/docs/guides/mock/data-packs/).

## What Kaanal is not (yet)

- It does not speak real database protocols — the database is a behavioural model (timing, failures), not Postgres. Nothing outside Kaanal connects to it.
- Timings are modelled from your configured numbers, not measured from a real system. Treat them as realistic estimates, not predictions.
