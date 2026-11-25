---
title: "Pinned states and kaanal cycle"
sidebar:
  order: 3
---

Your frontend has more than a happy path: a spinner while data loads, an "nothing here yet" page, an error banner, a slow screen. Real backends rarely show you these on demand. A **pin** forces one endpoint to answer as a chosen UI state, so you can look at each screen whenever you like, by hand or from a test.

## The five states

| State     | What the client gets                                                                                   |
| --------- | ------------------------------------------------------------------------------------------------------ |
| `loading` | The normal response, after an extra **10 s**.                                                          |
| `slow`    | The normal response, after an extra **2 s**.                                                           |
| `error`   | Status **500** with the body `{"error":"pinned","state":"error"}`.                                     |
| `empty`   | A well-formed response with no data (`[]`, `total: 0`, ...), derived from the schema. Status as usual. |
| `success` | The normal response. It does nothing by itself, but it **masks** a pin lower down (see "Layers").      |

A pin entry is `{ endpoint, state, status?, delay? }`:

- `status` sets the response status: 400 to 599 on `error` (4xx is fine), 200 to 299 on `empty`. `loading`, `slow` and `success` take none.
- `delay` replaces the default delay (above 0, at most 5 min). `success` takes none.
- A pinned `429` or `503` also carries `Retry-After: 1`.

