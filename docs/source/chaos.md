# Failures: kinds, timeouts and keyed chaos

How Kaanal makes things fail (B2a, rules R1.17, R1.18, R1.38). For the user-facing view see [../web/concepts.md](../web/concepts.md#chaos) and [../web/control-api.md](../web/control-api.md); the code is in [files/kaanal-core/chaos.md](files/kaanal-core/chaos.md), [files/kaanal-core/exec.md](files/kaanal-core/exec.md), [files/kaanal-core/replay.md](files/kaanal-core/replay.md) and [files/kaanal-server/mock.md](files/kaanal-server/mock.md).

## Failure kinds

A chaos entry has an input `error_kind` (`status` by default, `reset`, `hang`, `malformed`). The walk turns it into an outcome kind (`FailKind`: `Status`, `Reset`, `Malformed`, `Timeout`). What the client sees depends on **where** the failure happens.

| Input                             | Outcome kind | At a wire node (gateway or owner)                                                        | At a deep node (database, cache, external API, a service the owner calls) |
| --------------------------------- | ------------ | ---------------------------------------------------------------------------------------- | ------------------------------------------------------------------------- |
| `status` (`error_status` 500-504) | `Status`     | That status.                                                                             | That status, through `error_map`.                                         |
| `down`                            | `Status`     | 503.                                                                                     | 503, through `error_map`.                                                 |
| `reset`                           | `Reset`      | Status line and part of the body, then the connection aborts.                            | 502 (or the `error_map` result).                                          |
| `malformed`                       | `Malformed`  | The route's success status and a JSON body cut at about half, so it does not parse.      | 502 (or the `error_map` result).                                          |
| `hang`                            | `Timeout`    | 504 after the hop's length (the node's `timeout:`, else 60 s); `error_map` can remap it. | 504 after the same length, through `error_map`.                           |
| `timeout:` fired                  | `Timeout`    | 504 at the deadline; `error_map` can remap it.                                           | 504 at the deadline, through `error_map`.                                 |

- `error_status` is range-checked (500-504) for every kind, not only `status` (a stored 404 under `reset` would turn into a surprise if the kind changed later).
- A cache is never a failing node: see "Caches fail open".
- The default error body is `{"error":"chaos","node":NAME}`; non-status kinds add `"kind":"reset|malformed|timeout"`. The trace ends `node.error=STATUS` with the failure's own status before `error_map` (the status itself for kind `status`, 504 for a timeout), and `node.error=reset` / `malformed` for those kinds. The `X-Kaanal-Trace` still reports the real failure when `error_map` changes the response.
- `reset` is "partial response, then abort", not a literal TCP RST and not a refused connection (R1.18). The body stream is one chunk, one `Pending` (waker woken), then an error; that is the only shape hyper 1.x flushes the head for (plan task 0 spike, `kaanal-server/src/wire.rs`). `Content-Length` is the full length, so a client sees the abort as a short body.
- HEAD, 204 and 304 have no body to cut: `reset` and `malformed` answer the plain success response there (the trace still says `error=reset`).

## Wire nodes and deep nodes

The **wire nodes** of an endpoint are its gateway (if any) and its owner (`Endpoint::is_wire_node`). A gateway is a transparent proxy for the owner's response, so a failure at a wire node reaches the client as it is; a deep failure ends the chain and is turned into a response by `error_map` (Z7).

`error_map` lookup order is **node, kind, status, class** (Z8). A kind key is `reset`, `malformed` or `timeout`; a node with the same name wins. For a deep failure the status and class keys apply to the _default_ status (reset and malformed 502, timeout 504). At a wire node a `reset` or `malformed` is only intercepted by a node or kind key: a `5xx` key never turns a reset into JSON. A `timeout` at a wire node is **not** a wire failure: it stays remappable (like nginx `error_page 504`), and the default is a plain 504.

## `timeout:`

`timeout:` is a field on every node kind (`0 < timeout <= 5 min`, a positioned load error otherwise). It means "the deadline for calls to this node" and covers the **whole span**: queue wait, the node's own work and everything it calls downstream. A gateway with `timeout: 1s` times out when the database behind it is slow (Z2).

- A **span** (`PlanItem::Span { node, timeout, at, size }`) is a plan item opened before the edge hop of a call to a node that has a `timeout:`, and for the entry node at the root. Only nodes with a `timeout:` produce spans. The walk never predicts timeouts; the replay engine fires a deadline event at span start + `timeout` (Z3). `execute` computes the same thing by arithmetic, as the independent zero-load oracle (Z13).
- The failing node of a timeout is the span's node (Z9): the gateway timing out on a slow database fails as `gateway`.
- **Zombie hops (Z4).** A deadline fails the request through the existing `fail()`: queued tokens of the request leave their queues and the client is answered at the deadline, but hops already running end naturally and **hold their slot** until they do. Waiters leave; the work does not. This is what a real database does until its own `statement_timeout`, and it is what turns a slow dependency into a timeout cascade. Example: `pool: { size: 1, queue: 5 }`, `timeout: 500ms`, 400 ms ops, three simultaneous requests: #1 finishes at 400 ms; #2 gets the slot at 400 ms and times out running at 500 ms (its hop is a zombie until 800 ms); #3 times out in the queue at 500 ms; a fourth request arriving at 600 ms waits for the zombie until 800 ms, runs to 1200 ms and times out at its deadline of 1100 ms.
- **Instant order (Z14).** At one instant hop-dones and arrivals run first, deadlines in a later phase: a callee that ends exactly at its deadline succeeds, and a planned `Fail` reached at that instant is decided before any deadline. Two deadlines of one request at one instant: the outer span (lower index) fires.
- A span closes when the token that opened it has resumed at or past its end. A span that ends in a `parallel:` block stays open until the join. Closed spans and spans of a finished request (generation check) ignore their deadline when it pops; stale deadlines stay in the heap until their time, at most `timeout`.
- `NodeCounters.timed_out` / `NodeStats::timed_out` count deadlines at the span's node.

