---
title: "Endpoints reference"
sidebar:
  order: 3
---

What happens behind each route, and how to change it. Every route in a service's OpenAPI `spec` already works with no `endpoints:` at all; add an entry only to change what a route does: a different call chain, a fixed response, or your own error mapping.

## Where entries live

`endpoints:` in `kaanal.yaml` is either a map, or the path of a YAML file holding that map (relative to `kaanal.yaml`). The schema is the same.

```yaml
endpoints: kaanal.endpoints.yaml    # or write the map right here
```

Each key is `"METHOD /path"` as in the spec (`"GET /orders/{id}"`). Every field is optional.

**Base path.** If the spec declares `servers`, the path of the first server is part of every route: with `servers: [{ url: /api/v3 }]` (or `https://example.com/api/v3`), `GET /pet/{petId}` is served at `/api/v3/pet/{petId}` and its key is `"GET /api/v3/pet/{petId}"`, in `endpoints:`, in chaos, in pins and in the trace. A server URL of `/` or with no path adds nothing. Server variables use their `default`; a variable with no default is an error. Only the first server counts.

```yaml
"GET /orders/{id}":
  op: read                       # default op; else inferred from the method and path
  chain:                         # absent = the default chain (below)
    - cache: orders-cache
    - call: orders-svc
      chain: [ { call: orders-db, op: read, repeat: 2 } ]
    - parallel:
        - call: audit-svc
        - call: pay-api
  override: { status: 200, body: { id: 1 } }
  error_map: { orders-db: 503, "5xx": 502, pay-api: { status: 502, body: { error: upstream } } }
```

| Field        | Meaning                                                                                                                                                                                                                                                                                         |
| ------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `op`         | The operation database steps use unless a step sets its own: `read`, `write` or `scan`                                                                                                                                                                                                          |
| `chain`      | The steps that run after the owner's own work. Absent = the default chain                                                                                                                                                                                                                       |
| `override`   | A fixed response (`status`, default 200, `body`, and `headers`, a map of extra response headers, `content_type`, which sends a text `body` as is, for example `text/html`, and `encoding: base64`, which sends the bytes a base64 `body` decodes to). On success it replaces the generated body |
| `error_map`  | What a failure becomes for the client (see below)                                                                                                                                                                                                                                               |
| `collection` | Name of an in-memory collection the route reads or writes; not with `override`                                                                                                                                                                                                                  |
| `total`      | How many records a [paged](/docs/guides/mock/paging/) list route reports, 0 to 10000 (default: seeded per route); not with `override` or `collection`                                                                                                                                           |
| `states`     | What a [pinned state](/docs/guides/break/pins/) answers on this endpoint (see below)                                                                                                                                                                                                            |

A `HEAD` request with no entry of its own follows the `GET` entry.

## The default chain

With no `chain:`, a request does this, using the edges you drew:

1. The gateway's `proc`, and its auth calls, first (when there is a gateway).
2. The owning service's `proc` and, for a monolith, its inline op.
3. Its callees in edge order, each one's own callees below it.
4. Caches, for reads and scans.
5. Databases.

An edge with `calls: N` repeats that call N times. Each node a chain reaches is charged its `proc` (or its op, for a database).

An edge may also set `prob` (above 0, at most 1): the share of requests that take it. It is the default for every `call` over that edge, and a step's own `prob` wins. A skipped call costs nothing and leaves no trace. Caches keep their hit ratio and the gateway routes by endpoint, so neither edge takes a `prob`.

## Steps

