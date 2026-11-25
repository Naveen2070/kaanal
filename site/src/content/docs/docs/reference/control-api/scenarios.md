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

**Activate with a lease.** `POST .../activate` accepts an optional `{ "ttl": "30s" }` (`100ms`–`24h`), deactivating and emitting `expire` on expiry. An empty body, `{}`, or `{"ttl":null}` clears any lease; re-activating with `ttl` replaces it. Invalid values return `400` with no changes, and any deactivation/`reset`/delete drops the lease.

**Scoping a mock request.** Send `X-Kaanal-Scenario: a,b` (up to 16 comma-separated names, `<= 256` bytes) to evaluate only that stack, ignoring global scenarios and live chaos. Responses echo `X-Kaanal-Scope` (or `400 bad scenario scope` on invalid headers); `404`/`405` and `/__kaanal` ignore it. See [scenarios](/docs/guides/break/scenarios/#parallel-test-workers).

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

**Save.** `POST /__kaanal/scenarios/save` writes `scenarios/NAME.yaml` next to `kaanal.yaml` (created if missing) with only set fields, and defines the scenario immediately:

```json
{ "name": "flaky-db", "from": "live", "force": false, "clear_live": true }
```

| Field        | Rules                                                                                                     |
| ------------ | --------------------------------------------------------------------------------------------------------- |
| `name`       | Required.                                                                                                 |
| `from`       | `"live"` (default), `"effective"`, or `{ "scenario": "other" }` (`422` otherwise).                        |
| `clear_live` | `true` clears live chaos after writing so activating the saved scenario does not double-count it.         |
| `warnings`   | In the response: reports leased fields written as permanent (`"N fields have a ttl; the file has none"`). |

| Status  | When                                                                                                                                                                           |
| ------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| **400** | Bad name, inline name even with `force`, nothing to save, `mode: override` with `from: "live"`, or `from: "effective"` with stacked failure kinds/mixed node+node-at-endpoint. |
| **404** | Missing `from` scenario.                                                                                                                                                       |
| **409** | Exists without `force: true`.                                                                                                                                                  |
| **500** | Write error.                                                                                                                                                                   |

**Reset.** `POST /__kaanal/reset` deactivates all scenarios, undefines runtime scenarios (inline/file stay), clears live chaos, zeroes route counters, refills rate-limit buckets, and empties [collections](/docs/guides/mock/collections/), [response edits](/docs/reference/control-api/payload-edits/), and the [request log](/docs/guides/observe/observability/) (request IDs continue). Queues and pools finish in-flight requests normally.