## Hang

`hang` is one own hop of the node's `timeout` (else the 60 s `HANG_CAP`) followed by a planned `Fail { kind: Timeout, status: 504 }` at the hop's end (Z6). The hop **holds its gate slot** and counts toward capacity like any other hop (a hung node is busy). No event is scheduled beyond the cap, and the engine has no hang special case. An enclosing span with a smaller deadline fires first; for a node reached through a call, its own span opens one edge hop earlier, so its `timeout` pre-empts the hang `Fail`.

## Caches fail open (Z17)

A cache never fails the request. `timeout:` on a cache caps the lookup: a sampled lookup slower than the timeout is a miss charged exactly the timeout. `hang` is a miss charged the timeout or the cap. `reset`, `malformed` and `status` on a cache are a miss, as a failing cache already was. A cache step opens no span, and the timeout caps lookup service time only, not queue wait at the cache's gate.

## Endpoint chaos and keys

`ChaosKey` has three forms (Z10):

| Key                        | Applies to                                                                                                                              |
| -------------------------- | --------------------------------------------------------------------------------------------------------------------------------------- |
| `Node(n)`                  | Every visit of node `n`.                                                                                                                |
| `Endpoint("GET /orders")`  | Once, at the front door of that endpoint, before its chain. A failure belongs to the **entry node**. It does not touch nodes behind it. |
| `NodeAt(n, "GET /orders")` | Visits of node `n` while serving that client-facing route (`via:` callees do not change it).                                            |

Keys name exactly one thing. `DELETE` removes exactly the key named (an unknown endpoint is a `204`, an empty or unknown `node` a `404`), and chaos on `GET /x` does not reach the synthesized `HEAD /x` copy (D20).

- Endpoint `latency_add` is a leading `HopKind::Delay` hop on the entry node. The engine treats it like a `Net` hop: **no slot, no rate-limit token** (like Envoy's fault-filter delay). The trace shows `<entry>.delay=`. An endpoint-level `hang` is the same kind of `Delay` hop (the timeout or the cap long), so it holds no slot either, unlike a node-level hang.
- Endpoint chaos has its own RNG path, `draw_rng(root, entry node name, b"endpoint")`, so adding it re-seeds nothing else. `error_kind` is config, so no extra draw is made for it; the existing per-hop roll is made against the merged `error_rate` (Z15).
- A static endpoint needs a gateway or an owner, so an entry node always exists (the control API's 400 for "no entry node" is defensive).

## Merge rules

Stored entries are sparse `ChaosLayer`s (every field optional). `resolve(node, endpoint)` merges the node layer, then the node-at-endpoint layer (most specific last) with `merge` (Z11, R1.17):

| Field                        | Rule                                                                  |
| ---------------------------- | --------------------------------------------------------------------- |
| `latency_add`                | Sum.                                                                  |
| `down`                       | OR.                                                                   |
| `error_rate`                 | Independent union `1-(1-a)(1-b)`; one set rate comes back unchanged.  |
| `error_kind`, `error_status` | Last set wins. A layer that sets only `error_rate` never resets them. |

Defaults apply once, after the merge. The endpoint-wide layer is read once at the front door, not merged into node visits.

> **Mesh tools replace, Kaanal merges.** In Istio or Envoy the most specific fault rule replaces the broader one. In Kaanal a `node@endpoint` entry **adds** to the node's own chaos: latencies sum and rates union. To get "only this", clear the broader key. B2b will stack scenario layers with the same `merge`.

## Control API and CLI

Additive, with one breaking change (Z16):

- `error_kind` on `PUT /__kaanal/chaos/{node}`.
- `PUT /__kaanal/chaos` with `{node?, endpoint?, ...patch}`: node, endpoint or both (`NodeAt`). Unknown node or endpoint 404, the method is upper-cased, a body with neither is 400.
- `DELETE /__kaanal/chaos?node=&endpoint=` removes exactly that key; no query wipes everything; an unknown query key is 400 and wipes nothing.
- **`GET /__kaanal/chaos` is nested**: `{ "nodes": {...}, "endpoints": {...}, "node_at": [...] }` (a flat map would collide with a node named `endpoints`). The embedded UI and `kaanal status` moved with it.
- CLI: `kaanal chaos set [NODE] [--endpoint KEY] [--kind K]`, `kaanal chaos clear [NODE] [--endpoint KEY]`; `status` lists endpoint and node-at entries.

## Known limits

(From the design, section 9.)

- **Slots are held per hop, not across downstream calls (Y6).** A slow database never backs up a service's worker pool, the classic thread-per-request cascade. A capacity-model gap, left for B2b or a B1 follow-up.
- **`timeout:` is per callee**, not per caller or edge: two services calling one database cannot have different timeouts yet.
- **Failures are instant (E14).** Real 500s often come after the work; a slow error (`fail_after`) is not modelled.
- **`reset` is "partial response, then abort"**, not reset-before-headers or a refused connection; browsers show a rejected body read.
- Zombie hops make overload worse than before, by design; a cancellation-aware server (`abandon: cancel`) is a later knob.
- Out of B2a (B2b): YAML `scenarios:`, stacking, the `Arc` chaos snapshot, pins, cycle.
