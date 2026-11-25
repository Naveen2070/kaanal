---
title: "Live and events"
sidebar:
  order: 8
---

## The live simulation: `GET /__kaanal/live` and its commands

A continuous simulation on the same engine the mocks run on (Studio's default view). Traffic is generated from the first `traffic:` profile of the running system (Poisson arrivals at its `rps`, split by journey weight), so a real request to a mock and a generated one meet the same queues, limits, `cpu`, retries and breakers.

`GET /__kaanal/live[?fresh=1]` opens a Server-Sent Events stream (`?token=` is accepted for `EventSource`). The first client starts a **session** (`fresh=1` resets it, reconnects without `fresh` keep it, multiple clients share it, and it stops **5 seconds** after the last client leaves while mocks keep serving).

### Live stream events

- **`hello`**: `{ session, profile, note, rate, paused, overloaded, in_flight, submitted, clients }` (`note` explains if nothing can be generated).
- **`live`** (every `250 ms`):
  - **Counts**: `{ t_ms, requests, failed, status: { rate, paused, overloaded, in_flight } }` (integer sums per tick).
  - **`nodes[]`**: `{ arrived, dropped: { overflow, rate_limit, deadline, breaker, chaos }, queue, busy_slots, cpu_load }`, plus `hist` (own hop times, omitted when empty) and `replicas` (own hops per copy when `replicas > 1`). `queue` and `busy_slots` are sampled at tick end; `cpu_load` is running hops over `cores × copies`.
  - **`wires[]`**: `{ carried, wait_ns, wire_ns, retries, retried }` (`wire_ns / carried` is mean crossing; `carried - retries` is first-try calls).
  - **`hist`**: log-spaced 1%-precision latency histogram (`10 µs`–`1000 s`, `[bucket, count]` for occupied buckets).
  - **`journeys[]`**: up to `10` finished requests/tick (~40/s), with `{ node, replica, kind, start_ns, end_ns, wait_ns, attempt, edge }` hops, `stream` index, `dropped_at`, and `cause` (`overflow`, `rate_limit`, `deadline`, `breaker`, `oom`, `chaos`).
- **`marker`**: `{ at_ms, kind, text }` for `param`, `chaos`, `pause`, or `resume`.
- **`resync`**: sent once if the client fell behind (same payload as `hello`; page keeps its timeline with a gap).

The stream ends on Apply or shutdown; browsers reconnect for a fresh `hello`.

### Live controls & parameter edits

- **`POST /__kaanal/live/pause`** / **`POST /__kaanal/live/resume`**: stops or restarts the traffic generator while the engine keeps serving (`409` if no client is connected). Generator pauses new arrivals at `200,000` in-flight requests (`overloaded: true`).
- **`PUT /__kaanal/live/rate`**: `{ "rps": N }` (`0`–`20,000`; `400` outside range).
- **Live parameter edits (`PUT /__kaanal/draft`)**: while connected, parameter-only edits (`cpu`, slots, rate limits, NIC/wire `bandwidth`, `replicas`, service times, chaos-independent settings) apply immediately without draining (`live: { "live": "applied", "changed": ["app"], "structure_pending": false }`, `"unchanged"`, `"none"`, or `"invalid"` with `message`). Structural edits (add/remove/rename node or wire, kind change) wait for `POST /__kaanal/draft/apply`. Disk files change only on `POST /__kaanal/draft/save`.

## `GET /__kaanal/events`

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

- `id` is the request's [id](/docs/guides/observe/observability/#the-request-id) (also the response's `X-Request-Id`), `seq` its number, and `client_id` the sanitised `X-Request-Id` the client sent (or `null`). `endpoint` is the matched route template, `null` when no route matched. The event carries no hops: ask `GET /trace/{id}` for those.
- `rejected` is `null` for a request the call chain answered, otherwise `unmatched` (a `404` or `405`), `bad-scope`, `engine-stopped` or `client-gone` (the client left first; status `499`). Every request that gets an id produces exactly one `request` event; preflights and control calls produce none.
- `ms` is measured wall time, from the moment the request got its id; the per-hop _simulated_ breakdown is in the response's `X-Kaanal-Trace` header (parallel branches in `[a|b]`, cache lookups as `hit`/`miss`, long chains ending `,...+N hops`) and in the [log entry](/docs/guides/observe/observability/).
- `kind` is `reset`, `malformed` or `timeout` when the request failed that way (a reset or cut-off body still answers `200`, so the status alone does not show it); `null` otherwise.
- `request` events carry `pin`: `null`, or the state a pin forced on the request (`"loading"`, `"empty"`, `"error"`, `"slow"`, `"success"`). A pinned request has `failed_node: null` even when the chain failed.
- `request` events carry `scope`: `null` for a request without `X-Kaanal-Scenario`, otherwise the stack it was answered with (`""` for an empty header).
- `scenario` events carry the `action` — `activate`, `deactivate`, `set` (`PUT /scenarios/active`), `clear` (`DELETE /scenarios`), `reset`, `update` (the definition of an active scenario changed) or `expire` (a `ttl` lease ran out and the scenario was deactivated), `pin` (`PUT` or `DELETE /pins`, `name` is the endpoint or `null` for a clear-all) or `cycle` (`POST /cycle`, `name` is the endpoint) — the `name` it concerns (or `null`) and the `active` stack afterwards.
- A live chaos lease that runs out (a `PUT /chaos` with `ttl`) is an `expire` too, with `name: null` and an extra `chaos_key`, the key whose units were cleared: `{"action":"expire","name":null,"active":[],"chaos_key":{"node":"db"}}`. `chaos_key` is an object with only the parts the key has, named as in the `PUT /chaos` body and the `DELETE /chaos?node=&endpoint=` query: `{"node":"db"}`, `{"endpoint":"GET /x"}` or `{"node":"db","endpoint":"GET /x"}`. Only these events carry `chaos_key`.
- `lagged` means the stream fell behind and skipped that many events.
- `shutdown` is the final message (`{ "timeout_ms": N }` remaining in drain) when [shutdown](/docs/reference/control-api/shutdown-and-ui/#post-__kaanalshutdown) starts, after which the stream closes.
  - Clients should also treat a stream ending without `shutdown` as a shutdown (tiny budgets can cut before it writes).
  - `kaanal logs -f` exits `2` on `shutdown` (or if the instance lock clears); the web UI closes and shows "Server stopped". Cut requests at drain end are not delivered.
