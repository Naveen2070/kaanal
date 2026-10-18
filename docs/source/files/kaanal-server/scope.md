# `crates/kaanal-server/src/scope.rs`

Per-request scenario scoping: the `X-Kaanal-Scenario` header (B2b-3). User-facing behaviour: [../../../web/scenarios.md](../../../web/scenarios.md#parallel-test-workers).

## What it does

A mock request that names scenarios is answered with exactly that stack, in header order, with no live layer and no global activation. `parse` reads the header; `AppState::scoped` compiles each distinct stack once and keeps it in `AppState.scoped` until the next publish.

## Flow

1. `mock::handle` calls `scope_of` after routing (so a 404 or 405 never reads the header) and before the login body read and the counter draw (so a bad scope costs no draw).
2. `parse(headers)`: no header is `None`; an empty or blank value is `Some([])`, an explicitly empty stack; otherwise the names, trimmed, each checked with `valid_name`, deduplicated keeping the **last** position.
3. `AppState::scoped(key, names)`: `key` is the names joined by `,` (computed once by `scope_of`, also used for the echo). A hit is one shared read lock on the cache plus an `Arc` clone. A miss takes the `control` mutex, builds a `ChaosStack` from the names (`set_active`; an unknown name is `unknown scenario 'x'` and becomes the 400 detail), compiles it, takes the cache write lock and inserts.
4. `plan_of` samples the plan from `Scoped.chaos` instead of the published snapshot. The response gets `X-Kaanal-Scope: <key>` (the echo; an empty header echoes an empty value) and the `Event` gets `scope: Some(key)`.

## Items

| Item                      | Kind   | What it does                                                                                                                                                                     |
| ------------------------- | ------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `HEADER`                  | const  | `x-kaanal-scenario`, the request header.                                                                                                                                         |
| `SCOPE_ECHO`              | const  | `x-kaanal-scope`, the response header with the deduplicated stack that answered. Exposed to browsers through CORS.                                                               |
| `MAX_NAMES`               | const  | 16 names per header. Repeats count: seventeen copies of one name is 17.                                                                                                          |
| `MAX_HEADER_BYTES`        | const  | 256 bytes per value.                                                                                                                                                             |
| `MAX_CACHED`              | const  | 256 compiled stacks; the map is cleared when it is full.                                                                                                                         |
| `Scoped`                  | struct | One compiled stack (`chaos: EffectiveChaos`). Immutable once cached.                                                                                                             |
| `key(names)`              | fn     | Names joined by `,`: the cache key and the echo value.                                                                                                                           |
| `parse(headers)`          | fn     | `Result<Option<Vec<String>>, String>`. The `Err` text is the 400 `detail`: more than one header line, non-ASCII, over the byte or name limit, an empty segment, an invalid name. |
| `AppState::scoped(names)` | fn     | Cache lookup or compile, as in the flow. Only an unknown scenario name can fail.                                                                                                 |

## Lock order

`control` then `scoped` (write), only. A reader holds `scoped` (read) alone and never takes `control`, so a hit never waits for a writer to finish compiling. `AppState::publish` clears the cache while its caller holds `control`. Because a miss builds and inserts while holding `control` too, a stack compiled from an old definition can never be inserted after the publish that changed it: the two are serialised by the mutex, and the clear comes after the change.

`control` is a plain `Mutex` taken for microseconds by writers; a poisoned one is recovered, as everywhere else. The cache lock is `expect`ed (a panic while holding it would be a bug).

## `publish` clears the cache every time

`publish` runs on **every** control write (each goes through `commit`), and the cache is cleared on every one (T7, rev 1.1). The first design cleared only when a scenario _definition_ changed. That was rejected: it needs a flag at every definition-changing route (`PUT`, `DELETE`, a forced `save`, `reset`), and one forgotten route serves stale chaos for ever. Clearing on every publish means no route can forget, and the miss path is already serialised with `publish` by `control`, so a stale insert is impossible. The cost is that a global activation or live write by one worker makes the other workers recompile their few stacks once, microseconds each; control writes are rare next to requests.

## Limits

| Limit         | Value              | Over it                                |
| ------------- | ------------------ | -------------------------------------- |
| Header value  | 256 bytes          | 400 `... is over 256 bytes`            |
| Names         | 16 (repeats count) | 400 `... names more than 16 scenarios` |
| Header lines  | 1                  | 400 `send x-kaanal-scenario once, ...` |
| Cached stacks | 256                | the cache is cleared, then refilled    |

An empty name (`a,,b`, `a,`, `,a`), an invalid name and a non-ASCII value are 400 too. Valid names are `valid_name`'s: the same rules as for defining a scenario.

## Why no LRU

**Why no LRU.** A true LRU writes recency on every hit, so it needs a Mutex or write lock where a hit is now one shared read lock plus an Arc clone; that adds contention between parallel workers, the case this feature serves. A miss costs microseconds (one compile) and real runs use tens of stacks, so the 256 bound with clear-on-full is never reached. Use something else when a run generates stacks without bound (unique names per test, thousands of combinations) and profiling shows repeated compiles. Then do not wrap `Mutex<LruCache>`: add hit/miss counters first, and use a read-lock-only second-chance policy (an atomic referenced bit per entry, evicting under the write lock on a miss) or a concurrent cache crate (`quick_cache`, `moka`) after checking the dependency cost.

## What scoping does not isolate

Only chaos. Route counters, rate limits, queues and pools are shared (they model shared infrastructure), so a seeded run is exactly reproducible only when one worker runs alone. See the caveat in [scenarios.md](../../../web/scenarios.md#parallel-test-workers).

## Known limitations

- **No range check on a scoped compile.** `AppState::publish` validates the combined chaos (fault rate 0 to 1, status 500 to 504) before it swaps the global state in. `AppState::scoped` compiles a stack without that check. Each layer is validated when it is defined and the compile clamps latency, so the check cannot fire today. Revisit when pins (B2b-2) land: a pin adds response state that the range check does not cover, and the scoped path should share one guard with `publish` then.
- **The lease timer can die silently.** `lease_task` (see [control.md](control.md)) is a detached task. `publish` still uses `expect` on a poisoned chaos lock; if that panics inside the task, leases stop expiring for the rest of the process. A poisoned chaos lock already breaks every request, so this adds no new failure mode; a supervising spawn that logs the `JoinError` would make it visible.
- **Some tests depend on timing.** The ttl tests in `scenarios_http.rs` use real clocks with short margins (a 300 ms lease checked at 450 ms), and the paused-clock tests in `control.rs` settle the runtime with five `yield_now` calls tuned to the current-thread scheduler. They pass reliably on a quiet machine and could flake on a loaded CI runner, or break if `lease_task` gains more awaits. If one flakes, widen the margins (use a 1 s ttl for "still active" checks; the tests already poll for expiry) rather than adding sleeps.

## Unit tests in this file

`parse_rows` and `cache_clears_at_the_bound`: [../../tests.md](../../tests.md#crateskaanal-serversrcscopers-unit--the-scope-header-and-cache). HTTP behaviour: [`scoping_http.rs`](../../tests.md#crateskaanal-servertestsscoping_httprs--scenario-scoping).
