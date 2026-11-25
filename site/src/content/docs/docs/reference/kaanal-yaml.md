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

## Top-level sections

### `nodes`

Nine kinds: `gateway`, `service`, `monolith`, `database`, `cache`, `external-api`, `auth`, `load-balancer`, and `storage` (`first_byte` and `throughput`). See [Nodes](/docs/reference/nodes/) for all fields and [Auth](/docs/guides/mock/auth/) for `auth`.

- **Gateway:** optional (at most one); without it, requests enter at the route's owning service.
- **Database:** requires `ops` (`read`/`write`/`scan`) or a `flavor` preset.
- **Durations (`proc`, `ops`):** fixed (`5ms`), long-tailed (`{ p50: 4ms, p99: 20ms }`), or exponential (`{ mean: 5ms }`).
- **Concurrency:** `concurrency: N` caps concurrent requests on any node; extras queue in both live mode and the simulator (`node.wait=` in trace).

### `edges`

Defines the dependency graph, subject to the [edge table](/docs/reference/edges-and-failure/#edges-who-may-call-whom). Cycles, self-edges, duplicate edges, and dangling references fail at startup. Use `calls: N` to repeat a dependency per request.

### `scenarios`

Optional named chaos bundles defined inline or as files in `scenarios/`. Toggle them live; see [Scenarios](/docs/guides/break/scenarios/).

### `endpoints`

Per-endpoint behaviour; see [Endpoints](/docs/guides/mock/endpoints/). An `override` serves a fixed response — matching path + parameter names overrides the spec route, while a new path creates a static endpoint (requires a gateway).

## Validating a config

Check a file without starting the server:

```bash
kaanal validate -c kaanal.yaml    # ok / errors with file:line:col, plus warnings
```

Warnings flag schema keywords the data generator ignores (like `not` or `patternProperties`) so you're never surprised by generated data that violates them.
