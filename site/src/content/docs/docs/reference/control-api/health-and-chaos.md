---
title: "Health, topology and chaos"
sidebar:
  order: 1
---

## `GET /__kaanal/health`

Liveness probe. No token. Besides `ok` it says the little the [status page](/docs/guides/observe/status-page/) shows: the version, the uptime in whole seconds, and whether a system is loaded (`false` on a blank start before the first Apply). Nothing a token guards: no node names, counts, paths or figures. A probe that checks only the status code or `ok` is unaffected.

```json
{ "ok": true, "version": "0.1.0", "uptime_s": 83, "system": true }
```

## `GET /__kaanal/topology`

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

## `GET /__kaanal/chaos`

The **live layer**: chaos set through `PUT /__kaanal/chaos...`, in three groups (empty when nothing is set), plus `effective`, the result of the active [scenarios](/docs/reference/control-api/scenarios/) and the live layer together. (Before the failures release this was one flat map by node name.)

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

## `PUT /__kaanal/chaos/{node}`

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

**`ttl` (timed chaos).** A PUT with `"ttl": "30s"` leases what it sets, per **unit**: `latency` (`latency_ms`), `error` (`error_rate`, `error_kind` and `error_status` together), `down`, `mode`, and on an edge `jitter` (`jitter_ms`) and `bandwidth` (`bandwidth_cap`), each its own lease. A PUT without `ttl` makes the units it sets permanent again (a plain `{"down": true}` after a timed one keeps the node down), and units a PUT does not mention keep their lease, so `latency_ms: 200` set permanently plus `{"down": true, "ttl": "30s"}` leases only `down`. Sending a unit again with a new `ttl` replaces its deadline. A bad `ttl` (outside the range, not parseable) is `400` naming `ttl`, and nothing changes. The lease lives in the server, so it outlasts the client that set it. It ends with its field: `DELETE` of the entry, `POST /__kaanal/reset`, `down: false` (for `down`), or a `save` with `clear_live`. When the time is up the unit's fields are **cleared**, not restored to an earlier value (`mode` goes back to merge), an entry with no settings left (equal to an empty entry, no `mode` either) is removed while one that keeps a permanent bare `mode: merge` stays, and one `expire` event names the key (see [events](/docs/reference/control-api/live-and-events/#get-__kaanalevents)); units of one key that run out together are cleared in one step with one event. The answer and `GET /__kaanal/chaos` carry `expires_in_ms`. Leases are in memory only, and `scenarios/save` from live writes leased fields as permanent with a warning counting the leased fields (`"3 fields have a ttl; the file has none"`; `mode` is not counted).

**`mode: override`.** By default a live entry adds to the active scenarios (latencies sum, `down` ORs, error rates stack). With `"mode": "override"` the entry replaces everything lower layers contribute to that key, so `{"mode":"override","down":false}` or a bare `{"mode":"override"}` calms a scenario that takes the node down. A node override also calms that node's node-at-endpoint entries from scenarios; an endpoint or node-at-endpoint override affects only its own key, so a node-at-endpoint override cannot calm chaos set on the node itself (use a node override for that). The mode **sticks**: a later `PUT` without `mode` keeps it, and `"mode": "merge"` goes back to stacking.

## `PUT /__kaanal/chaos`

Like the per-node route, but the body also names where the chaos applies: `node`, `endpoint` (for example `"GET /orders"`; the method is case-insensitive) or both (the node only while it serves that endpoint). One of them is required (`400` otherwise); an unknown node or endpoint is `404`; an endpoint that is not `METHOD /path` is `400`. Entries from different layers stack: a node entry and a node-at-endpoint entry, or a live entry and a scenario, all apply (latencies add, `down` is an OR, error rates combine as independent chances). **A higher layer wins over a lower one** for the same field, whatever the key; inside one layer the node-at-endpoint entry goes before the node's. An endpoint entry is a separate roll at the front door, before the chain, and is not merged into the nodes behind it (see [chaos internals](https://github.com/Naveen2070/kaanal/blob/main/docs/source/chaos.md#endpoint-chaos-and-keys)). Answers like the per-node route.

