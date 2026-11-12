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

**Start of an outage (`from_ms`).** `down: true` on a node or one of its instances may carry `from_ms`, the moment of the run (milliseconds from its start) the outage begins; the simulator brings the node down then, and a balancer's `detect` counts from it (see [scenarios](scenarios.md)). Without it the node is down from the start. A PUT that sets `down` without `from_ms` drops an earlier start, and `down: false` removes the entry's start with the outage. `from_ms` without `down: true`, or on an endpoint or a link, is a `400`. The live mocks plan with a clock too: the time since the running system started (an Apply starts it again; a live parameter edit keeps it), so a `from_ms` outage begins that many milliseconds after the system started, and a balancer's `detect` counts from it, for real requests as in a simulation. The live stream's `hello` and every `live` tick carry that clock as `system_ms`, so a page can set an outage "N seconds from now" as `from_ms = system_ms + N * 1000`. The entry in `GET /__kaanal/chaos` shows `from_ms` when set.

To make a **link** misbehave, name an edge instead: `{ "edge": { "from": "app", "to": "db" }, "latency_ms": 20, "jitter_ms": 5 }`. An edge takes no `node`, `endpoint` or `replica` (`400`) and must exist (`404`: `no edge 'a' -> 'b'`). It takes `latency_ms`, `jitter_ms` (up to that much extra either way), `error_rate` with `error_kind` `reset` (the default) or `hang` (never `status` or `malformed`, and no `error_status`), `down` (a partition) and `bandwidth_cap` (bytes per second as a number, or a string with a unit such as `"500KB/s"`, the same as in `kaanal.yaml`: units `B`, `KB`, `MB`, `GB`, KB = 1024 bytes, `/s` optional, a fraction rounds to the nearest byte; only on an edge that has a `bandwidth`, and at most that value). A string that is not a size or a rate (`"5XB"`, `"-5KB"`, `"KB"`) is a `422` naming the field and the units; a cap of 0 after conversion is a `400` like a plain `0`. `kaanal chaos set --bandwidth-cap 500KB/s` takes the same text. `jitter_ms` and `bandwidth_cap` on any other key are a `400`. The same `ttl` and `mode` rules apply; `jitter_ms` is leased with `latency_ms` and `bandwidth_cap` as its own `bandwidth` unit. `GET /__kaanal/chaos` lists them under `edges` as `{ edge: { from, to }, ... }`; clear one with `DELETE /__kaanal/chaos?edge_from=app&edge_to=db`. What it does to a call over the edge: `latency_ms` and `jitter_ms` change only the network hop (jitter is uniform in plus or minus its value, never below zero); `error_rate` drops a share of calls as a `reset` (a 502 at the call) or a `hang`; `down` is a partition, a hang: the caller waits the callee's `timeout` (else 60 s) and gets a 504; `bandwidth_cap` lowers the link speed, so a payload takes longer. A link to a cache that drops a lookup is a miss, never a failure.

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

