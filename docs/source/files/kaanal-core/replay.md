# `crates/kaanal-core/src/replay/` (`mod.rs`, `bucket.rs`, `gate.rs`, `engine.rs`)

The replay machine both drivers share (B1, rules R2.5 and R2.10). One page covers the whole directory: four small files that read better together.

## What it does

A request's `Plan` is walked against per-node limits: a slot limit ([`Gate`](#gate)) and a rate limit ([`Bucket`](#bucket)). Everything here is pure: time is a `u64` of nanoseconds that the caller supplies, and nothing sleeps, reads a clock or does IO. The simulator drives it with virtual time, live mode with real time. The [`Engine`](#engine) walks plans against gates on that clock; bounded queues, buckets and overflow are wired into it in the next change.

## Bucket

`Bucket::new(&RateLimit)` makes a full bucket; `Bucket::take(now) -> Result<(), u64>` takes one token.

- It is the generic cell rate algorithm (GCRA) in integer nanoseconds. State is one `tat` (theoretical arrival time of the next cell), starting at 0.
- `take(now)`: with `window = (burst - 1) * interval`, if `tat > now + window` the bucket is empty and `Err(tat - now - window)` says how many nanoseconds until a token exists. Otherwise `tat = max(tat, now) + interval` and the result is `Ok(())`.
- A refusal changes nothing: refused callers do not push the next token away.
- An idle bucket never holds more than `burst` tokens.
- `tat` is a `u128`. With `now < 2^64`, `interval < 2^64` and `burst < 2^32` it cannot overflow, so `take` needs no checked arithmetic and cannot fail (decision Q5; the spec said `u64`). An `Err` wait larger than `u64::MAX` saturates.

## Gate

`Gate::new(&Admission)` makes a gate with every slot free. Waiters are `(request slot, token index)` pairs (`Waiter`).

- `arrive(w)` returns `Admit::Run` (a slot was free and is now held), `Admit::Queued` (all busy, `w` is in the FIFO) or `Admit::Overflow` (all busy and the queue is at its cap). A cap of `None` never overflows; a cap of 0 overflows as soon as every slot is busy.
- `release()` ends a running hop. The oldest waiter, if any, gets the slot at once and the slot is not freed, so a later arrival at the same instant cannot barge past waiters. With no waiter the slot is freed.
- `cancel(w)` removes one queued waiter (its request failed); the others keep their order. A waiter that is not queued is ignored.
- `busy()`, `queued()`, `slots()` and `overflow_status()` read the state.

## Engine

The old simulator loop, moved here so live mode can run the same machine. It does not own arrivals: the caller submits requests and steps events.

| Method                            | What it does                                                                                                                                                                                                                                                                                                                                   |
| --------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `Engine::new(&Topology)`          | Every gate free. Gates come from `Node.admission` (a number is an unbounded queue, the map form or `pool` a bounded one) and buckets from `Node.rate_limit`.                                                                                                                                                                                   |
| `set_window(warmup, end)`         | Counters and `drain_finished` cover requests arriving at or after `warmup`; utilisation integrates busy time inside `[warmup, end]`.                                                                                                                                                                                                           |
| `submit(now, Arc<Plan>, tag)`     | Starts a request at `now` ns. It first runs every event due strictly before `now` (each at its own time; events exactly at `now` stay queued so ties keep their order); `now` earlier than the engine clock panics. The plan (from `sample_plan` or `execute`) is shared by `Arc` because cancelled siblings keep reading it after the answer. |
| `step()`                          | Runs exactly one event, the earliest by `(time, seq)`. `Ok(false)` = none scheduled.                                                                                                                                                                                                                                                           |
| `advance_to(now)`                 | Runs every event due at or before `now`, each **at its own scheduled time**: a caller that wakes late (a coarse OS timer) gets the same hop starts and ends as one that wakes on time.                                                                                                                                                         |
| `stamp()`, `peek()`, `next_due()` | A caller with its own event queue (the simulator's arrivals) takes sequence numbers from `stamp()` and merges `(time, seq)` with `peek()`, so ties order exactly as they did with one heap.                                                                                                                                                    |
| `drain_finished()`                | (also `drain_finished_iter()`, which keeps the buffer's capacity so a driver that drains after every event allocates nothing per answer.) The answers decided so far, as `Finished` (tag, submit order, arrival, `done`, result, hops with `start` relative to arrival, cut `dur` and queue `wait`).                                           |
| `node_counters()`, `table_len()`  | Per-node `area`, `max_queue`, `completed`, `wait`; the request table length (it is reused, R6.13).                                                                                                                                                                                                                                             |

**Answer at the end of the instant (Q3).** A request is answered when it fails or completes, at the end of that instant (no other event is scheduled at the same time, so a same-instant `Fail` with a lower plan index can still replace the winner, D17). It is not delayed until its cancelled siblings' hops end, so live replies at the failure instant. The table slot is freed once the answer was emitted and no token is busy.

**Counters (Q4).** `completed` and `wait` accumulate when a hop starts, for requests arriving at or after `warmup`.

A clock overflow (`u64::MAX` ns, about 584 years) is an `EngineError::ClockOverflow` and the engine stays failed, never a silently wrong result.

## Limits in the engine

Every node can carry a slot gate (`Admission`) and a rate limit (`RateLimit`); `Engine::new` builds a `Gate` and a `Bucket` from them.

**A visit takes one token (Y6).** A visit is a run of own hops at one node with no edge hop between them. The first own hop of a visit takes a token from the node's bucket; the node's other own hops in the same visit (a monolith's proc then inline op) take none. Every call of a node is its own visit (an edge hop resets the visit), and each `parallel` branch's visit takes its own token; branch tokens start with the visit state of the token that forked them (Q14).

**Order at a hop start (Y7).** A planned chaos failure comes first (it takes no token), then the rate limit, then the gate:

1. **Rate limit.** An empty bucket fails the request at that instant with status 429 and `retry_after` = the wait until a token exists. The hop never starts and takes neither a slot nor a queue place. Counted in `NodeCounters.limited`.
2. **Gate.** A free slot runs the hop; busy slots queue it (FIFO); a full queue fails the request at that instant with the gate's `overflow_status` (`ChainResult::Err.retry_after` is `None`). Counted in `NodeCounters.overflowed`.

A failure the engine produces cancels queued siblings exactly like a planned one. The answer carries `retry_after` unrounded; turning it into a `Retry-After` header (whole seconds, at least 1) is the server's job.

**D17 through zero-duration hops.** After a failure at instant T, a sibling branch stops at its next hop, except that it steps over zero-duration hops (and zero-duration edge hops) at nodes with no slot limit and no rate limit, without starting them, so a lower-index `Fail` reached at T through them still replaces the winner, as `execute()` decides. A zero-duration hop at a gated or rate-limited node is not stepped over.

**Limit L1 (Q6).** A 429 or an overflow failure that ties with another _engine_ failure at the same instant keeps the one the engine processed first. A planned `Fail` item keeps the D17 rule: a lower plan index reached at the same instant still replaces a 429 or an overflow, and then the refusal is taken back out of `limited`/`overflowed`, so the counters equal the answers. The `parallel` branches need not be zero-cost for this: any exact tie (fixed latencies, equal hop sums) can produce it.