You can change what a state answers for one endpoint with [`states:`](/docs/guides/mock/endpoints/#states-what-a-pin-answers); a value in the pin itself beats `states:`, which beats the defaults above.

## The chain still runs, and a pin wins

A pin changes **what the client receives**, not what happens inside. The whole call chain still runs: hops cost their time, queues and pools fill, rate limits take tokens, and the trace shows all of it. Then the pin decides the answer, and it wins over everything: a node you set `down`, a chaos error, `error_map`, a broken connection (`reset`, `malformed`), a `429` from a rate limit, a full queue. So `success` pinned on an endpoint whose database is down answers `200`, and the trace still tells you the database failed.

The trace ends with `pinned=STATE`, and a pin's delay is the first piece, charged to the entry node (the gateway, or the service that owns a route with no gateway):

```
x-kaanal-trace: nginx.delay=10000.0ms,nginx.proc=1.0ms,net=1.0ms,app.proc=2.0ms,net=1.0ms,db.read=4.0ms,pinned=loading
```

With the database down and `success` pinned:

```
x-kaanal-trace: nginx.proc=1.0ms,net=1.0ms,app.proc=2.0ms,net=1.0ms,db.error=503,pinned=success
```

Two more consequences:

- **The delay sits outside every timeout.** A `loading` pin of 10 s under a gateway `timeout: 5s` still answers after about 10 s, with no timeout in the trace. The pin delay holds no slot or token, and adds no random draw, so a seeded run draws the same numbers with and without a pin. It is a hop of its own: chaos `latency_ms` on the endpoint is a separate `delay` piece after it, and each delay is capped at 5 min on its own (they are not summed or clamped together).
- **The request events** on the [event stream](/docs/reference/control-api/live-and-events/#get-__kaanalevents) carry `"pin": "error"` (or `null`), and `failed_node` stays `null` for a pinned request even though the trace shows the chain failure. In the [request log](/docs/guides/observe/observability/) a pinned request has the flag `PN` (a pin delay adds `PD` and `DI`), and its entry keeps the chain's own failure in `chain_error`; `kaanal trace` ends such a request with `pinned=success hid: db.error=503`.

## `kaanal cycle`: step through the states

```bash
kaanal cycle "GET /orders"          # GET /orders -> loading
kaanal cycle "GET /orders"          # GET /orders -> empty
kaanal cycle "GET /orders"          # GET /orders -> error
kaanal cycle "GET /orders"          # GET /orders -> slow
kaanal cycle "GET /orders"          # GET /orders -> success
kaanal cycle "GET /orders"          # GET /orders -> loading (it wraps)
```

Each call moves the endpoint's **live pin** to the next state: `loading`, `empty`, `error`, `slow`, `success`, then back. With no pin yet, the first step is `loading`. The next state comes from the live pin only: a pin from a scenario does not count, so the first `cycle` after a scenario starts at `loading` and then masks the scenario's pin. A state that cannot work on that endpoint is skipped (`empty` where there is no empty form), so there the cycle has four states.

```bash
kaanal cycle "GET /orders" --state error --status 429 --delay 500ms   # GET /orders -> error (status 429, delay 500ms)
kaanal cycle "GET /orders" --off                                      # GET /orders -> unpinned
kaanal pins                                                           # what is pinned, and by whom
```

| Flag         | Meaning                                                                                             |
| ------------ | --------------------------------------------------------------------------------------------------- |
| `--state S`  | Pin that state directly instead of stepping.                                                        |
| `--status N` | Status for the resulting state (400-599 on `error`, 200-299 on `empty`).                            |
| `--delay D`  | Delay for the resulting state, e.g. `2s` (above 0, at most 5 min).                                  |
| `--off`      | Remove this endpoint's live pin (also fine when there is none). Cannot be combined with the others. |

A bad combination (`--status` landing on `loading`) is refused with the server's message and changes nothing. A wrong `--state` name or an unreadable `--delay` is refused before anything is sent.

`kaanal pins` lists the live pins, then the pin in force per endpoint with the layer it comes from:

```
live pins:
  GET /users               slow
effective pins:
  GET /users               slow     live
  GET /users/{id}          error    scenario:pinned (status 404)
```

`kaanal status` adds one `pin KEY: state (live)` line per pin in force, and `kaanal scenario show NAME` lists a scenario's pins.

## Pins in scenarios

A scenario can carry a `pins:` list next to (or instead of) `chaos:`. Activating it applies the pins; deactivating it, a `--ttl` expiry, or deleting it takes them away with it.

```yaml
scenarios:
  orders-empty:
    description: "New customer, no orders yet"
    pins:
      - { endpoint: "GET /orders", state: empty }
  checkout-trouble:
    chaos:
      - { node: payments, latency_ms: 800 }
    pins:
      - { endpoint: "POST /checkout", state: error, status: 402, delay: 1s }
```

- A scenario needs at least one entry in `chaos:` or `pins:`. `chaos:` is optional; a pins-only scenario is fine.
- The same endpoint twice in one scenario is an error. A `HEAD` made from its `GET` is refused (pin the `GET`). The endpoint must exist and enter a chain.
- `empty` on an endpoint with no empty form is refused when the file loads, naming the endpoint (see "How `empty` is built").
- Pins count toward the 256 entries a scenario may hold.
- `kaanal validate` and `kaanal scenario load` check pins like the rest of the file.

## Layers: who wins

Pins stack like chaos, but per endpoint the **highest layer wins whole**: its state, status and delay, with nothing merged from below. From bottom to top: the active scenarios in activation order, then the live pins (`kaanal cycle`, `PUT /__kaanal/pins`).

`success` is a real value: pin it from a higher layer and it hides a lower pin, like switching a feature flag off. `kaanal cycle EP --off` removes only the live pin, so a scenario's pin underneath shows again.

`kaanal reset` clears live pins with the rest of the live layer. `kaanal chaos clear` leaves live pins alone, and `DELETE /__kaanal/pins` leaves live chaos alone.

To keep a combination, `kaanal scenario save NAME` writes live pins into the file's `pins:` list (with `--from effective`, the winning pin per endpoint; `--clear-live` clears live pins too).

## Parallel test workers

Live pins are global, so `kaanal cycle` would change the page for every worker. Workers should pin through scenarios and the `X-Kaanal-Scenario` header (see [Parallel test workers](/docs/guides/break/scenarios/#parallel-test-workers)):

```ts
const context = await browser.newContext({
  extraHTTPHeaders: { 'X-Kaanal-Scenario': 'orders-empty' },
});
```

A request that carries the header sees **only the pins of the scenarios it names**. Live pins never reach it, and global scenarios do not either. So `kaanal cycle` in your terminal does not disturb tests running in parallel, and the other way round.

## How `empty` is built

`empty` answers with the "no data" shape of the endpoint's normal response, built from its OpenAPI response schema (or from the body of an `override:`), with the route's normal success status. Rules, applied to the top level and to nested objects:

| Where                                                                                                                          | Empty form                                                    |
| ------------------------------------------------------------------------------------------------------------------------------ | ------------------------------------------------------------- |
| A response that is an array                                                                                                    | `[]`                                                          |
| A property that is an array                                                                                                    | `[]`                                                          |
| A number named `total`, `count`, `totalCount`, `total_count`, `totalItems`, `total_items`, `totalElements`, `numFound`, `size` | `0`                                                           |
| A boolean named `hasNext`, `has_next`, `hasMore`, `has_more`                                                                   | `false`                                                       |
| A property named `next`, `nextCursor`, `next_cursor`, `nextPage`, `next_page`                                                  | `null`                                                        |
| A nested object                                                                                                                | the same rules, one level down                                |
| Any other property (`page`, `limit`, `name`, ...)                                                                              | generated as usual (kept as written, for an `override:` body) |
| A bare scalar, or a top-level `oneOf`/`anyOf`/`allOf`                                                                          | none: the pin is refused                                      |

Names match exactly, ignoring case. For an `override:` body the same rules run on the value: arrays are emptied, those names are reset, other values stay.

**This is a heuristic with a ceiling.** It knows common list and pagination names, nothing more. A response that is a single object (`GET /users/{id}`) has no list to empty, so `empty` returns the usual generated object. A list called `results` with a count called `hits` gets `results: []` but keeps a generated `hits`. For those, write the body you want:

```yaml
endpoints:
  "GET /users/{id}":
    states:
      empty: { body: { id: null, firstName: "", lastName: "" } }
  "GET /search":
    states:
      empty: { status: 200, body: { results: [], hits: 0 } }
```

A `states.empty.body` is returned **verbatim** (no `{id}` path substitution) and also satisfies the load-time check.

Special cases:

- **204 and 304 routes.** A route that normally answers 204 or 304 has no body, so `empty` on it keeps the status and sends no body, with no `states:` needed. A 204 or 304 status never carries a body, whether the route normally answers it or a pin sets it (`--status 204`, `states.empty.status: 204`).
- **Built-in routes.** The login and JWKS routes of an `auth` node have no empty form by default: an `empty` pin on them is refused, and `kaanal cycle` skips `empty` there (four states). `loading`, `slow`, `error` and `success` work. Give the route a `states.empty.body`, or an `override:` (which replaces the built-in answer, so the route is an override route), and it can be pinned `empty`.
- **`override:` routes.** An endpoint with an `override:` body (static or on a spec route) derives `empty` from that body, not from the schema.

## HEAD, 4xx and the rest

- **HEAD.** A `HEAD` request follows its `GET`'s pin: same status, same `Retry-After`, same delay, `pinned=` in the trace, and no body. A `HEAD` that is only the automatic copy of a `GET` cannot be pinned on its own (pin the `GET`).
- **Errors.** `error` takes any status from 400 to 599, so a pinned `404`, `409` or `422` is fine. The body is `states.error.body` if you set one, else `{"error":"pinned","state":"error"}`. `429` and `503` add `Retry-After: 1`. `error_map` never applies to a pin.
- **Static routes** need a gateway (or an owner node) to be pinnable, like chaos on them; one without an entry node is refused.

## Limits worth knowing

- One pin per endpoint per layer; custom state names (such as `maintenance`) do not exist yet.
- A pin never changes what the chain did, only the answer. If you want the backend to fail for real, use [chaos](/docs/guides/break/scenarios/).
- `states:` on `loading` and `slow` can set `delay` (their `status` and their `body` are refused: they answer as the route normally does).
- The simulator ignores pins: it models the backend, not the page.
- Internals: [source/files/kaanal-server/pins.md](https://github.com/Naveen2070/kaanal/blob/main/docs/source/files/kaanal-server/pins.md).