**Reset.** `POST /__kaanal/reset` deactivates every scenario, **undefines the runtime-defined ones** (inline and file scenarios stay), clears the live chaos, zeroes the per-route request counters (so a seeded run repeats), refills rate-limit buckets, empties every [collection](collections.md), clears every [response edit](#payload-edits) and empties the [request log](observability.md) (request ids keep counting). Queues and pools are not reset; requests already in flight finish with the chaos they started with. Call it between tests.

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

### Payload edits

A live edit rewrites what one route answers: its status, headers and body (JSON; text with a `content_type`; or bytes with `encoding: "base64"`, the body then being base64 and the `content_type` required), from the next request on; `once: true` makes it answer one request only. It is what Studio's [Admin mode](admin-mode.md) edits. It sits below chaos and pins and above the route's `override:`; a request that names a scenario scope never sees it. All routes need the token.

| Method and path                            | Purpose                                                                                                                                       | Success                                                 |
| ------------------------------------------ | --------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------- |
| `GET /__kaanal/payload`                    | The edits, in route order                                                                                                                     | `200 { "edits": [ { route, status, body, headers } ] }` |
| `PUT /__kaanal/payload`                    | Set the edit of one route                                                                                                                     | `204`                                                   |
| `DELETE /__kaanal/payload?route=KEY`       | Back to normal for one route                                                                                                                  | `204`                                                   |
| `GET /__kaanal/instances`                  | The cloud instance table `instance:` reads: `{ types: [[name, vcpu, memory_mib, net_mbps], ...] }`, sorted by name (`net_mbps` 0 = no figure) | `200`                                                   |
| `GET /__kaanal/prices`                     | The regions of each cloud the cost estimate can price in, `{ defaults, regions }`.                                                            |                                                         |
| `GET /__kaanal/prices/{provider}/{region}` | One region's prices, `{ provider, region, prices: [[name, usd_micro_hr], ...] }` (404 for an unknown provider or region).                     |                                                         |
| `POST /__kaanal/payload/stage-file`        | Write the edit (the body of `PUT`) into the endpoints file the draft names; `endpoints:` must be a file, not inline                           | `200 { "file" }`                                        |
| `GET /__kaanal/payload/preview?route=KEY`  | What the route answers with no edit (a starting text; path parameters are filled with `1`)                                                    | `200 { status, body, headers }`                         |

**Body** of `PUT`: `{ "route": "GET /users/{id}", "status": 503, "body": { "down": true }, "headers": { "retry-after": "5" } }`. `route` is required; `status` defaults to 200, `body` to none and `headers` to none. Unknown keys are `400`.

| Status | When                                                                                                                                                                                                                                                                                                                                                              |
| ------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `400`  | The body is not JSON or has an unknown key; a missing or malformed `route` in the query                                                                                                                                                                                                                                                                           |
| `404`  | No such route (`PUT`), no edit on the route (`DELETE`), unknown route (`preview`)                                                                                                                                                                                                                                                                                 |
| `413`  | The body is over 1 MiB                                                                                                                                                                                                                                                                                                                                            |
| `422`  | `{ "error", "field" }`: the edit is refused, and `field` is `status` (not 200 to 599, or a body on a `204` or `304`), `body` (over 1 MiB), `headers` (over 32, a bad name, a value over 4096 bytes or with a control character, or a name Kaanal sets itself) or `route` (a `collection:` route, a built-in login or key-set route, a `HEAD` made from its `GET`) |

`stage-file` validates like `PUT`, patches the file with the comment-keeping splice and writes it atomically (`409` when the draft's `endpoints:` is inline, the file changed meanwhile, or the result would not be an endpoints file); the running system keeps its live edit until the next Apply reads the file. A refused `PUT` changes nothing. An Apply keeps the edits whose route still takes one and lists the others as `payloads_dropped` in its reply. `POST /__kaanal/reset` clears every edit.

### Draft

The draft is the text of the `kaanal.yaml` being edited, kept apart from the running system and from the disk. Editing it changes neither; Apply and Save do. It starts as the config file's text exactly as `kaanal up` read it (comments and line endings included); a server started with no file starts with an empty draft.

`GET /__kaanal/draft` returns `{ "text", "dirty", "generation", "lost", "rev", "doc", "doc_error", "project_id", "project_name", "kind_defaults", "kinds", "layout", "layout_note", "layout_dirty", "saved", "renames", "boot" }`. `renames` lists the `rename_node` edits made since the text was set, as `[from, to]` pairs in order (a whole-text `PUT`, Reload or restart empties it): it is what `fix_scenarios: "rewrite"` on Save follows. `project_name` is the name of the folder the config file sits in (a base name, never a path, at most 80 characters; `null` with no file), which Studio shows as its heading. `boot` is a random id drawn once per server process: `rev` restarts at 0 with the process, so Studio treats a changed `boot` as a restart and drops what it held for the old one. `layout_dirty` is true when the positions differ from the sidecar, so a Save would write them (a layout edit never makes `dirty` true). `doc` is the draft text as JSON (the file's own keys: `nodes`, `edges`, ...), or `null` with `doc_error` (the diagnostic `POST /draft/validate` gives: `line`, `col`, `message`) when the text is not a valid config; the status stays 200. `project_id` is 16 hex characters derived from the config file's path (never the path; `none` with no file). `kind_defaults` is, per node kind, the node Studio creates (`type` and its default capacity). `PUT /draft` answers with the same fields. `GET /__kaanal/demo` returns `{ "text": ... }`, Studio's self-contained starter system (no spec or site file; it has a `peak` profile, so a draft run works at once). `dirty` is true when the text differs from the last applied one, and `generation` counts the systems applied (0 before the first Apply of a blank start).

`kinds` is the rules of each node kind from the engine: `[{ "kind": "gateway", "callees": ["service", "monolith", "auth"], "service_time": true }, ...]`. Studio offers only the wires `callees` allows and shows a Time control only when `service_time` is true (`storage` says false: its time is `first_byte`, shown with its `throughput`), so a kind added to the engine needs no copy of these rules in the page (it still needs an icon, a colour, a label and a glossary entry there).

`PUT /__kaanal/draft` with `{ "text": "..." }` replaces the draft and returns the same view. The text is **not** validated, so a half-typed file can be stored. Last writer wins (one user; two tabs overwrite each other). A body with both `text` and `patch`, or none of `text`, `patch` and `layout`, is `400`, and one of the wrong shape is the usual `422`; a text over 1 MiB (1048576 bytes) is `400` and the draft is unchanged (the text is what is counted, not the JSON: a body may be up to about 6 MiB so that a text of control characters, six bytes each once escaped, still gets that answer). Both routes need the token.

**Patches.** `PUT /__kaanal/draft` also takes `{ "patch": [ { "op": "set", "path": ["nodes", "db", "concurrency"], "value": 8 } ] }` instead of `text`: the edits are made on the draft text one after the other, and only the bytes they touch change, so the comments, the order of keys and the flow style (`{ from: a, to: b }`) of the file stay as they are. `op` is `set` (replace the value at `path`), `insert` (the last step of `path` is the new key, the rest is the mapping it goes into; `["nodes", "cache"]` adds `cache:` under `nodes:`), `append` (`path` is a list, `value` is the new last item, written in the list's own style: a new `-` line under the others in a block list, `, item` in a flow list; a missing path is `400`, and what is not a list, an empty block list or an anchored list is left as it was and listed in `lost`) `swap` (`path` and `with` are two different items of one list; they trade places as text, each keeping its own comments and the text between them stays put; a `with` that is missing or not in the same list is `400`, and an anchored item is left as it was and listed in `lost`), or `remove` (a mapping entry or a list item). A `path` step is a key (text) or a list index (a number). A `value` is JSON and is written as YAML in the file's style: bare words where that is safe (`db`, `2ms`), quotes where a bare word would mean something else (`"yes"`, `"8"`), `{ type: cache, flavor: redis }` for an object (in the order sent), `[a, b]` for a list. The answer is the usual view; `lost` lists what a patch could not keep: a value with an anchor or an alias (changing it would change its copies) is left as it was, a comment directly above a removed entry stays behind, a comment between `key:` and its block value goes with the value, and removing the only entry of a block mapping leaves what held it empty (null). An edit inside an anchored value is refused too, not only on it. A key that `insert` adds is written bare only when that reads back as the same text; otherwise it is double-quoted, so a key can never carry syntax. A `null` value is a value (`"value": null` sets null); only a missing `value` is an error. A duplicate key anywhere in the draft makes a patch `400` (which one would it edit?). A patch that cannot be made (nothing at the path, a key already there, a draft that is not YAML, a missing `value`) is `400` and the draft is unchanged. Replacing the whole `text` clears `lost`.

**Two operations for building a system.** `{ "op": "add_node", "name": "cache1", "kind": "cache" }` adds a node under `nodes:` with the capacity and the fields its kind needs to be valid (gateway `concurrency: 256`, service 32, monolith 64, database `pool: { size: 20 }` with `flavor: postgres`, cache with `flavor: redis`, external-api `pool: { size: 10 }`, auth 32; see [nodes](nodes.md#nodes-added-in-studio)); an unknown kind, a missing `name` or `kind`, or a name that exists is `400`. `{ "op": "rename_node", "from": "db", "to": "store" }` changes the node's name everywhere it is written in the draft, in one step: the `nodes:` key, `edges` (`from`, `to`), the steps of node `chains:` and of inline `endpoints:` `chain:` lists (`call`, `cache`, or a bare name), the keys of an endpoint's `error_map`, and inline `scenarios:` (`chaos[].node` and the ends of `edge`). The answer has `touched`, one entry per place changed (`"nodes.db"`, `"edges[1].to"`, ...). It is refused (`400`, nothing changed) when the old name is not a node, the new one is (or is empty or the same), or a reference is inside an anchored value (it cannot be changed without changing its copies). Files outside `kaanal.yaml` are not part of the draft and are not rewritten: an `endpoints:` file is reported in `notes`, and `scenarios/*.yaml` files that still name the old node are flagged by Apply (see `fix_scenarios` under Save). An Apply treats a rename as a removal plus an addition, so live chaos on the old name is dropped and reported like any removed node.

**Layout.** Clearing every position cannot be persisted: an empty layout deletes nothing, so Save leaves an existing `kaanal.layout.json` as it is. Where Studio draws each node is kept out of `kaanal.yaml`, in `kaanal.layout.json` beside it (a fixed name; no request names a path): `{ "version": 1, "nodes": { "gw": [120, 80] } }`. `layout` in the view is that map, node name to `[x, y]`; positions of nodes the text does not define are left out (only when the text is a valid config, so nothing is dropped while it is half-typed). `layout_note` is `null`, or says why the file was not used when the draft was made: missing is no note; unreadable, not JSON, an unknown version or key, a bad position, over 256 KiB, or a link or anything but a regular file (not followed) means no positions and a note, never an error. `PUT /__kaanal/draft` takes `layout` alone or with `text` or `patch`; it replaces the whole map. Every coordinate must be within ±1000000, at most 2000 positions, no empty name, and the file it would make at most 256 KiB; otherwise `400` and nothing in the draft changes (the text included). A layout change never changes `rev` or `dirty`, so moving a node does not re-run a simulation. `rename_node` moves the node's position with it. Last writer wins: there is no conflict check for the layout file. Clearing every position cannot be persisted: Save with an empty layout writes nothing and deletes nothing, so the old `kaanal.layout.json` stays (`layout_note` says so).

**Files named in a draft stay inside the project.** A draft arrives through the API, so `spec:`, `data:` and `endpoints:` in it must be relative paths inside the project folder: an absolute path or one with `..` is refused (even for a file that exists), as is one that leads out through a link, and the answer repeats only what the draft said. A spec or endpoints file must be a regular file of at most 16 MiB. Messages and `flagged` files name paths relative to the project, never the machine's layout. (`kaanal up` is unchanged: a config file you wrote yourself may point wherever you can read; a project that uses `spec: ../shared/api.yaml` starts, but cannot be Applied until the file is inside the project or linked into it.)

`POST /__kaanal/draft/validate` builds the draft off to the side with the loader `kaanal up` uses (parse, topology, specs, endpoints, scenarios) and answers `{ "ok": true, "diagnostics": [], "warnings": [...], "flagged": [...] }` (`flagged` as under Apply), or `{ "ok": false, "diagnostics": [ { "file": null, "line": 9, "col": 14, "path": ["edges", 1, "to"], "message": "..." } ], "warnings": [] }`. A problem in the draft is data: the status is `200` and `ok` is false. The loader stops at the first error, so there is at most one diagnostic. `line` is 1-based and `col` 0-based; both are `null` when the error has no place, and `file` names the file when the error is in a spec or the endpoints file rather than `kaanal.yaml` (then `line` and `col` are that file's, or `null`). With a body `{ "text": "..." }` it validates that text instead and stores nothing. It never swaps the running system, never starts an engine, and does not touch the draft. Over 1 MiB is `400`.

`POST /__kaanal/draft/apply` makes the draft the running system, without a restart. It builds the draft (as validate does), then **holds mock traffic back**, waits for the mock requests already in flight to finish, swaps the system and lets traffic go on. A request that arrives while it drains waits and then runs on the new system; one already running finishes on the old. Control-API requests are never held. The body is optional: `{ "timeout_ms": 10000 }` is how long to wait for the drain (default 10 s, at most 60 s).

```
{ "ok": true, "generation": 2,
  "kept": { "live_chaos": 1, "active_scenarios": ["s1"], "runtime_scenarios": ["x"], "pins": 1 },
  "dropped": { "live_chaos": ["node 'db'"], "scenario_chaos": [], "active_scenarios": ["payments-down"], "pins": [] },
  "flagged": [] }
```

- **What changes.** The topology, routes, endpoints, specs, data, auth issuer and engine are new; `generation` counts up. Route counters start again at zero (so a seeded sequence restarts), the `collection:` items are emptied (they belong to the system that defined them), and rate-limit buckets and queues start empty. An Apply always rebuilds from disk, so it also picks up an edited spec, data or scenario file, even when the draft text did not change. The control token, key files and port do not change. The scenarios are reloaded from the new draft and `scenarios/`.
- **What is kept, by name.** Live chaos, its `ttl` leases, the runtime scenarios (those made with `PUT /scenarios/{name}`, which are in no file), the active scenarios and the live pins are carried over where what they name still exists: a chaos key by node name (so reordering nodes is harmless), an edge by its two node names, a replica only while the node still has it, an endpoint key only while the route exists, a scenario only while the new draft (or `scenarios/`) still defines it, a pin only while its endpoint exists (a pin on `empty` also needs the endpoint to still have an empty form). A rename is a removal plus an addition. `kept` counts them: `{ "live_chaos": 1, "active_scenarios": ["s1"], "pins": 1 }`; `dropped` lists what went: `{ "live_chaos": ["node 'db'", "node 'db' replica 2"], "active_scenarios": ["s2"], "pins": ["GET /users/{id}"] }`. A leased unit on a dropped key loses its lease with it. What is kept is also checked the way the API checks a new entry: a `bandwidth_cap` on an edge that no longer has that much bandwidth is removed (the rest of the entry stays; it is listed as `... bandwidth_cap` under `dropped.live_chaos`), a pin or endpoint chaos on a route that is now static with no entry node, or only a `HEAD` made from its `GET`, is dropped. A runtime scenario is carried the same way (its dropped parts are listed under `dropped.scenario_chaos` as `scenario NAME: ...`); if the new draft defines a scenario of the same name, the draft's definition replaces it, and that is listed too.
- **`flagged`.** A `scenarios/*.yaml` that names a node, edge or endpoint the new system lacks does not fail the Apply (or validate): it is left out of the new system and listed: `[ { "file": "...scenarios/f.yaml", "scenario": "f", "message": "..." } ]`. The file stays on disk, and startup is strict, so the next `kaanal up` fails on it until you fix or remove it. Scenarios defined inline in the draft are part of it and stay strict.
- `400` when the draft does not build: `{ "error": "...", "diagnostics": [ ... ] }` as in validate, and nothing changes. `400` also for a `timeout_ms` over 60 s.
- `409` `drain timed out; the old system keeps running` when requests did not finish in time: traffic resumes at once, the draft is kept, and nothing else changes. Apply again later, or with a longer `timeout_ms`.
- **What holds the drain.** Any mock request in flight, from the moment it arrives until its answer is built: a long chain, a `hang` chaos (up to 60 s), or a client that sends its body slowly. If an Apply keeps ending in `409`, wait for those or raise `timeout_ms`.
- One Apply at a time; a second waits for the first. A control request that was working on the old system when the swap happened (for example a scenario save) answers `409` `the system changed while this request ran; retry`. The reads that pair chaos with the system (`GET /chaos`, `GET /scenarios/active`, `GET /scenarios/{name}`) never answer that: they answer from whichever system is in force once they hold the control lock.
- After an Apply, `dirty` on the draft is false and `generation` is the new one; the `events` stream carries an `applied` message.

### Simulation

Run the simulator on the applied system and read the report. See [simulation.md](simulation.md) for the body, the limits and how to read a summary, and [traffic.md](traffic.md) for profiles.

| Route                             | Answer                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| --------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `POST /__kaanal/sim/run`          | Starts a run; `source` is `applied` (default) or `draft` (the draft text, no Apply; its own `scenarios:` count). `202 { "id", "state": "running" }` with `Location: /__kaanal/sim/runs/{id}`. `400` for a bad body, a profile that names an unknown endpoint, or a request over the server caps (the message names the cap); `404` for an unknown profile or scenario; `409 { "error", "id" }` while another run is in progress, or `409` with no system (blank start; not for `source: "draft"`); `429` while two draft builds, samples or skeletons are in progress; a draft that does not build is `400 { "error", "diagnostic" }`.                                                                                                                                                                                                  |
| `GET /__kaanal/sim/runs`          | `{ "runs": [...] }`: the last eight runs (oldest first) without their summaries.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| `GET /__kaanal/sim/runs/{id}`     | One run: `state` (`running`, `done`, `failed`), `generation`, `source`, `draft_rev` (the draft's `rev` for a draft run, else `null`), `profile`, `scenario`, `started_ms`, `finished_ms`, and `summary` or `error`. `404` once pushed out of the last eight.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                            |
| `GET /__kaanal/sim/runs/{id}/log` | A page of the run's log sample: `{ "lines", "next", "total", "sampled": true, "note" }`. Query: `node`, `replica`, `endpoint` (`METHOD /path`), `level` (`info`, `warn`, `error`), `request` (a request counter, which is per stream, so pair it with `endpoint`), `limit` (default 200, at most 1000), `after` (index of the last line seen; `next` is the `after` of the next page, `null` at the end). `total` counts the stored lines before the filters. A line has `t_ms`, `level`, `request`, `stream`, `node`, `replica`, `event`, `wait_ms`, `service_ms`, `status`, `note`; `replica`, `wait_ms` and `service_ms` are `null` on a failure line (the engine does not record them), so a `replica` filter skips those. `400` for a bad `level` or `endpoint`; `404` for an unknown run; `409` while it runs or after it failed. |
| `POST /__kaanal/sample`           | `{ "endpoint": "GET /users/{id}", "seed"?: 7 }` answers `{ endpoint, status, body, bytes, label }`: one body from the generator the mock uses. `404` for an endpoint that is not a route; `400` over 256 KiB; `409` with no system.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| `GET /__kaanal/openapi/skeleton`  | `{ "text", "endpoints" }`: a generated OpenAPI 3.0 file with one operation per key of the **draft's** `endpoints:` (inline, or an endpoints file inside the project). `409` when the draft has no endpoints, has more than 10000, or a key is not `METHOD /path` or has a malformed `{parameter}`; `400` when the draft does not parse or the endpoints file is outside the project or unreadable.                                                                                                                                                                                                                                                                                                                                                                                                                                      |

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

### The live simulation: `GET /__kaanal/live` and its commands

A continuous simulation on the same engine the mocks run on (Studio's default view). Traffic is generated from the first `traffic:` profile of the running system (Poisson arrivals at its `rps`, split by journey weight), so a real request to a mock and a generated one meet the same queues, limits, `cpu`, retries and breakers.

`GET /__kaanal/live[?fresh=1]`: a server-sent events stream. The token may be given as `?token=` here (as on `/events`; the browser `EventSource` cannot set headers); the commands below need the `Authorization` header. The first client starts a **session**; `fresh=1` (a page load) resets it, an automatic reconnect (no `fresh`) keeps it; two clients share one session; the session stops **5 seconds** after the last client leaves, so a closed page leaves nothing running (the mocks and the engine keep serving). Events:

- `hello`: the session (`session`, `profile`, `note` when nothing can be generated (no profile, a route that does not exist), `rate`, `paused`, `overloaded`, `in_flight`, `submitted`, `clients`).
- `live`, every 250 ms: `{ t_ms, requests, failed, nodes: [{ arrived, dropped: { overflow, rate_limit, deadline, breaker, chaos }, queue, busy_slots, cpu_load }], wires: [{ carried, wait_ns, wire_ns, retries, retried }], hist: [[bucket, count]], journeys: [...], status: { rate, paused, overloaded, in_flight } }`. Counts are integers and sums, so a window of ticks is added up on the page; `hist` is a log-spaced latency histogram (1% precision, 10 microseconds to 1,000 seconds, occupied buckets only) of the requests that were served in the tick. `journeys` are at most 10 finished requests a tick (about 40 a second), each with its hops `{ node, replica, kind, start_ns, end_ns, wait_ns, attempt, edge }` and `dropped_at` for a dropped one; a generated journey also has `stream` (an index into the `streams` of the `hello`, the routes of the offered mix) and a dropped one has `cause` (`overflow`, `rate_limit`, `deadline`, `breaker`, `oom` or `chaos`). A wire's `wait_ns` is the time requests spent waiting to get onto it and `wire_ns` the time on it, both summed over the crossings in the tick, so `wire_ns / carried` is the mean crossing. `retries` counts the crossings that were a retry (attempt 2 or later, counted on the wire the first try came in through) and `retried` the calls that made at least one, so the calls are `carried - retries`. A node also carries `hist` (the work times of its own hops, waiting excluded, as occupied `[bucket, count]` pairs of the same buckets as the tick's `hist`) and, when it ran on more than one copy, `replicas` (own hops per copy); both are left out when empty. `queue` and `busy_slots` are read at the instant of the tick; `cpu_load` is running hops over cores x copies for a node with `cpu`.
- `marker`: `{ at_ms, kind, text }` for a `param` edit, a `chaos` change, `pause` or `resume`.
- `resync`: sent once after the client fell behind and missed events (the status as `hello`); the page keeps its timeline with a gap.

The stream ends when the session ends (an Apply restarted it, the server shuts down); the browser reconnects and gets a new `hello`.

`POST /__kaanal/live/pause`, `POST /__kaanal/live/resume` (the generator stops or restarts; the engine keeps serving and draining), `PUT /__kaanal/live/rate` with `{ "rps": N }` (at most 20,000 a second; above it, a negative or a non-number is a 400 and the rate stays). All answer with the session status, `409` when no page is connected. A generator stops adding requests when 200,000 are unfinished (`overloaded: true`), so an overloaded node with no queue limit cannot grow without bound.

**Parameter edits go straight in.** While a page follows the live run, a change of the draft (`PUT /__kaanal/draft`) that only changes parameters is applied to the running engine at once, without Apply and without a drain: queues, running work and breaker state carry over (`cpu`, slot limits, rate limits, node cards and wire bandwidth, copy counts, service times, chaos-independent settings). The reply gains `live`: `{ "live": "applied", "changed": ["app"], "structure_pending": false }`, `"unchanged"` (nothing running changed; it may still hold structure edits), `"none"` (no page is connected: the draft waits for Apply as before) or `"invalid"` with `message` (the draft does not parse, a value was refused: the running system is as it was). Structure edits (a component or wire added or removed, a rename, a changed kind) stay in the draft until `POST /__kaanal/draft/apply`, which restarts the live run; a patch that mixes both applies its parameter part and leaves the structure part. The file on disk changes only on `POST /__kaanal/draft/save`.

### `GET /__kaanal/events`

Server-Sent Events stream of every mock request as it completes, of scenario changes, and of each Apply of the draft (`applied`, with the new `generation`).

```
event: request
data: {"id":"r19a2-0001","client_id":"checkout-17","seq":1,"rejected":null,"ts_ms":1792345019677,"method":"GET","path":"/users/7","endpoint":"GET /users/{id}","status":200,"ms":25,"nodes":["nginx","app","db"],"failed_node":null,"pin":null,"kind":null,"scope":null}

event: scenario
data: {"action":"activate","name":"payments-down","active":["payments-down"]}

event: applied
data: {"generation":2}

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

`POST /__kaanal/draft/save` writes the draft text to the config file the server was started from. The body is optional: `{ "confirm_loss": true, "fix_scenarios": "none", "overwrite": false }`. It answers `{ "ok": true, "written": ["kaanal.yaml"], "removed": [], "layout_written": true, "layout_note": null }`. Save does not touch the running system (that is Apply), and nothing is written to disk until Save: patches, validate and Apply leave the file alone.

- **`fix_scenarios`** says what to do with the scenario files the draft no longer resolves against (the `flagged` ones): `"none"` (the default) leaves them, so the next `kaanal up` fails on them until you fix them; `"remove"` deletes them (only regular files directly in the project's own `scenarios/` folder, which must not itself be a link; they are listed under `removed` as `scenarios/NAME`), so the next `kaanal up` loads; `"rewrite"` points the flagged scenario files at the renamed node: the draft remembers the `rename_node` operations made since its text was set (a whole-text `PUT` forgets them), and Save changes each of those names in the files' `chaos[].node` and `chaos[].edge` ends, chained in order (`a -> b -> c` ends at `c`), as part of the same all-or-none write; a file that names none of them stays flagged, a file with a name inside an anchored value stops the Save (`400`, nothing written), and `rewrite` with no rename recorded is `400`. The files to delete are worked out from the very text that is written, so a `PUT` that lands during a Save cannot change which files go (the saved text is what was checked; the newer draft stays and the next Save writes it). `remove` on a draft that does not build is `400`. A Save that is refused (conflict, `lost`) deletes nothing.
- **Every file changes or none.** The new files are written to temporary files first; then the removals and the config are made in order (the config last), each remembering what it replaced, and if one fails the ones already made are put back (if even that fails, the error names the files that could not be put back). A file that cannot be read stops the Save before anything changes.
- The file is replaced **atomically**: the text goes to a temporary file in the same folder, is flushed to disk, and is renamed over the config, so a reader (or a crash) sees the old file or the new one, never half. The file keeps its permissions, and a config that is a link is written through to its target (the link stays).
- `409` with `"conflict": true` when the file on disk is not what Studio last read or wrote (an editor or `git checkout` changed it, it was deleted, or it appeared where there was none). Nothing is written. Two ways out: send the same Save with `"overwrite": true` (the draft replaces the file, a deleted file is written again), or `POST /__kaanal/draft/reload` (below) to take the file and drop the draft. What is compared is the text of the file, so touching it without changing it is not a conflict.
- `409` with `"lost": [...]` when patches could not keep something (see Patches) and `confirm_loss` is not true; nothing is written. With `confirm_loss: true` the draft is saved as it stands, and `lost` is cleared.
- `409` when the server was not started from a config file; `500` when the write fails (every file is as it was, and no temporary file is left, unless the error says some file could not be put back).
- **Save holds the same path rule as validate and Apply.** A draft whose `spec:`, `data:` or `endpoints:` is absolute, has `..`, or leaves the project through a link is `400` and nothing is written (otherwise the next `kaanal up`, which takes paths as written, would trust what the API stored). Text that is not YAML at all is not checked: nobody can load it.
- The temporary file has a name that cannot be guessed and is created exclusively, so a file or link planted at that path is never written through.
- The draft text is saved as it is, valid or not (a half-typed file can be saved); `kaanal up` on a file that does not load fails with its usual located error.
- **The layout** is written after the config succeeds, to `kaanal.layout.json` in the config file's folder, by the same temporary-file-and-rename, and the answer gains `"layout_written": true|false` and `"layout_note"` (why not, or `null`). Positions of nodes the saved text does not define are dropped (when it is a valid config). An empty layout writes nothing and removes nothing. If `kaanal.layout.json` is a link or not a regular file, it is not replaced, and if the config's folder resolves somewhere else than when the Save began, nothing is written there; either way the config write stands (it is not rolled back) and `layout_note` says so.

`POST /__kaanal/draft/reload` throws the draft away and reads the config file (and `kaanal.layout.json`) again, with no body. The draft is then the file (`saved` is true, `lost` and the recorded renames are cleared), `rev` moves when the text changed, and what is applied stays applied, so `dirty` says whether the file differs from it. It answers the draft view. `409` when the server has no config file or the file cannot be read (it is missing, say); `400` over 1 MiB. The draft is unchanged on an error.

**Blank start.** `kaanal up --blank` (or `kaanal studio`) starts with no system. The control API and `/__kaanal/ui` work; every other path answers `404 {"error":"no system applied yet"}`; what needs a system (topology, chaos, pins, scenarios) answers `409 no system applied yet; apply a draft`. The draft is `version: 1\nnodes: {}\n`, `generation` is `0` and `dirty` is true. Build it with `add_node` and the other patches, and `POST /__kaanal/draft/apply`: an empty draft is `400` with the topology's own message (an entry node is needed), and the first Apply that builds makes generation 1. `Save` writes the file named with `-c` (default `kaanal.yaml`): if that file exists when the server starts, Save writes over it only if it has not changed since then, like any Save. A blank start keeps a signing key in memory, so an `auth` node added later works; the project's own key file is made by `kaanal up` once the file is saved.
