---
title: "Health, topology and chaos"
sidebar:
  order: 1
---

## `GET /__kaanal/health`

Liveness probe. No token. Besides `ok` it says the little the [status page](/docs/guides/observe/status-page/) shows: the version, the uptime in whole seconds, and whether a system is loaded (`false` on a blank start before the first Apply). Nothing a token guards: no node names, counts, paths or figures. A probe that checks only the status code or `ok` is unaffected.

```json
{ "ok": true, "version": "0.1.1", "uptime_s": 83, "system": true }
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

### Timed chaos (`ttl`)

A `PUT` with `"ttl": "30s"` (`100ms`–`24h`) leases the units it sets:

| Unit        | Fields leased                              | Applies to |
| ----------- | ------------------------------------------ | ---------- |
| `latency`   | `latency_ms`                               | all        |
| `error`     | `error_rate`, `error_kind`, `error_status` | all        |
| `down`      | `down`                                     | all        |
| `mode`      | `mode`                                     | all        |
| `jitter`    | `jitter_ms`                                | edge only  |
| `bandwidth` | `bandwidth_cap`                            | edge only  |

- **Partial updates**: a `PUT` without `ttl` makes the units it touches permanent again; untouched units keep their existing leases. Re-sending a unit with a new `ttl` replaces its deadline. Invalid `ttl` values return `400` without changing state.
- **Expiry**: when a lease expires, its fields are **cleared** (not restored; `mode` returns to `merge`), empty entries are removed, and an `expire` event is emitted on `/__kaanal/events`.
- **Visibility & saving**: `expires_in_ms` reports remaining time; `scenarios/save` from `live` writes leased fields as permanent and emits a warning (`"N fields have a ttl; the file has none"`; `mode` is not counted). Leases are in-memory only.

### `mode: override`

By default, live entries stack on top of active scenarios (latencies sum, `down` ORs, error rates combine). With `"mode": "override"`, the live entry replaces lower layers for that key (e.g. `{"mode":"override","down":false}` calms a scenario outage, and so does a bare `{"mode":"override"}`). A node override also calms that node's `node_at` scenario entries; an endpoint or `node_at` override only affects its own key, so a `node_at` override cannot calm chaos set on the node itself (use a node override). The mode sticks: a later `PUT` without `mode` keeps it, and `"mode": "merge"` goes back to stacking.

## `PUT /__kaanal/chaos`

Like the per-node route, but targets `node`, `endpoint` (`"GET /orders"`), or both (`node_at`). At least one target is required (`400` otherwise; unknown node/endpoint is `404`; invalid endpoint format is `400`).

- **Layering**: higher layers win for the same field; within a layer, `node_at` goes before `node`. `endpoint` entries roll once at the front door before the chain (see [chaos internals](https://github.com/Naveen2070/kaanal/blob/main/docs/source/chaos.md#endpoint-chaos-and-keys)).

### Replicas (`replica`)

Target one copy of a replicated node with `"replica": N` (0-based, with `node` only; no `endpoint`). Out-of-range indices return `400`. Downed replicas are skipped by the balancer (`503` if all are down). Listed under `replicas` in `GET /__kaanal/chaos`; clear with `DELETE /__kaanal/chaos?node=db&replica=1`.

### Scheduled outage start (`from_ms`)

`down: true` on a node or replica accepts `from_ms` (ms from run/system start) so the simulator and live mocks bring the node down at that instant and balancers count `detect` from it. Live streams report the current clock as `system_ms` (`from_ms = system_ms + N * 1000`). A `PUT` setting `down` without `from_ms` clears the start time; `from_ms` without `down: true` or on an endpoint/edge returns `400`.

### Link chaos (`edge`)

Target a wire with `{ "edge": { "from": "app", "to": "db" }, ... }` (cannot combine with `node`, `endpoint`, or `replica`; unknown edge is `404`):

| Field                         | Effect on the link                                                                                                                                                                                                                                                                                                                                                                                                                       |
| ----------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `latency_ms` and `jitter_ms`  | Adjusts the network hop (`jitter_ms` is uniform `±` clamped at `0`, leased together with `latency`).                                                                                                                                                                                                                                                                                                                                     |
| `error_rate` and `error_kind` | `reset` (default, `502`) or `hang` (never `status`/`malformed` or `error_status`).                                                                                                                                                                                                                                                                                                                                                       |
| `down: true`                  | Link partition (hangs until callee `timeout`, else `60s`, then `504`).                                                                                                                                                                                                                                                                                                                                                                   |
| `bandwidth_cap`               | Caps link speed (bytes/s integer or `"500KB/s"` string; `1 KB = 1024 B`; must be `<=` edge's configured `bandwidth`; units `B`, `KB`, `MB`, `GB`, `/s` optional, fractions round to the nearest byte; a string that is not a size or rate, such as `"5XB"`, `"-5KB"`, or `"KB"`, returns `422` naming the field and units; a cap of `0` after conversion returns `400`). `kaanal chaos set --bandwidth-cap 500KB/s` takes the same text. |

Dropped lookups to a `cache` count as misses, never failures. Edge entries are listed under `edges` in `GET /__kaanal/chaos`; clear with `DELETE /__kaanal/chaos?edge_from=app&edge_to=db`.

```json
{ "endpoint": "GET /orders", "error_rate": 1.0, "error_kind": "reset" }
```

## `DELETE /__kaanal/chaos/{node}` / `DELETE /__kaanal/chaos`

Clear one node (`404` if unknown) or all **live** chaos. Both return `204`.

`DELETE /__kaanal/chaos?node=&endpoint=` clears exactly the entry under that key (`node`, `endpoint` or both) and leaves the others:

| Case                    | Status                    |
| ----------------------- | ------------------------- |
| Unknown query parameter | `400` (clears nothing)    |
| Malformed `endpoint`    | `400`                     |
| Unknown endpoint        | `204` (nothing to remove) |
| Empty or unknown `node` | `404`                     |

**Scenarios are not touched**; use `DELETE /__kaanal/scenarios` or `POST /__kaanal/reset`.
