---
title: "Live and events"
sidebar:
  order: 8
---

## The live simulation: `GET /__kaanal/live` and its commands

A continuous simulation on the same engine the mocks run on (Studio's default view). Traffic is generated from the first `traffic:` profile of the running system (Poisson arrivals at its `rps`, split by journey weight), so a real request to a mock and a generated one meet the same queues, limits, `cpu`, retries and breakers.

`GET /__kaanal/live[?fresh=1]`: a server-sent events stream. The token may be given as `?token=` here (as on `/events`; the browser `EventSource` cannot set headers); the commands below need the `Authorization` header. The first client starts a **session**; `fresh=1` (a page load) resets it, an automatic reconnect (no `fresh`) keeps it; two clients share one session; the session stops **5 seconds** after the last client leaves, so a closed page leaves nothing running (the mocks and the engine keep serving). Events:

- `hello`: the session (`session`, `profile`, `note` when nothing can be generated (no profile, a route that does not exist), `rate`, `paused`, `overloaded`, `in_flight`, `submitted`, `clients`).
- `live`, every 250 ms: `{ t_ms, requests, failed, nodes: [{ arrived, dropped: { overflow, rate_limit, deadline, breaker, chaos }, queue, busy_slots, cpu_load }], wires: [{ carried, wait_ns, wire_ns, retries, retried }], hist: [[bucket, count]], journeys: [...], status: { rate, paused, overloaded, in_flight } }`. Counts are integers and sums, so a window of ticks is added up on the page; `hist` is a log-spaced latency histogram (1% precision, 10 microseconds to 1,000 seconds, occupied buckets only) of the requests that were served in the tick. `journeys` are at most 10 finished requests a tick (about 40 a second), each with its hops `{ node, replica, kind, start_ns, end_ns, wait_ns, attempt, edge }` and `dropped_at` for a dropped one; a generated journey also has `stream` (an index into the `streams` of the `hello`, the routes of the offered mix) and a dropped one has `cause` (`overflow`, `rate_limit`, `deadline`, `breaker`, `oom` or `chaos`). A wire's `wait_ns` is the time requests spent waiting to get onto it and `wire_ns` the time on it, both summed over the crossings in the tick, so `wire_ns / carried` is the mean crossing. `retries` counts the crossings that were a retry (attempt 2 or later, counted on the wire the first try came in through) and `retried` the calls that made at least one, so the calls are `carried - retries`. A node also carries `hist` (the work times of its own hops, waiting excluded, as occupied `[bucket, count]` pairs of the same buckets as the tick's `hist`) and, when it ran on more than one copy, `replicas` (own hops per copy); both are left out when empty. `queue` and `busy_slots` are read at the instant of the tick; `cpu_load` is running hops over cores x copies for a node with `cpu`.
- `marker`: `{ at_ms, kind, text }` for a `param` edit, a `chaos` change, `pause` or `resume`.
- `resync`: sent once after the client fell behind and missed events (the status as `hello`); the page keeps its timeline with a gap.

The stream ends when the session ends (an Apply restarted it, the server shuts down); the browser reconnects and gets a new `hello`.

`POST /__kaanal/live/pause`, `POST /__kaanal/live/resume` (the generator stops or restarts; the engine keeps serving and draining), `PUT /__kaanal/live/rate` with `{ "rps": N }` (at most 20,000 a second; above it, a negative or a non-number is a 400 and the rate stays). All answer with the session status, `409` when no page is connected. A generator stops adding requests when 200,000 are unfinished (`overloaded: true`), so an overloaded node with no queue limit cannot grow without bound.

**Parameter edits go straight in.** While a page follows the live run, a change of the draft (`PUT /__kaanal/draft`) that only changes parameters is applied to the running engine at once, without Apply and without a drain: queues, running work and breaker state carry over (`cpu`, slot limits, rate limits, node cards and wire bandwidth, copy counts, service times, chaos-independent settings). The reply gains `live`: `{ "live": "applied", "changed": ["app"], "structure_pending": false }`, `"unchanged"` (nothing running changed; it may still hold structure edits), `"none"` (no page is connected: the draft waits for Apply as before) or `"invalid"` with `message` (the draft does not parse, a value was refused: the running system is as it was). Structure edits (a component or wire added or removed, a rename, a changed kind) stay in the draft until `POST /__kaanal/draft/apply`, which restarts the live run; a patch that mixes both applies its parameter part and leaves the structure part. The file on disk changes only on `POST /__kaanal/draft/save`.

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
- `shutdown` is the last message of a stream: it is sent once, with `{ "timeout_ms": N }` (the time left in the drain), when a [shutdown](/docs/reference/control-api/shutdown-and-ui/#post-__kaanalshutdown) starts, and the stream then ends. A client that sees the stream end without it should treat that as a shutdown too (with a tiny budget the connection can be cut before the message is written). `kaanal logs -f` does this: a `shutdown` event means the server is gone (exit 2 once the process has exited), and so does a stream that ends without one and whose instance lock is free after a short wait. The web UI closes the stream and shows "Server stopped" instead of reconnecting. The events of requests that are cut at the end of the drain are not delivered.
