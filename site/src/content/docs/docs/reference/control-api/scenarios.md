---
title: "Scenarios"
sidebar:
  order: 2
---

A [scenario](/docs/guides/break/scenarios/) is a named list of chaos entries. The scenarios you activate form a stack, in activation order, with the live chaos above them. Names follow `^[a-z0-9][a-z0-9_-]{0,63}$`; `active`, `save`, `reset` and Windows device names are reserved. All of these need the token. Every write is atomic: a bad request changes nothing, and once a `2xx` response is back, every later request sees the new state.

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

**Scoping a mock request.** A request to a mock route may carry `X-Kaanal-Scenario: a,b` (comma separated names, at most 16, at most 256 bytes) and is then answered with exactly that stack, ignoring the global stack and the live layer. The response echoes it as `X-Kaanal-Scope: a,b` (an empty request header echoes an empty value). A bad header answers `400 { "error": "bad scenario scope", "detail": "..." }`. `404` and `405` answers, and the `/__kaanal` namespace, ignore the header. Details and the shared-counter caveat: [scenarios](/docs/guides/break/scenarios/#parallel-test-workers).

**List.** `GET /__kaanal/scenarios` returns `[{ "name", "source", "description", "entries", "pins", "active" }]` (`entries` counts chaos entries and pins; `pins` lists them as `{ endpoint, state, status?, delay? }`, `delay` as text like `"2s"`; `GET /scenarios/{name}` carries `chaos` and `pins`) in definition order; `source` is `inline` (`kaanal.yaml`), `file` (`scenarios/NAME.yaml`) or `runtime`.

**Define.** `PUT /__kaanal/scenarios/{name}` takes the same body as a scenario file (`chaos` and `pins` are both optional, one of them is required; pins are `{ endpoint, state, status?, delay? }`, see [pins](/docs/guides/break/pins/)):

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

**Reset.** `POST /__kaanal/reset` deactivates every scenario, **undefines the runtime-defined ones** (inline and file scenarios stay), clears the live chaos, zeroes the per-route request counters (so a seeded run repeats), refills rate-limit buckets, empties every [collection](/docs/guides/mock/collections/), clears every [response edit](/docs/reference/control-api/payload-edits/) and empties the [request log](/docs/guides/observe/observability/) (request ids keep counting). Queues and pools are not reset; requests already in flight finish with the chaos they started with. Call it between tests.
