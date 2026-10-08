# User guide

Kaanal serves a fake version of your backend from a single YAML file, with realistic data, realistic timing, and chaos you can flip live.

## Install & run

```bash
cargo install kaanal        # or grab a release binary
kaanal up                   # reads ./kaanal.yaml, serves on http://localhost:8080
```

`kaanal up` prints two URLs:

```
Mocks: http://localhost:8080
UI:    http://localhost:8080/__kaanal/ui#token=<64 hex chars>
```

Point your frontend's `API_BASE_URL` at the mocks URL. Open the UI URL to watch requests and flip chaos. Only one instance per project directory; a second `kaanal up` tells you it's already running.

## `kaanal.yaml`

```yaml
version: 1
seed: 42                 # same seed → same data and timing, every run
port: 8080
nodes:
  nginx: { type: gateway, flavor: nginx, proc: 1ms }
  app:   { type: service, spec: openapi.yaml, proc: 5ms }
  db:    { type: database, flavor: postgres,
           ops: { read: { p50: 4ms, p99: 20ms }, write: 8ms, scan: 30ms } }
edges:
  - { from: nginx, to: app, net: 1ms }
  - { from: app, to: db, net: 1ms }
routes:                          # optional static overrides
  "GET /health": { body: { ok: true } }
  "GET /users":  { body: [ { id: 1 } ], status: 200 }
```

- **nodes** — `gateway` (exactly one), `service` (with an OpenAPI 3.0 `spec`), `database` (needs `ops` costs for `read`/`write`/`scan`). `proc` is processing time. A duration is `5ms` or a percentile pair `{ p50: 4ms, p99: 20ms }` (a realistic long-tailed distribution).
- **edges** — the dependency graph. Phase one: `gateway → service → database` only. Cycles and dangling references are rejected at startup.
- **routes** — static responses that replace or extend the spec's routes. Same path + same parameter names = override; otherwise a new route.

Check a file without starting:

```bash
kaanal validate -c kaanal.yaml    # ok / errors with file:line:col, plus warnings
```

Warnings flag schema keywords the data generator ignores (like `pattern` or `uniqueItems`) so you're never surprised by generated data that violates them.

## CLI

| Command                                                                               | What it does                                              |
| ------------------------------------------------------------------------------------- | --------------------------------------------------------- |
| `kaanal up [-c f] [--port p] [--host ip]`                                             | Start (foreground). Ctrl+C stops.                         |
| `kaanal validate [-c f]`                                                              | Check config, topology and specs.                         |
| `kaanal status`                                                                       | Table of nodes with their live chaos settings.            |
| `kaanal chaos set <node> [--latency 300ms] [--error-rate 10%] [--error 503] [--down]` | Change chaos live. Fields merge — set only what you name. |
| `kaanal chaos clear [node]`                                                           | Clear one node, or everything with no argument.           |

Exit codes: `0` ok, `1` error, `2` no running instance.

## What a request looks like

```bash
curl -i http://localhost:8080/users/42
```

```http
HTTP/1.1 200 OK
x-kaanal-trace: nginx.proc=1.0ms,net=1.0ms,app.proc=5.0ms,net=1.0ms,db.read=4.2ms
access-control-allow-origin: *

{ "id": 42, "firstName": "Priya", "email": "priya.iyer42@example.com", ... }
```

- The **body** is generated from your OpenAPI response schema — real-looking names, emails, pincodes — and is identical on every call to the same URL. Path params flow into matching fields (`id: 42`).
- The **timing** is the sum of the hops in the trace header. The client actually waits it.
- **CORS** is wide open on mock routes, so any local frontend just works.

## Chaos, live

```bash
kaanal chaos set db --latency 300ms        # every db hop +300 ms
kaanal chaos set db --error-rate 10% --error 504
kaanal chaos set app --down                # hard outage: 503 for everything through app
kaanal status                              # see what's active
kaanal chaos clear db                      # back to normal
```

Failures propagate upstream the way real ones do: a failing database makes the request fail with the database's status, after spending the time of the hops before it. The trace header names the failing node (`...,db.error=504`).

## The status page

Open the UI URL printed by `kaanal up`. It shows a node table with live rps/error% and chaos controls (latency, error %, status, down), plus a live request log. Details: [web-ui.md](web-ui.md).

## Safety notes

- Kaanal binds `127.0.0.1`. `--host 0.0.0.0` works but warns; the control API still requires the token and a loopback `Host` header.
- The token is per-run and lives in `.kaanal/run.json`, readable only by you.
- All generated data is synthetic. Emails always use `example.com`.