To act on one copy of a replicated node, add `"replica": N` (0-based) next to `node`: `{ "node": "db", "replica": 1, "down": true }`. A replica needs a node, takes no `endpoint` (it applies whatever the endpoint) and must exist (`400` otherwise: `node 'db' has 3 replicas (0 to 2)`). A downed replica is skipped by the balancer from the request's arrival on; if every replica is down the call fails with a 503. A replica entry also carries the node's own entry and its node-at-endpoint entries. `GET /__kaanal/chaos` lists them under `replicas: [{ node, replica, ... }]` (the group is left out when there are none), `DELETE /__kaanal/chaos?node=db&replica=1` removes one.

**Start of an outage (`from_ms`).** `down: true` on a node or one of its instances may carry `from_ms`, the moment of the run (milliseconds from its start) the outage begins; the simulator brings the node down then, and a balancer's `detect` counts from it (see [scenarios](/docs/guides/break/scenarios/)). Without it the node is down from the start. A PUT that sets `down` without `from_ms` drops an earlier start, and `down: false` removes the entry's start with the outage. `from_ms` without `down: true`, or on an endpoint or a link, is a `400`. The live mocks plan with a clock too: the time since the running system started (an Apply starts it again; a live parameter edit keeps it), so a `from_ms` outage begins that many milliseconds after the system started, and a balancer's `detect` counts from it, for real requests as in a simulation. The live stream's `hello` and every `live` tick carry that clock as `system_ms`, so a page can set an outage "N seconds from now" as `from_ms = system_ms + N * 1000`. The entry in `GET /__kaanal/chaos` shows `from_ms` when set.

To make a **link** misbehave, name an edge instead: `{ "edge": { "from": "app", "to": "db" }, "latency_ms": 20, "jitter_ms": 5 }`. An edge takes no `node`, `endpoint` or `replica` (`400`) and must exist (`404`: `no edge 'a' -> 'b'`). It takes `latency_ms`, `jitter_ms` (up to that much extra either way), `error_rate` with `error_kind` `reset` (the default) or `hang` (never `status` or `malformed`, and no `error_status`), `down` (a partition) and `bandwidth_cap` (bytes per second as a number, or a string with a unit such as `"500KB/s"`, the same as in `kaanal.yaml`: units `B`, `KB`, `MB`, `GB`, KB = 1024 bytes, `/s` optional, a fraction rounds to the nearest byte; only on an edge that has a `bandwidth`, and at most that value). A string that is not a size or a rate (`"5XB"`, `"-5KB"`, `"KB"`) is a `422` naming the field and the units; a cap of 0 after conversion is a `400` like a plain `0`. `kaanal chaos set --bandwidth-cap 500KB/s` takes the same text. `jitter_ms` and `bandwidth_cap` on any other key are a `400`. The same `ttl` and `mode` rules apply; `jitter_ms` is leased with `latency_ms` and `bandwidth_cap` as its own `bandwidth` unit. `GET /__kaanal/chaos` lists them under `edges` as `{ edge: { from, to }, ... }`; clear one with `DELETE /__kaanal/chaos?edge_from=app&edge_to=db`. What it does to a call over the edge: `latency_ms` and `jitter_ms` change only the network hop (jitter is uniform in plus or minus its value, never below zero); `error_rate` drops a share of calls as a `reset` (a 502 at the call) or a `hang`; `down` is a partition, a hang: the caller waits the callee's `timeout` (else 60 s) and gets a 504; `bandwidth_cap` lowers the link speed, so a payload takes longer. A link to a cache that drops a lookup is a miss, never a failure.

```json
{ "endpoint": "GET /orders", "error_rate": 1.0, "error_kind": "reset" }
```

## `DELETE /__kaanal/chaos/{node}` / `DELETE /__kaanal/chaos`

Clear one node (`404` if unknown) or all **live** chaos. Both return `204`. `DELETE /__kaanal/chaos?node=&endpoint=` clears exactly the entry under that key (`node`, `endpoint` or both) and leaves the others; an unknown query parameter is `400` and clears nothing, a malformed `endpoint` is `400`, an unknown endpoint is `204` (nothing to remove) and an empty or unknown `node` is `404`. **Scenarios are not touched**; use `DELETE /__kaanal/scenarios` or `POST /__kaanal/reset`.
