# Control API reference

Everything under `/__kaanal/` — the single control plane used by the CLI, the status page, and your own scripts and test setups.

## Authentication & transport rules

- **Token**: every endpoint except `health` and `ui` requires `Authorization: Bearer <token>`. The token is printed by `kaanal up` and stored in `.kaanal/run.json`. On `/__kaanal/events` only, `?token=` is accepted (browser `EventSource` can't set headers).
- **Host guard**: the `Host` header must be `localhost`, `127.0.0.1` or `[::1]`, with the bound port (or none). Anything else → `403`. This blocks DNS-rebinding attacks from browsers.
- **No CORS** on this namespace, ever — browsers on other origins cannot call it.
- Responses carry `Cache-Control: no-store` and `X-Content-Type-Options: nosniff`.
- Errors are always JSON: `{ "error": "message" }`.

## Endpoints

### `GET /__kaanal/health`

Liveness probe. No token.

```json
{ "ok": true }
```

### `GET /__kaanal/topology`

The running topology.

```json
{
  "nodes": [ { "name": "nginx", "type": "gateway", "flavor": "nginx" } ],
  "edges": [ { "from": "nginx", "to": "app" } ],
  "routes": [ "GET /health", "GET /users", "GET /users/{id}", "POST /orders" ],
  "endpoints": [ { "key": "GET /users/{id}", "visits": { "nginx": 1.0, "app": 1.0, "db": 1.0 } } ]
}
```
`endpoints[].visits` is how many times one request to that endpoint is expected to enter each node (zero entries are left out; HEAD copies are not listed). It follows the chains: `calls`, `repeat` and `prob` multiply, and a cache hit ends the rest of its sequence, so a database behind a cache with an 80% hit ratio shows 0.2. Chaos is ignored.

### `GET /__kaanal/chaos`

The **live layer**: chaos set through `PUT /__kaanal/chaos...`, in three groups (empty when nothing is set), plus `effective`, the result of the active [scenarios](#scenarios) and the live layer together. (Before the failures release this was one flat map by node name.)

```json
{
  "nodes": { "db": { "latency_ms": 300, "error_rate": 0.1, "error_kind": "status", "error_status": 503, "down": false } },
  "endpoints": { "GET /orders": { "latency_ms": 0, "error_rate": 0.5, "error_kind": "reset", "error_status": 503, "down": false } },
  "node_at": [{ "node": "db", "endpoint": "GET /orders", "latency_ms": 100, "error_rate": 0, "error_kind": "status", "error_status": 503, "down": false }],
  "effective": {
    "nodes": { "db": { "latency_ms": 300, "down": false, "faults": [{ "upto": 0.1, "kind": "status", "status": 503 }] } },
    "endpoints": { "GET /orders": { "latency_ms": 0, "down": false, "faults": [{ "upto": 0.5, "kind": "reset", "status": 503 }] } },
    "node_at": [{ "node": "db", "endpoint": "GET /orders", "latency_ms": 400, "down": false, "faults": [{ "upto": 0.1, "kind": "status", "status": 503 }] }]
  }
}
```

- An entry set with `mode: override` shows `"mode": "override"`.
- An entry with leased fields (set with a `ttl`, see below) shows `"expires_in_ms": { "down": 29500 }`: each leased unit and the milliseconds left, rounded up. Absent when nothing on the entry is leased.
- `effective` is what requests actually experience. Per key: `latency_ms` (the sum over all layers, at most 300 000), `down`, and `faults`, the failure bands from the top layer down. `upto` is the cumulative probability, so `[{"upto":0.3,"kind":"reset"},{"upto":0.51,"kind":"status","status":503}]` means 30% resets and 21% 503s. **Keys that do nothing are left out** (for example one a live override calmed), so an empty `effective` means no chaos at all.

### `PUT /__kaanal/chaos/{node}`

**Merges** a patch into the node's live chaos — fields you omit keep their current values. Returns the stored state of that node's live entry, plus `effective` and `warnings`.

```json
{ "latency_ms": 300, "error_rate": 0.1, "error_kind": "status", "error_status": 504, "down": true, "mode": "merge" }
```

| Field          | Rules                                                                                                                                                                                                                     |
| -------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `latency_ms`   | integer, 0 – 300 000                                                                                                                                                                                                      |
| `error_rate`   | number, 0.0 – 1.0                                                                                                                                                                                                         |
| `error_kind`   | `status` (default), `reset`, `hang` or `malformed`; anything else is 400                                                                                                                                                  |
| `error_status` | integer, 500 – 504 (checked for every kind)                                                                                                                                                                               |
| `down`         | boolean — when true, everything through the node fails 503                                                                                                                                                                |
| `mode`         | `merge` (default) or `override`; anything else is 400. `override` replaces what scenarios add to this key (see below).                                                                                                    |
| `ttl`          | optional duration string, 100 ms – 24 h (`"30s"`, `"5m"`); leases the fields this PUT sets (see below); a `ttl` with nothing to lease (no latency, error, down or `mode: override`) is a 400 `ttl needs a field to lease` |

Unknown node → `404 "unknown node '<name>'"`. Out-of-range values → `400`. Unknown JSON keys → rejected.

In the live layer `down: false` means "unset": it clears a `down` you set earlier, and an entry left with nothing in it is removed. It never lowers a scenario's outage; use `mode: override` for that.

**`warnings`** is an array of strings, empty when all is well. An `error_kind` or `error_status` without an `error_rate` **in the same entry** does nothing, because a failure kind belongs to its own rate; the response says so (`"chaos on node 'app': error_kind/error_status have no effect without error_rate"`). Only the key you just wrote is reported. It is a warning, not an error: the write succeeds.

**`ttl` (timed chaos).** A PUT with `"ttl": "30s"` leases what it sets, per **unit**: `latency` (`latency_ms`), `error` (`error_rate`, `error_kind` and `error_status` together), `down`, `mode`, and on an edge `jitter` (`jitter_ms`) and `bandwidth` (`bandwidth_cap`), each its own lease. A PUT without `ttl` makes the units it sets permanent again (a plain `{"down": true}` after a timed one keeps the node down), and units a PUT does not mention keep their lease, so `latency_ms: 200` set permanently plus `{"down": true, "ttl": "30s"}` leases only `down`. Sending a unit again with a new `ttl` replaces its deadline. A bad `ttl` (outside the range, not parseable) is `400` naming `ttl`, and nothing changes. The lease lives in the server, so it outlasts the client that set it. It ends with its field: `DELETE` of the entry, `POST /__kaanal/reset`, `down: false` (for `down`), or a `save` with `clear_live`. When the time is up the unit's fields are **cleared**, not restored to an earlier value (`mode` goes back to merge), an entry with no settings left (equal to an empty entry, no `mode` either) is removed while one that keeps a permanent bare `mode: merge` stays, and one `expire` event names the key (see [events](#get-__kaanalevents)); units of one key that run out together are cleared in one step with one event. The answer and `GET /__kaanal/chaos` carry `expires_in_ms`. Leases are in memory only, and `scenarios/save` from live writes leased fields as permanent with a warning counting the leased fields (`"3 fields have a ttl; the file has none"`; `mode` is not counted).

**`mode: override`.** By default a live entry adds to the active scenarios (latencies sum, `down` ORs, error rates stack). With `"mode": "override"` the entry replaces everything lower layers contribute to that key, so `{"mode":"override","down":false}` or a bare `{"mode":"override"}` calms a scenario that takes the node down. A node override also calms that node's node-at-endpoint entries from scenarios; an endpoint or node-at-endpoint override affects only its own key, so a node-at-endpoint override cannot calm chaos set on the node itself (use a node override for that). The mode **sticks**: a later `PUT` without `mode` keeps it, and `"mode": "merge"` goes back to stacking.

### `PUT /__kaanal/chaos`

Like the per-node route, but the body also names where the chaos applies: `node`, `endpoint` (for example `"GET /orders"`; the method is case-insensitive) or both (the node only while it serves that endpoint). One of them is required (`400` otherwise); an unknown node or endpoint is `404`; an endpoint that is not `METHOD /path` is `400`. Entries from different layers stack: a node entry and a node-at-endpoint entry, or a live entry and a scenario, all apply (latencies add, `down` is an OR, error rates combine as independent chances). **A higher layer wins over a lower one** for the same field, whatever the key; inside one layer the node-at-endpoint entry goes before the node's. An endpoint entry is a separate roll at the front door, before the chain, and is not merged into the nodes behind it (see [chaos internals](../source/chaos.md#endpoint-chaos-and-keys)). Answers like the per-node route.

To act on one copy of a replicated node, add `"replica": N` (0-based) next to `node`: `{ "node": "db", "replica": 1, "down": true }`. A replica needs a node, takes no `endpoint` (it applies whatever the endpoint) and must exist (`400` otherwise: `node 'db' has 3 replicas (0 to 2)`). A downed replica is skipped by the balancer from the request's arrival on; if every replica is down the call fails with a 503. A replica entry also carries the node's own entry and its node-at-endpoint entries. `GET /__kaanal/chaos` lists them under `replicas: [{ node, replica, ... }]` (the group is left out when there are none), `DELETE /__kaanal/chaos?node=db&replica=1` removes one.

To make a **link** misbehave, name an edge instead: `{ "edge": { "from": "app", "to": "db" }, "latency_ms": 20, "jitter_ms": 5 }`. An edge takes no `node`, `endpoint` or `replica` (`400`) and must exist (`404`: `no edge 'a' -> 'b'`). It takes `latency_ms`, `jitter_ms` (up to that much extra either way), `error_rate` with `error_kind` `reset` (the default) or `hang` (never `status` or `malformed`, and no `error_status`), `down` (a partition) and `bandwidth_cap` (bytes per second; only on an edge that has a `bandwidth`, and at most that value). `jitter_ms` and `bandwidth_cap` on any other key are a `400`. The same `ttl` and `mode` rules apply; `jitter_ms` is leased with `latency_ms` and `bandwidth_cap` as its own `bandwidth` unit. `GET /__kaanal/chaos` lists them under `edges` as `{ edge: { from, to }, ... }`; clear one with `DELETE /__kaanal/chaos?edge_from=app&edge_to=db`. What it does to a call over the edge: `latency_ms` and `jitter_ms` change only the network hop (jitter is uniform in plus or minus its value, never below zero); `error_rate` drops a share of calls as a `reset` (a 502 at the call) or a `hang`; `down` is a partition, a hang: the caller waits the callee's `timeout` (else 60 s) and gets a 504; `bandwidth_cap` lowers the link speed, so a payload takes longer. A link to a cache that drops a lookup is a miss, never a failure.

```json
{ "endpoint": "GET /orders", "error_rate": 1.0, "error_kind": "reset" }
```

### `DELETE /__kaanal/chaos/{node}` / `DELETE /__kaanal/chaos`

Clear one node (`404` if unknown) or all **live** chaos. Both return `204`. `DELETE /__kaanal/chaos?node=&endpoint=` clears exactly the entry under that key (`node`, `endpoint` or both) and leaves the others; an unknown query parameter is `400` and clears nothing, a malformed `endpoint` is `400`, an unknown endpoint is `204` (nothing to remove) and an empty or unknown `node` is `404`. **Scenarios are not touched**; use `DELETE /__kaanal/scenarios` or `POST /__kaanal/reset`.

### Scenarios

A [scenario](scenarios.md) is a named list of chaos entries. The scenarios you activate form a stack, in activation order, with the live chaos above them. Names follow `^[a-z0-9][a-z0-9_-]{0,63}$`; `active`, `save`, `reset` and Windows device names are reserved. All of these need the token. Every write is atomic: a bad request changes nothing, and once a `2xx` response is back, every later request sees the new state.

| Method and path                              | Purpose                                                           | Success                                                         |
| -------------------------------------------- | ----------------------------------------------------------------- | --------------------------------------------------------------- |
| `GET /__kaanal/scenarios`                    | List the defined scenarios                                        | `200` array                                                     |
| `GET /__kaanal/scenarios/{name}`             | One definition (export it for a bug report)                       | `200`                                                           |
| `PUT /__kaanal/scenarios/{name}`             | Define or replace a runtime scenario                              | `200 { "name", "warnings" }`                                    |
| `DELETE /__kaanal/scenarios/{name}`          | Undefine a runtime scenario                                       | `204`                                                           |
| `POST /__kaanal/scenarios/{name}/activate`   | Push to the top of the scenarios (a re-activation moves it there) | `200 { "active", "expires_in_ms", "effective" }`                |
| `POST /__kaanal/scenarios/{name}/deactivate` | Remove from the stack                                             | `204` (also when it was not active, or the name is not defined) |
| `GET /__kaanal/scenarios/active`             | The stack and the compiled result                                 | `200 { "active", "effective" }`                                 |
| `PUT /__kaanal/scenarios/active`             | Set the whole stack at once, body `["a","b"]` (bottom to top)     | `200 { "active", "effective" }`                                 |
| `DELETE /__kaanal/scenarios`                 | Deactivate every scenario; live chaos stays                       | `204`                                                           |
| `POST /__kaanal/scenarios/save`              | Write `scenarios/NAME.yaml`                                       | `200 { "name", "path", "warnings" }`                            |
| `POST /__kaanal/reset`                       | Clean slate for tests                                             | `204`                                                           |

`active` is the list of active names, bottom to top; `effective` has the same shape as in `GET /__kaanal/chaos` (calm keys omitted); `expires_in_ms` maps each leased active scenario to the milliseconds left, rounded up (`{}` when none is leased). `GET`/`PUT` of `scenarios/active` carry it too, and each active leased row of the list has an `expires_in_ms` number.

**Activate with a lease.** `POST .../activate` takes an optional JSON body `{ "ttl": "30s" }` (humantime, 100 ms to 24 h). The scenario is deactivated when the time is up and an `expire` event is sent. An empty body, `{}` or `{"ttl":null}` is a plain activation and removes any lease the scenario had; activating again with a `ttl` replaces the lease. Anything else is `400`: a `ttl` outside the range or not parseable, a non-string `ttl`, another key, invalid JSON. Nothing changes on a `400`. A lease is dropped when its activation ends (deactivate, `PUT scenarios/active`, `DELETE scenarios`, `reset`, deleting the scenario).

**Scoping a mock request.** A request to a mock route may carry `X-Kaanal-Scenario: a,b` (comma separated names, at most 16, at most 256 bytes) and is then answered with exactly that stack, ignoring the global stack and the live layer. The response echoes it as `X-Kaanal-Scope: a,b` (an empty request header echoes an empty value). A bad header answers `400 { "error": "bad scenario scope", "detail": "..." }`. `404` and `405` answers, and the `/__kaanal` namespace, ignore the header. Details and the shared-counter caveat: [scenarios](scenarios.md#parallel-test-workers).

**List.** `GET /__kaanal/scenarios` returns `[{ "name", "source", "description", "entries", "pins", "active" }]` (`entries` counts chaos entries and pins; `pins` lists them as `{ endpoint, state, status?, delay? }`, `delay` as text like `"2s"`; `GET /scenarios/{name}` carries `chaos` and `pins`) in definition order; `source` is `inline` (`kaanal.yaml`), `file` (`scenarios/NAME.yaml`) or `runtime`.

**Define.** `PUT /__kaanal/scenarios/{name}` takes the same body as a scenario file (`chaos` and `pins` are both optional, one of them is required; pins are `{ endpoint, state, status?, delay? }`, see [pins](pins.md)):

```json
{
  "description": "Payments hard down",
  "chaos": [
    { "node": "payments", "down": true },
    { "endpoint": "POST /checkout", "latency_ms": 800 },
    { "node": "db", "endpoint": "GET /orders", "error_rate": 0.3, "error_kind": "reset" }
  ]
}
```

It is checked exactly like a file: names against your topology, limits, duplicate keys, `mode` is not allowed in entries (scenario entries always stack). `warnings` lists entries that do nothing (an orphan kind or status) or never fire (a node the endpoint does not visit). Replacing an **active** scenario takes effect at once. Only runtime scenarios can be replaced or undefined: a scenario from `kaanal.yaml` or a file answers `409`.

| Status | When                                                                                                                                                                                        |
| ------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `400`  | Bad name, bad entry (the message says where: `chaos[2].node: ...` or `pins[0].status: ...`), a `limits:` key, a scenario with neither `chaos` nor `pins`, an `empty` pin with no empty form |
| `404`  | Unknown scenario, or an entry naming an unknown node or endpoint                                                                                                                            |
| `422`  | The body is not a scenario (for example an unknown key such as `mode` in an entry)                                                                                                          |
| `409`  | Redefining or undefining a scenario that comes from the config or a file                                                                                                                    |

**Save.** `POST /__kaanal/scenarios/save` with

```json
{ "name": "flaky-db", "from": "live", "force": false, "clear_live": true }
```

- `name` is required. `from` is `"live"` (default), `"effective"` or `{ "scenario": "other" }`; any other shape, including a bare scenario name, is `422`.
- Writes `scenarios/NAME.yaml` next to `kaanal.yaml` (created if missing), with only the fields that were set, and defines the scenario at once. `path` in the response is the file.
- `400`: bad name; the name is defined inline in `kaanal.yaml` (even with `force`); nothing to save; a live `mode: override` with `from: "live"`; `from: "effective"` with a key that has two or more failure kinds stacked, or a node-at-endpoint entry whose node also has chaos (the message names the key). `404`: the `from` scenario does not exist. `409`: the scenario or file exists and `force` is not true. `500`: the file could not be written.
- `warnings` is an array of strings: with `from: "live"` or `"effective"`, live fields set with a `ttl` are written as permanent (a file has no `ttl`), and the response says how many: `"N fields have a ttl; the file has none"`.
- `clear_live: true` clears the live chaos after writing, so activating the saved scenario does not count it twice. Saving over an active scenario updates it in place.

**Reset.** `POST /__kaanal/reset` deactivates every scenario, **undefines the runtime-defined ones** (inline and file scenarios stay), clears the live chaos, zeroes the per-route request counters (so a seeded run repeats), refills rate-limit buckets, empties every [collection](collections.md) and empties the [request log](observability.md) (request ids keep counting). Queues and pools are not reset; requests already in flight finish with the chaos they started with. Call it between tests.

### Pins

Live [pins](pins.md) force an endpoint to answer as `loading`, `empty`, `error`, `slow` or `success`. They sit above every scenario's pins. All four routes need the token, and every write is atomic like the scenario routes.

| Method and path                     | Purpose                                                                  | Success                                               |
| ----------------------------------- | ------------------------------------------------------------------------ | ----------------------------------------------------- |
| `GET /__kaanal/pins`                | Live pins and the pin in force per endpoint                              | `200 { "live", "effective" }`                         |
| `PUT /__kaanal/pins`                | Set the live pin of one endpoint                                         | `200 { "endpoint", "state", "status"?, "delay_ms"? }` |
| `DELETE /__kaanal/pins?endpoint=EP` | Remove that live pin; with no query, every live pin. Scenario pins stay. | `204` (also when there was nothing to remove)         |
| `POST /__kaanal/cycle`              | Step the endpoint's live pin to the next state, or pin `state`           | `200`, same shape as `PUT`                            |

**Body** of `PUT /pins` and `POST /cycle`:

```json
{ "endpoint": "GET /orders", "state": "error", "status": 429, "delay": "500ms" }
```

- `endpoint` is required (`METHOD /path`, method case-insensitive). `state` is required for `PUT` and optional for `cycle` (none: the next of `loading, empty, error, slow, success` after the endpoint's current live pin, wrapping, skipping `empty` where it has no empty form). `status` and `delay` (text such as `"2s"`) are optional. Unknown keys are `422`.
- Rules: `success` takes no `status` or `delay`; `status` is 400 to 599 on `error`, 200 to 299 on `empty`, not allowed on `loading` or `slow`; `delay` is above 0 and at most 5 min.

**`GET /pins`**:

```json
{
  "live": [ { "endpoint": "GET /orders", "state": "error", "status": 429, "delay_ms": 500 } ],
  "effective": {
    "GET /orders": { "state": "error", "status": 429, "delay_ms": 500, "source": "live" },
    "GET /users": { "state": "empty", "source": { "scenario": "orders-empty" } }
  }
}
```

`effective` is the winning pin per endpoint (a `success` pin is listed, because it masks); `source` is `"live"` or `{ "scenario": NAME }`.

| Status | When                                                                                                                                                                                      |
| ------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `400`  | Malformed endpoint, a `HEAD` made from its `GET` (pin the `GET`), a route with no entry node, unknown `state`, a `status` or `delay` the state does not allow, `empty` with no empty form |
| `404`  | Unknown endpoint                                                                                                                                                                          |
| `422`  | The body is not a pin (unknown key, missing `endpoint`)                                                                                                                                   |

A `400` or `404` changes nothing. `POST /__kaanal/reset` clears live pins; `DELETE /__kaanal/chaos` leaves them.

### `GET /__kaanal/logs`

The request log: the last finished mock requests, oldest completion first. What an entry holds, the flags and the commands that use this route are in [observability](observability.md).

| Parameter  | Meaning                                                                                                                                                                                                     |
| ---------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `limit`    | How many of the latest matching entries to return: an integer from 1 to 1000. Default 100.                                                                                                                  |
| `endpoint` | The route template as logged, `GET /users/{id}` (the method's case and the spacing are normalised, the text must be `METHOD /path`). A concrete URL such as `GET /users/1` is accepted and matches nothing. |
| `status`   | Comma separated codes and classes: `404`, `5xx`, `404,5xx`.                                                                                                                                                 |
| `failed`   | `true` keeps requests with a `failed_node`; `false` keeps everything.                                                                                                                                       |
| `scope`    | The exact scope key as echoed in `X-Kaanal-Scope`. **`scope=` with no value** is the scope of an empty `X-Kaanal-Scenario` header, not "any scope".                                                         |
| `since`    | A duration (`5m`, `30s`) or an RFC 3339 time: only requests that finished at or after it.                                                                                                                   |
| `after`    | A `done` number: only entries that finished after it. To poll, pass the highest `done` you have seen.                                                                                                       |

```json
{
  "entries": [
    {
      "id": "r19a2-0004", "client_id": null, "seq": 4, "done": 4, "ts_ms": 1792345019872,
      "method": "GET", "path": "/users/7", "endpoint": "GET /users/{id}", "status": 400,
      "ms": 0, "trace_ms": 0.0, "scope": null, "pin": null, "flags": [],
      "chain_error": null, "failed_node": null, "kind": null, "rejected": "bad-scope",
      "trace": "", "hops": [], "truncated": false
    }
  ],
  "oldest": "r19a2-0001",
  "newest": "r19a2-0004",
  "cap": 1000,
  "evicted": 0
}
```

- `entries` are ordered by completion (`done`), not by id: a slow request that started first comes after the fast ones that overtook it.
- `oldest` and `newest` are the ids with the lowest and highest number **in the ring** (both `null` when it is empty). `cap` is the ring size, 1000, and `evicted` counts the entries pushed out since start (a `POST /reset` does not change it).
- A hop is `{ node, replica?, kind, op?, start_ms, dur_ms, wait_ms, lane, fork, cut, why }`; `kind` is `net`, `proc`, `op` (then `op` is `read`, `write` or `scan`), `hit`, `miss` or `delay`. `replica` (0-based) is present only for a node with more than one replica, and the trace text names that hop `db#2`. `chain_error` is `{ node, status, kind, cause }`.
- Nothing is applied partly: an unknown, repeated, empty (except `scope`) or malformed parameter answers `400 { "error": "parameter 'limit': expected an integer from 1 to 1000" }`, naming the parameter. `limit` takes plain digits only (no sign, no space).
- `POST /__kaanal/reset` empties the ring. Request ids keep counting.

### `GET /__kaanal/trace/{id}`

One log entry, in the same shape as above. `{id}` is one of:

- Kaanal's id, `r19a2-0004`. Leading zeros do not matter (`r19a2-00004` is the same entry).
- `last`: the most recent entry **to finish**.
- A client `X-Request-Id` that was sent with a request (the newest request that used it).

A text that is also an id is read as an id first. Percent-escapes are decoded.

| Status | Body                                                                                        | When                                                                                                            |
| ------ | ------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------- |
| `200`  | the entry                                                                                   | Found.                                                                                                          |
| `404`  | `{ "error": "evicted", "detail": "older than the last 1000 requests, or still in flight" }` | An id of this run, at or below the highest number recorded, that is not in the ring.                            |
| `404`  | `{ "error": "unknown" }`                                                                    | An id of another run, a number above the highest recorded, a client id nobody sent, or `last` on an empty ring. |
| `400`  | `{ "error": "expected a request id like r04f2-0042, 'last' or a client X-Request-Id" }`     | Neither an id, `last` nor a valid client id (`[A-Za-z0-9._-]`, at most 64), or escapes that are not UTF-8.      |

Both routes need the token and the `Host` guard like every other control route, and neither is open to CORS.

### `GET /__kaanal/events`

Server-Sent Events stream of every mock request as it completes, and of scenario changes.

```
event: request
data: {"id":"r19a2-0001","client_id":"checkout-17","seq":1,"rejected":null,"ts_ms":1792345019677,"method":"GET","path":"/users/7","endpoint":"GET /users/{id}","status":200,"ms":25,"nodes":["nginx","app","db"],"failed_node":null,"pin":null,"kind":null,"scope":null}

event: scenario
data: {"action":"activate","name":"payments-down","active":["payments-down"]}

event: lagged
data: 37
```

- `id` is the request's [id](observability.md#the-request-id) (also the response's `X-Request-Id`), `seq` its number, and `client_id` the sanitised `X-Request-Id` the client sent (or `null`). `endpoint` is the matched route template, `null` when no route matched. The event carries no hops: ask `GET /trace/{id}` for those.
- `rejected` is `null` for a request the call chain answered, otherwise `unmatched` (a `404` or `405`), `bad-scope`, `engine-stopped` or `client-gone` (the client left first; status `499`). Every request that gets an id produces exactly one `request` event; preflights and control calls produce none.
- `ms` is measured wall time, from the moment the request got its id; the per-hop _simulated_ breakdown is in the response's `X-Kaanal-Trace` header (parallel branches in `[a|b]`, cache lookups as `hit`/`miss`, long chains ending `,...+N hops`) and in the [log entry](observability.md).
- `kind` is `reset`, `malformed` or `timeout` when the request failed that way (a reset or cut-off body still answers `200`, so the status alone does not show it); `null` otherwise.
- `request` events carry `pin`: `null`, or the state a pin forced on the request (`"loading"`, `"empty"`, `"error"`, `"slow"`, `"success"`). A pinned request has `failed_node: null` even when the chain failed.
- `request` events carry `scope`: `null` for a request without `X-Kaanal-Scenario`, otherwise the stack it was answered with (`""` for an empty header).
- `scenario` events carry the `action` — `activate`, `deactivate`, `set` (`PUT /scenarios/active`), `clear` (`DELETE /scenarios`), `reset`, `update` (the definition of an active scenario changed) or `expire` (a `ttl` lease ran out and the scenario was deactivated), `pin` (`PUT` or `DELETE /pins`, `name` is the endpoint or `null` for a clear-all) or `cycle` (`POST /cycle`, `name` is the endpoint) — the `name` it concerns (or `null`) and the `active` stack afterwards.
- A live chaos lease that runs out (a `PUT /chaos` with `ttl`) is an `expire` too, with `name: null` and an extra `chaos_key`, the key whose units were cleared: `{"action":"expire","name":null,"active":[],"chaos_key":{"node":"db"}}`. `chaos_key` is an object with only the parts the key has, named as in the `PUT /chaos` body and the `DELETE /chaos?node=&endpoint=` query: `{"node":"db"}`, `{"endpoint":"GET /x"}` or `{"node":"db","endpoint":"GET /x"}`. Only these events carry `chaos_key`.
- `lagged` means the stream fell behind and skipped that many events.
- `shutdown` is the last message of a stream: it is sent once, with `{ "timeout_ms": N }` (the time left in the drain), when a [shutdown](#post-__kaanalshutdown) starts, and the stream then ends. A client that sees the stream end without it should treat that as a shutdown too (with a tiny budget the connection can be cut before the message is written). `kaanal logs -f` does this: a `shutdown` event means the server is gone (exit 2 once the process has exited), and so does a stream that ends without one and whose instance lock is free after a short wait. The web UI closes the stream and shows "Server stopped" instead of reconnecting. The events of requests that are cut at the end of the drain are not delivered.

### `POST /__kaanal/shutdown`

Stops the instance gracefully: it stops accepting connections, lets requests already in flight finish for the budget, cuts what is left, removes `.kaanal/run.json` and exits 0. `kaanal down`, `SIGTERM` and Ctrl+C take the same path; see [lifecycle](lifecycle.md) for the whole story.

The body is optional. A missing, empty or whitespace-only body means a 5 s budget:

```json
{ "timeout": "30s" }
```

`timeout` is a duration string (`500ms`, `5s`, `1m`), 1 ms to 300 s. It is the only key.

```http
HTTP/1.1 202 Accepted
content-type: application/json

{"shutting_down":true,"timeout_ms":30000}
```

The first call fixes the deadline. A second call that still reaches the server (on a connection that was already open; new connections are refused once the drain starts) changes nothing: it answers `202` with `timeout_ms` set to the time left, and never extends the deadline.

| Status | When                                                                                                                         |
| ------ | ---------------------------------------------------------------------------------------------------------------------------- |
| `202`  | The drain has started (or was already running).                                                                              |
| `400`  | The body is not a JSON object, has a key other than `timeout`, or `timeout` is not a duration string between 1 ms and 300 s. |
| `401`  | Missing or wrong token.                                                                                                      |
| `403`  | A `Host` header that is not `localhost`, `127.0.0.1` or `[::1]`.                                                             |
| `405`  | Any method other than `POST`.                                                                                                |

A `400` stops nothing: the server keeps running.

**A tiny budget can lose the `202`.** The deadline is fixed before the answer is sent, so with a budget of a few milliseconds the connection can be cut before the `202` reaches you. A client that gets a transport error from this call has no way to tell that from a server that was already gone; `kaanal down` reads it as "the drain is running" because it still holds the instance lock, and waits for the process to end. Do the same: check that the process is gone rather than retrying the call.

Every open [`/__kaanal/events`](#get-__kaanalevents) stream ends at once with a final `shutdown` message, so a status page or a follower never holds the drain open.

```bash
curl -X POST -H "Authorization: Bearer $TOKEN" -d '{"timeout":"10s"}' "http://127.0.0.1:$PORT/__kaanal/shutdown"
```

### `GET /__kaanal/ui`

The embedded status page (HTML). No token in the request — the page reads it from the URL fragment (`#token=…`), which browsers never send to servers. See [web-ui.md](web-ui.md).

## Example: drive chaos from a test setup

```bash
TOKEN=$(jq -r .token .kaanal/run.json)
PORT=$(jq -r .port .kaanal/run.json)
curl -X PUT -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
     -d '{"down": true}' "http://127.0.0.1:$PORT/__kaanal/chaos/db"
# ... run the test ...
curl -X DELETE -H "Authorization: Bearer $TOKEN" "http://127.0.0.1:$PORT/__kaanal/chaos"
```

Or start every test clean and switch on a [scenario](scenarios.md):

```bash
curl -X POST -H "Authorization: Bearer $TOKEN" "http://127.0.0.1:$PORT/__kaanal/reset"
curl -X POST -H "Authorization: Bearer $TOKEN" "http://127.0.0.1:$PORT/__kaanal/scenarios/payments-down/activate"
```
