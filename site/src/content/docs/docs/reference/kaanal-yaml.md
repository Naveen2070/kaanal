---
title: "kaanal.yaml"
sidebar:
  order: 1
---

```yaml
version: 1
seed: 42                 # same seed → same data and timing, every run
determinism: controlled  # or realistic: timings vary per run; `kaanal up` prints the seed so you can replay it
port: 8080
nodes:
  nginx: { type: gateway, flavor: nginx, proc: 1ms }
  app:   { type: service, spec: openapi.yaml, proc: 5ms }
  db:    { type: database, flavor: postgres,
           ops: { read: { p50: 4ms, p99: 20ms }, write: 8ms, scan: 30ms } }
edges:
  - { from: nginx, to: app, net: 1ms }
  - { from: app, to: db, net: 1ms }
endpoints:                       # optional: per-endpoint overrides (a file path also works)
  "GET /health": { override: { body: { ok: true } } }
  "GET /users":  { override: { status: 200, body: [ { id: 1 } ] } }
```

- **nodes** — nine kinds: `gateway`, `service`, `monolith`, `database`, `cache`, `external-api`, `auth`, `load-balancer`, `storage` (an object store: `first_byte` and `throughput`; see [Nodes](/docs/reference/nodes/) for every field; `auth` serves login and JWKS, see [Auth](/docs/guides/mock/auth/)). The gateway is optional (at most one); without it, requests enter at the service that owns the route. A `database` needs `ops` costs for `read`/`write`/`scan`, or a `flavor` preset. `proc` is processing time. A duration is `5ms`, a percentile pair `{ p50: 4ms, p99: 20ms }` (a realistic long-tailed distribution) or `{ mean: 5ms }` (exponential, as in queueing theory). Any node may set `concurrency: N` (how many requests it serves at once). Live mode and the simulator both enforce it: extra requests queue, and the trace shows how long with `node.wait=`.
- **edges** — the dependency graph, limited to the [edge table](/docs/reference/edges-and-failure/#edges-who-may-call-whom). No cycles, no self or duplicate edges, no dangling references; all rejected at startup. `calls: N` sets how many times the target is called per request.
- **scenarios** — optional named chaos bundles (`scenarios:` here, or one file each in `scenarios/` next to this file), switched on live; see [Scenarios](/docs/guides/break/scenarios/).
- **endpoints** — per-endpoint behaviour, fully described in [Endpoints](/docs/guides/mock/endpoints/). An entry with `override` serves a fixed response that replaces or extends the spec's routes. Same path + same parameter names = override; otherwise a new static endpoint, which needs a gateway.

Check a file without starting:

```bash
kaanal validate -c kaanal.yaml    # ok / errors with file:line:col, plus warnings
```

Warnings flag schema keywords the data generator ignores (like `not` or `patternProperties`) so you're never surprised by generated data that violates them.