| Step                    | What it does                                                                                                                                |
| ----------------------- | ------------------------------------------------------------------------------------------------------------------------------------------- |
| `call: node`            | Charge the edge's `net`, then the callee's work. The edge must exist                                                                        |
| `repeat: N`             | On a `call`: N sequential calls, each sampled on its own (default: the edge's `calls`)                                                      |
| `prob: p`               | On a `call`: the share of requests that take it, above 0 and at most 1 (default: the edge's `prob`, else 1). One draw covers every `repeat` |
| `op: read\|write\|scan` | On a `call` to a database or monolith: use this op for this step only                                                                       |
| `chain: [ ... ]`        | On a `call` to a service or monolith: the steps it runs, written inline                                                                     |
| `with: name`            | On a `call`: run a named chain from the callee's `chains:` (below)                                                                          |
| `via: "GET /x"`         | On a `call`: run what another endpoint of the callee does, with that endpoint's op                                                          |
| `cache: node`           | Look the key up in a cache node                                                                                                             |
| `parallel: [ ... ]`     | Run branches at the same time                                                                                                               |

`chain`, `with` and `via` are mutually exclusive. A bare `- call: node` is the usual form. A `parallel` branch is one step or a list of steps.

Named chains live on the service or monolith:

```yaml
nodes:
  orders-svc:
    type: service
    spec: orders.yaml
    chains:
      reserve: [ { call: stock-db, op: write }, { call: pay-api } ]
```

## How steps behave

- **Cache hit.** A hit ends the sequence the `cache:` step is in (a call's body, a parallel branch, or the endpoint's chain); the caller carries on with its next step. A miss charges the miss cost and continues. A cache that is down or failing counts as a miss.
- **Ops flow down.** The endpoint's `op` applies to every database step below it. `op:` on a step changes that one step. `via:` uses the op of the endpoint it names.
- **`parallel`.** Branches start together and the block ends when the slowest one does. The earliest failure (in time) fails the block and the other branches stop at that moment. Ties go to the first branch.
- **Failures.** A failing step stops its sequence and the failure reaches the client. There are no retries.

## Error mapping

`error_map` turns a failure into the response you want. The failing node's name is tried first, then the failure kind (`reset`, `malformed` or `timeout`), then the exact status, then its class (`5xx`); with no match a status failure keeps its own status, a deep `reset` or `malformed` becomes a 502 and a `timeout` a 504, and at the gateway or the route's owner a `reset` or `malformed` breaks the connection or the body instead (a node or kind key still wins there). A value is a status number or `{ status, body }`.

An endpoint maps only its own errors: when a step uses `via:`, the `error_map` of the endpoint it points at is not used, and `kaanal validate` warns about it.

## `states:` (what a pin answers)

A [pin](/docs/guides/break/pins/) forces an endpoint to answer as `loading`, `empty`, `error` or `slow`. `states:` changes what each of those answers on this endpoint:

```yaml
"GET /orders":
  states:
    loading: { delay: 30s }                       # default +10 s
    slow: { delay: 4s }                           # default +2 s
    error: { status: 404, body: { error: gone } } # default 500 and {"error":"pinned","state":"error"}
    empty: { body: { orders: [], total: 0 } }     # default: derived from the response schema
```

| Field    | Meaning                                                                                |
| -------- | -------------------------------------------------------------------------------------- |
| `status` | `error`: 400 to 599. `empty`: 200 to 299. Not allowed on `loading` or `slow`.          |
| `body`   | Returned as written, with no `{id}` substitution. Allowed on `error` and `empty` only. |
| `delay`  | Above 0, at most 5 min.                                                                |

Keys are `loading`, `empty`, `error` and `slow`; `success` is the normal response and cannot be overridden. A value on the pin itself (`kaanal cycle --status`, `--delay`) beats `states:`, which beats the defaults. A 204 or 304 status never carries a body, so `empty: { status: 204, body: X }` drops the body. A `states.empty.body` also lets you pin `empty` on a route that has no schema-derived empty form. `states:` is checked on every entry, even a stale one, so a typo is always caught.

## Static endpoints

An entry with an `override` that no spec defines is a static endpoint. It needs a gateway, which answers it (paying its `proc` and running its auth calls).

```yaml
"GET /health": { override: { body: { ok: true } } }
```

## Limits

Chains are checked when the file loads; breaking a limit is an error that names the endpoint and step.

| Limit                                            | Value   |
| ------------------------------------------------ | ------- |
| Steps in one chain                               | 256     |
| Branches in one `parallel`                       | 32      |
| Nesting of `parallel` and inline `chain:` bodies | 16      |
| `repeat`                                         | 1 000   |
| Hops in one request                              | 100 000 |
| Calls nested                                     | 64 deep |

## Determinism

`determinism: controlled` (the default) makes timings and failures repeat on every run for the same `seed`. With `determinism: realistic` they vary from run to run: `kaanal up` draws a seed and prints it. To replay that run's timings and failures, set `determinism: controlled` and `seed:` to the printed number. Generated data follows `seed:` in both modes, so it changes with the new `seed:`.

## `kaanal endpoints`

```bash
kaanal endpoints [-c kaanal.yaml] [--write]
```

Prints one skeleton line for each spec route with no entry yet, sorted, under a `# Added by kaanal endpoints` header:

```yaml
# Added by kaanal endpoints
"GET /users/{id}": {} # owner app, op read
"POST /orders": {} # owner app, op write
```

It only ever adds. With `--write`:

- If `endpoints:` names a file, the lines are appended to it. Your existing text and line endings are untouched.
- If `endpoints:` names a file that is one flow-style `{ ... }` map, it is refused and left alone (lines cannot be appended to a flow map); rewrite it in block style, or paste the lines in yourself.
- If `endpoints:` is a map inside `kaanal.yaml`, nothing is written (Kaanal never rewrites that file); paste the printed lines below `endpoints:` (they are already indented).
- If there is no `endpoints:` key, `kaanal.endpoints.yaml` is created and the command tells you the line to add to `kaanal.yaml`.

Run it again and it says `nothing to add`.

`kaanal validate` also warns about stale entries (no spec defines the route and it has no override) and about the `error_map` and `override` of an endpoint a `via:` points at, which are ignored (`via` runs that endpoint's chain, not its fixed response).

## The trace

The `X-Kaanal-Trace` header lists each hop in order. A `parallel` block shows its branches in brackets split by `|`, a cache shows `hit` or `miss`, and a failure ends the header with `node.error=STATUS`. A header cut at 4 KiB ends with `,...+N hops` (N counts the trace pieces left out; a `node.wait=` piece counts like a hop); when the request failed, the failure follows it (`...,...+N hops,node.error=STATUS`):

```
nginx.proc=1.0ms,net=0.0ms,app.proc=2.0ms,[net=0.0ms,audit.proc=1.0ms|net=0.0ms,pay.proc=4.0ms],net=0.0ms,c.hit=0.2ms
```

## Errors

Mistakes in an endpoints file are reported with that file's path (as kaanal resolved it, so `.\kaanal.endpoints.yaml` on Windows), line and column. A missing or unreadable file is reported at the `endpoints:` value in `kaanal.yaml`:

```text
kaanal.endpoints.yaml:3:13: endpoint 'GET /users/{id}': unknown node 'pay'
```

## What is not modelled

- A cache hit is a dice roll at the node's `hit_ratio`; the cache remembers nothing between requests.
- No connection reuse and no TLS cost.
- No retries. A `timeout:` on a node is a deadline for calls to it (see [Nodes](/docs/reference/nodes/)); there are no per-edge or per-caller timeouts.
- A `parallel` branch cancelled by a sibling's failure still uses the capacity it had already taken (a running hop finishes; a queued one leaves the line), live and in the simulator.
