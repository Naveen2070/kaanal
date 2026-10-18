# Failures: kinds, timeouts, keyed chaos and scenarios

How Kaanal makes things fail (B2a failures, B2b-1 scenarios; rules R1.17, R1.18, R1.38, R1.39). For the user-facing view see [../web/concepts.md](../web/concepts.md#chaos) and [../web/control-api.md](../web/control-api.md); the code is in [files/kaanal-core/chaos.md](files/kaanal-core/chaos.md), [files/kaanal-core/scenario.md](files/kaanal-core/scenario.md), [files/kaanal-server/control.md](files/kaanal-server/control.md), [files/kaanal-core/exec.md](files/kaanal-core/exec.md), [files/kaanal-core/replay.md](files/kaanal-core/replay.md) and [files/kaanal-server/mock.md](files/kaanal-server/mock.md).

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
- Endpoint chaos has its own RNG path, `draw_rng(root, entry node name, b"endpoint")`, so adding it re-seeds nothing else. `error_kind` is config, so no extra draw is made for it; the existing per-hop roll is made against the compiled faults (Z15: one fault is `u < rate`, several are banded, see "Banded faults").
- A static endpoint needs a gateway or an owner, so an entry node always exists (the control API's 400 for "no entry node" is defensive).

## Layers and the stack

Chaos has more than one source (B2b-1, rule R1.39). The state is an ordered **stack of layers**, bottom to top:

1. every **active scenario**, in activation order (see "Scenarios");
2. the **live layer**, everything written through `PUT/DELETE /__kaanal/chaos...`, always on top.

Each layer is a sparse `ChaosState`: keys (`Node`, `Endpoint`, `NodeAt`) mapped to `ChaosLayer`s whose fields are all optional. Re-activating an active scenario moves it to the top of the scenarios (still below live); deactivating removes it; `PUT /__kaanal/scenarios/active` sets the whole stack in one step.

## Compile on write

Hops never walk the stack. Every control write builds a new immutable `Effective { stack, chaos }` and swaps it in; `chaos` is an `EffectiveChaos`, the stack folded **per key** into `Resolved` values:

| Part             | What it holds                                                                                                                                                                     |
| ---------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `EffectiveChaos` | Three maps: nodes, node-at-endpoint pairs, endpoints. `visit(node, endpoint)` is the node-at entry if there is one, else the node entry; `endpoint(key)` is the front-door entry. |
| `Resolved`       | `latency_add` (already clamped), `down`, and up to `MAX_FAULTS` (8) banded faults in a fixed array. Reading one allocates nothing (R6.13).                                        |
| node-at entry    | Already includes the node's own contribution: the compile folds the node layers and the pair layers together, so a visit does one lookup and no merge.                            |
| `Fault`          | `upto` (the cumulative probability bound of its band), `kind`, `status`.                                                                                                          |

An empty stack compiles to an empty `EffectiveChaos`, and the executor's per-hop `is_empty` check is the same fast path as before. `execute`, `sample_plan`, the replay engine and the simulator all take `&EffectiveChaos`; a bare `ChaosState` is one `ChaosState::compile()` away, so tests and the sim CLI path stay mechanical.

## Layer order beats specificity (S6)

For one field of one key, a **higher layer wins over a lower layer whatever the keys are**. Key specificity (`NodeAt` before `Node`) only breaks a tie **inside one layer**. This is the CSS `@layer` / feature-flag model: a live toggle on a node is never silently ignored by a low-layer `node@endpoint` entry from a scenario. B2a's rule "most specific last" for kind and status is replaced by this (Z11 amended).

How a visit resolves, per `Resolved`:

| Field         | Rule                                                                                                                        |
| ------------- | --------------------------------------------------------------------------------------------------------------------------- |
| `latency_add` | Sum over every layer and key that applies, then **one clamp to 5 min** inside the compile (S9).                             |
| `down`        | OR over every layer.                                                                                                        |
| faults        | One fault per layer that sets `error_rate > 0`, ordered top layer first (same layer: node-at before node), banded as below. |

### (kind, status) is a unit (S7)

A fault has its own rate. A layer contributes a fault only when it sets `error_rate > 0`; the unset half of the pair takes the default (status 503, kind `status`). A layer that sets `error_kind` or `error_status` **without** a rate adds nothing, and the control API and the loaders report it as a **warning**, never an error and never silently (`ChaosLayer::has_orphan_fault_fields`, `ChaosState::warnings`, the `warnings` array of the responses, `kaanal validate`, a stderr line from the CLI). The pair of a higher layer takes the first band; there are no hybrids such as hang + status 500. `error_status` is still range-checked (500-504) whether or not it ends up used.

### Banded faults (S8)

With several faults, one roll `u` per visit decides. Band `i` is hit when `u` falls in `[P(i-1), P(i))`, with band probabilities `p1`, `(1-p1)*p2`, `(1-p1)(1-p2)*p3`, ... The bounds are the running independent union (`upto`), so the total is `1-Π(1-p)`: 30% `reset` in the top layer plus 30% `503` below gives about 30% resets and 21% 503s, 51% in all. `Resolved::hit(roll)` scans the bounds; `down` short-circuits to a 503.

**One draw, as in B2a (Z15): a single fault is exactly `u < rate`**, so every B2a seeded outcome is unchanged and activating a scenario adds no RNG draw (S14). At most 8 faults are kept; faults of lower layers past the cap fold into the last band by rate union (the last band keeps its own kind).

### Mode: merge and override (S13)

A **live** entry may carry `mode: override` (default `merge`). For that key it replaces everything lower layers contributed (faults, `down`, latency) instead of merging, so a live `down: false` or `error_rate: 0` can calm a scenario. Scenario entries are always `merge`. Scope:

- A **node** override at layer rank r also drops the lower layers' `node@endpoint` entries of that node (calming `payments` calms `payments@GET /x` from scenarios). A pair set in the same or a higher layer still applies.
- A **node@endpoint** override affects only its pair; an **endpoint** override only that endpoint.
- The mode is **sticky**: a `PUT` without `mode` keeps what is stored, `mode: merge` clears an override. A bare `mode: override` stores a layer with no fields, which calms the key. `mode: merge` on a key with no live entry stores nothing.

`override` is stored in `ChaosLayer.replace` (`Some(true)`). A live override cannot be saved with `from: live` (files are merge-only); use `from: effective`.

### Latency clamp location (S9)

The clamp to `MAX_LATENCY_ADD` (5 min) is applied once, at the end of `resolve_entries` in the compile. That one `min` covers node, node-at, endpoint, `ChaosView`, the simulator and the `execute` oracle together. It also tightens B2a: node 5 min plus node@endpoint 5 min used to sum to 10 min and now gives 5. Re-validation of a compiled result covers rate and status only (`AppState::publish`), never latency: stacking caps, it never rejects.

> **Mesh tools replace, Kaanal stacks.** In Istio or Envoy the most specific fault rule replaces the broader one. In Kaanal entries from different layers add (latencies sum, rates union, faults band) unless the live layer says `mode: override`.

## Scenarios

A scenario is a name, a list of chaos entries and, since B2b-2, a list of UI-state pins ([pins](files/kaanal-core/pin.md)); either list may be absent but not both. Each entry is one key (`node`, `endpoint`, or both) plus the patch fields of `PUT /__kaanal/chaos`: `latency_ms`, `error_rate`, `error_kind`, `error_status`, `down`.

- **Where they come from (S2).** Inline under `scenarios:` in `kaanal.yaml`, and as `scenarios/NAME.yaml` next to it (one scenario per file: `description`, `chaos`). The file stem is the name. The same name in both places is a load error naming both. Runtime scenarios come from `PUT /__kaanal/scenarios/{name}` and from `save`.
- **Names (S15).** `^[a-z0-9][a-z0-9_-]{0,63}$`, lower case only (NTFS is case-insensitive). Reserved: `active`, `save`, `reset`, and the Windows device names `con prn aux nul com1-9 lpt1-9`. The same rule holds for file stems, inline keys, the `PUT` path (after percent-decoding), the `save` body and the CLI `load` stem. An invalid stem in `scenarios/` is a load error, never a silent skip.
- **Rejected at load (S20).** A `limits:` key ("not supported yet", reserved so shared files do not break later), unknown keys, a scenario with neither `chaos:` nor `pins:`, a pin with a bad state, status or delay, the same endpoint pinned twice, a pin on a `HEAD`, an unknown endpoint, or an `empty` pin with no empty form, an entry without `node`/`endpoint`, an entry that sets nothing, a duplicate key inside one scenario (the message names the earlier entry), a bad `error_kind`, values outside the chaos limits (R6.9). The core parser checks shape and limits; the **server** checks that nodes and endpoints exist (`AppState::build`, `kaanal validate`), because endpoints exist only after `compile`. A `node` + `endpoint` pair the chain never visits is a **warning**.
- **Bounds (S16).** At most 128 scenarios, 256 entries each (the raw entry count is checked before duplicate keys could collapse a huge list), 1 MiB per file (BOM stripped), `*.yaml` only, no subdirectories. Dotfiles and editor temp files (`*~`, `.#*`) are ignored silently; symlinks, `.yml`/`.YAML` near-misses and non-UTF-8 names are skipped with a warning.
- **Where `scenarios/` lives.** Next to the config file, canonicalized once at startup, not the current directory. `up -d` / `down` (sub-project C) must not change the cwd before it is resolved.

`ScenarioSet` holds the definitions with their `Source` (`Inline`, `File`, `Runtime`). A runtime `PUT` can create or replace only a runtime scenario; `DELETE /scenarios/{name}` undefines only a runtime one (R1.17 amended, S19) and answers 409 for the others.

## One writer, one snapshot (S10, S11)

`AppState` holds `chaos: RwLock<Arc<Effective>>` and `control: Mutex<Control { scenarios, stack }>`.

- **Writers.** Every control write (`PUT/DELETE /chaos`, scenario define, undefine, activate, deactivate, set, clear, `save`, `reset`) goes through `control::commit`: lock the mutex, back up `Control`, run the mutation, compile and validate via `AppState::publish`, swap the `Arc`. A mutation or publish error restores the backup, so a bad request changes nothing. The guard is never held across an `.await` (the handler futures would stop being `Send`). No lost updates.
- **Readers.** `mock::plan_of` calls `AppState::snapshot()`, one `Arc` clone, and samples the whole plan from it. A request in flight across a swap keeps its snapshot; after a 2xx write returns, every later request sees it (read-your-writes). The `RwLock` is held only for the pointer swap, so folds never block readers. Readers never touch the mutex; only writers and the two scenario GETs (`list_scenarios`, `get_scenario`) do.
- **Poisoning.** A poisoned mutex is recovered (`into_inner`), not propagated. `commit` restores its backup on an error return, not on a panic; the request path keeps the last published `Arc` either way.

## Reset (S18)

`POST /__kaanal/reset` is the WireMock-style clean slate for `beforeEach`:

- deactivates every scenario and **undefines the runtime-defined ones** (inline and file scenarios stay);
- clears the live layer;
- zeroes every per-route request counter (so a seeded run repeats, R1.34);
- **refills the replay engine's rate-limit buckets only**. Gates (slots, queues) and pools are **not** reset: they hold in-flight requests and each frees its own as it finishes, so an idle server is already clean. Resetting them mid-flight would leak or double-free a slot.

The counters and buckets are touched after the publish succeeded, so a failed reset changes nothing. The refill is a fire-and-forget `Cmd::Refill` on the actor channel (FIFO) that first runs the events already due, so every request started after the response sees full buckets; a request in flight keeps its slots and takes its later tokens from the refilled bucket. The buckets are time-based (GCRA), so repeating a rate-limited run needs the same wall-clock pacing, not only the same order. Reset between test cases, not during one.

## Save (S17)

`POST /__kaanal/scenarios/save` with `{ name, from?, force?, clear_live? }` writes `scenarios/NAME.yaml`.

- **`from` is tagged:** `"live"` (default), `"effective"` or `{"scenario": "<name>"}`. Anything else is a 422 (strict: `{"live": null}`, extra keys and a bare name are refused). `effective` exports the compiled per-key result (the FR-CHAOS-11 bug-report case).
- **Sparse YAML:** only the stored optional fields, never the dense `ChaosView`, so reloading adds no explicit default that would last-win. Copying a scenario keeps its description.
- **Refusals:** a name that exists inline in `kaanal.yaml` (400, even with `force`); an existing scenario or file without `force` (409); nothing to save (400); more than 128 scenarios (400); a live override with `from: live` (400); `from: effective` with a key that has two or more failure bands, or a node@endpoint pair whose node also has chaos (400 naming the key: one entry cannot hold them, and the pair already includes the node). An unknown `from` scenario is 404.
- **`force`:** replaces a runtime or file scenario. A file-defined scenario overwritten with `force` is re-read into memory, so disk and memory never differ.
- **`clear_live: true`** clears the live layer after writing, so activating the saved scenario does not count it twice.
- **Atomic write (`scenarios::write`):** without `force`, `create_new` (no check-then-write race) then `fsync`; with `force`, a temp file `.NAME.tmp` (a planted file or symlink there is removed first, never followed, then the temp is made with `create_new`), `fsync`, `rename` over the target (which replaces a symlink instead of following it). `scenarios/` is created if missing; the resolved parent must equal the canonical directory found at startup. The directory is fsynced on Unix only. The blocking file IO runs under the control mutex (a rare, tiny write; marked `ponytail:` in the code).
- **Registered in memory** as a `File` scenario. Saving over an active scenario emits an `update` event.

## Events (S21)

The SSE bus carries `BusEvent::{Request, Scenario}`. A scenario message is `event: scenario`, `data: { "action", "name", "active" }` with `active` the stack after the change. Actions:

| Action       | When                                                                                     |
| ------------ | ---------------------------------------------------------------------------------------- |
| `activate`   | `POST /scenarios/{name}/activate`                                                        |
| `deactivate` | `POST /scenarios/{name}/deactivate` (only if it was active), or undefining an active one |
| `update`     | An active scenario's definition changed (`PUT` over it, `save` with `force` over it)     |
| `set`        | `PUT /scenarios/active`                                                                  |
| `clear`      | `DELETE /scenarios`                                                                      |
| `reset`      | `POST /reset`                                                                            |
| _(event)_    | `ui.html` ignores unknown event names, so the page needs no change.                      |

## Designed for, not built

- **Per-request scoping (B2b-3).** An `X-Kaanal-Scenario: a,b` header or cookie resolved to a cached `EffectiveChaos` per combination (the compiled snapshot makes that cheap). Needed before parallel test workers can share one instance; until then run one instance per worker (separate working directory and port, because the instance lock is `.kaanal/lock` in the cwd).
- **`ttl` on activate (B2b-3).** A crashed test must not leave chaos on. The writer mutex and the event enum leave room for a timer task.
- Runtime limit overrides, time schedules, hot reload and the UI scenario panel are later.

## B2a semantics that changed for stacked keys

| B2a                                                              | Now                                                                                          |
| ---------------------------------------------------------------- | -------------------------------------------------------------------------------------------- |
| Matching keys merge, "most specific last"; last kind/status wins | Layer order first, then specificity inside a layer (S6).                                     |
| `error_kind` / `error_status` inherit from any layer             | They belong to their own layer's `error_rate`; without a rate they do nothing and warn (S7). |
| Rates union into one rate with one kind                          | Faults band on one roll, each keeping its kind and status (S8).                              |
| Node 5 min + node@endpoint 5 min = 10 min latency                | One 5 min clamp on the sum (S9).                                                             |
| `ChaosState::resolve` merged per hop                             | One compile per write; hops do one borrowed lookup.                                          |
| `RwLock<ChaosState>`                                             | `RwLock<Arc<Effective>>` plus a writer mutex.                                                |

Unchanged: one fault is `u < rate` bit for bit, `down` as a 503 on every visit, and the endpoint-wide key as its own front-door roll. A single layer with a single fault resolves exactly as in B2a.

## Control API and CLI

Additive, with one breaking change (Z16, B2a), and the B2b-1 routes on top (full reference: [../web/control-api.md](../web/control-api.md), user view: [../web/scenarios.md](../web/scenarios.md)):

- `error_kind` and `mode` on `PUT /__kaanal/chaos/{node}`; `PUT /__kaanal/chaos` with `{node?, endpoint?, ...patch}` (node, endpoint or both). Unknown node or endpoint 404, the method is upper-cased, a malformed endpoint (not `METHOD /path`) or a body with neither is 400. The response is the stored layer's view plus `effective` and `warnings`.
- `DELETE /__kaanal/chaos?node=&endpoint=` removes exactly that live key; no query wipes the **live layer only**; an unknown query key is 400 and wipes nothing. Scenarios are untouched.
- **`GET /__kaanal/chaos` is nested**: `{ "nodes", "endpoints", "node_at" }` for the live layer plus `"effective"`, the compiled result (calm keys omitted).
- Scenario routes: list, show, define, undefine, activate, deactivate, set the stack, clear, active, save, and `POST /reset`.
- CLI: `kaanal chaos set|clear`, `kaanal scenario list|show|activate|deactivate|set|clear|active|load|save`, `kaanal reset`; `status` prints the effective chaos and the active scenario names.

## Known limits

(From the design, section 9.)

- **Slots are held per hop, not across downstream calls (Y6).** A slow database never backs up a service's worker pool, the classic thread-per-request cascade. A capacity-model gap, left for a B1 follow-up.
- **`timeout:` is per callee**, not per caller or edge: two services calling one database cannot have different timeouts yet.
- **Failures are instant (E14).** Real 500s often come after the work; a slow error (`fail_after`) is not modelled.
- **`reset` is "partial response, then abort"**, not reset-before-headers or a refused connection; browsers show a rejected body read.
- Zombie hops make overload worse than before, by design; a cancellation-aware server (`abandon: cancel`) is a later knob.
- Out of B2b-1 (pins, cycle, scoping and `ttl` have since landed): runtime limit overrides, scenario files in the sim run config, time schedules, hot reload and the UI scenario panel.
- `save` with `from: effective` refuses a key with two or more failure bands and a node@endpoint pair whose node has chaos; save from live or from a scenario instead.
- Scenario activation is not reproducible across a test suite unless route counters are also reset (`POST /reset`).
