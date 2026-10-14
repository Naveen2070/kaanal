# User guide

Kaanal serves a fake version of your backend from a single YAML file, with realistic data, realistic timing, and chaos you can flip live.

## Install & run

```bash
cargo install kaanal        # or grab a release binary
kaanal up                   # reads ./kaanal.yaml, serves on http://localhost:8080
```

`kaanal up` prints two URLs (and an `Auth:` line when the file has an `auth` node):

```
Mocks: http://localhost:8080
UI:    http://localhost:8080/__kaanal/ui#token=<64 hex chars>
Auth:  POST http://localhost:8080/login, JWKS http://localhost:8080/.well-known/jwks.json, 1 user
```

Point your frontend's `API_BASE_URL` at the mocks URL. Open the UI URL to watch requests and flip chaos. Only one instance per project directory; a second `kaanal up` tells you it's already running.

**Log in from your frontend.** With an `auth` node, `POST /login` returns a signed JWT. Send `Content-Type: application/json` (a `fetch` with `JSON.stringify` and no header sends `text/plain` and gets 415). See [auth.md](auth.md).

## `kaanal.yaml`

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

- **nodes** — seven kinds: `gateway`, `service`, `monolith`, `database`, `cache`, `external-api`, `auth` (see [nodes.md](nodes.md) for every field; `auth` serves login and JWKS, see [auth.md](auth.md)). The gateway is optional (at most one); without it, requests enter at the service that owns the route. A `database` needs `ops` costs for `read`/`write`/`scan`, or a `flavor` preset. `proc` is processing time. A duration is `5ms`, a percentile pair `{ p50: 4ms, p99: 20ms }` (a realistic long-tailed distribution) or `{ mean: 5ms }` (exponential, as in queueing theory). Any node may set `concurrency: N` (how many requests it serves at once). Live mode and the simulator both enforce it: extra requests queue, and the trace shows how long with `node.wait=`.
- **edges** — the dependency graph, limited to the [edge table](nodes.md#edges-who-may-call-whom). No cycles, no self or duplicate edges, no dangling references; all rejected at startup. `calls: N` sets how many times the target is called per request.
- **endpoints** — per-endpoint behaviour, fully described in [endpoints.md](endpoints.md). An entry with `override` serves a fixed response that replaces or extends the spec's routes. Same path + same parameter names = override; otherwise a new static endpoint, which needs a gateway.

Check a file without starting:

```bash
kaanal validate -c kaanal.yaml    # ok / errors with file:line:col, plus warnings
```

Warnings flag schema keywords the data generator ignores (like `pattern` or `uniqueItems`) so you're never surprised by generated data that violates them.

## CLI

| Command                                                                                                                   | What it does                                                                                    |
| ------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------- |
| `kaanal up [-c f] [--port p] [--host ip]`                                                                                 | Start (foreground). Ctrl+C stops.                                                               |
| `kaanal validate [-c f]`                                                                                                  | Check config, topology and specs.                                                               |
| `kaanal schema`                                                                                                           | Print the JSON Schema; save it and point your editor's YAML plugin at it.                       |
| `kaanal endpoints [-c f] [--write]`                                                                                       | Print skeleton entries for spec routes with none; `--write` appends them to the endpoints file. |
| `kaanal status`                                                                                                           | Table of nodes with their live chaos settings.                                                  |
| `kaanal chaos set [node] [--endpoint "GET /x"] [--latency 300ms] [--error-rate 10%] [--error 503] [--kind hang] [--down]` | Change chaos live. Fields merge — set only what you name.                                       |
| `kaanal chaos clear [node] [--endpoint "GET /x"]`                                                                         | Clear one node, one endpoint or one node-at-endpoint entry, or everything with no argument.     |

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
- The **timing** is what the trace header adds up to: hops in sequence add; a `parallel` block costs its slowest branch. The client actually waits it.
- A `parallel` step shows its branches in brackets, split by `|`, and a cache lookup shows `hit` or `miss`: `nginx.proc=1.0ms,net=0.0ms,app.proc=2.0ms,[net=0.0ms,audit.proc=1.0ms|net=0.0ms,pay.proc=4.0ms],net=0.0ms,c.hit=0.2ms`. A failure ends the header with `node.error=STATUS`. Very long chains end with `,...+N hops`, where N is the number of hops left out.
- When a node is full, a request queues and the trace shows it before the hop it waited for: `...,net=1.0ms,db.wait=98.0ms,db.read=100.0ms`. A node with a `rate_limit` answers `429` with a `Retry-After` header (whole seconds) once its tokens run out.
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
- `KAANAL_CONFIG_DIR` sets the base folder for the auth signing key (default: your per-user local config folder). See [auth.md](auth.md).
- All generated data is synthetic. Emails always use `example.com`.
