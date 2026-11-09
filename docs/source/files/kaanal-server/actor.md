# `crates/kaanal-server/src/actor.rs`

The live driver of the replay engine: one task owns the `Engine` and a catch-up clock.

## What it does

`spawn(engine)` starts one tokio task and returns an `EngineHandle` (`Clone`; clones share the task). The channel carries `Cmd::Run { plan, reply }` and `Cmd::Refill`. A handler calls `handle.run(plan).await`, which sends the sampled `Plan` and waits for its `Finished` (result, arrival and done times, per-hop waits). `None` means the task has stopped (a clock overflow or runtime shutdown); `mock.rs` answers 500 `engine stopped`.

The task loop waits for the first of: a new plan, or the engine's next due event (`Engine::next_due`, as a `sleep_until` on the task's start instant). On wake it runs `advance_to(now)`; for a new plan it calls `submit(now, plan, tag)` alone (it runs every event due strictly before `now` itself, so an arrival that ties with a release goes first, as in the simulator; the select is biased toward queued plans for the same reason), then answers every finished request. When the next event is already due as the loop comes round (a zero-length hop), the wake branch is ready at once instead of a `sleep_until`: tokio rounds a timer up to its next tick (about 15 ms on Windows), which was a floor on every lone request. It stays the second, biased branch, so a plan already queued still goes first. It cannot spin: `advance_to(now)` runs every event due at or before `now`, so the next pass sees a later event or a failed engine.

## Why a catch-up clock

`advance_to` runs every event due **at its scheduled time**, not at wake-up time. A late wake-up (the Windows timer ticks about every 15 ms) therefore delays only the answer; the hop starts, queue order and waits of the requests behind it are exact.

## Refill (`POST /reset`)

`EngineHandle::refill_buckets()` is fire and forget (so the control handler can call it under its lock). The actor handles `Cmd::Refill` by first running `advance_to(now)`, so events already due take their tokens from the old bucket, then calling `Engine::refill_buckets()`. The channel is FIFO, so every request **started** after the reset returns sees full buckets. Gates and pools are not reset (they hold in-flight requests). A request already in flight keeps its slots and queue places, but its later hops at a rate-limited node take tokens from the refilled bucket. The buckets are time-based (GCRA), so repeating a rate-limited run needs the same wall-clock pacing, not only the same request order.

## Dropped clients

A request whose client went away stays in the engine until it ends, like a call still running on a real server. Its slot and queue place are released by the engine at the normal time; only the reply is dropped.

## Items

| Item                | Kind   | What it does                                                                                 |
| ------------------- | ------ | -------------------------------------------------------------------------------------------- |
| `EngineHandle`      | struct | `Clone` sender to the task; `run(plan) -> Option<Finished>`, and `refill_buckets()` (below). |
| `Cmd` _(private)_   | enum   | `Run { plan, reply }` or `Refill`.                                                           |
| `spawn(engine)`     | fn     | Starts the task on the current runtime (`serve` calls it once).                              |
| `drive` _(private)_ | fn     | The task loop; the `tag → reply` map lives here.                                             |

## Tests

[../../tests.md](../../tests.md#crateskaanal-serversrcactorrs-unit--the-live-engine-task): the differential test against the engine driven directly, the tie tests (an arrival at a release instant, and simultaneous arrivals against an already-due release), the no-timer-floor test, the dropped-receiver test, the refill-ordering test, and the ignored throughput benchmark.

## Known limits

The command channel is unbounded and a request's arrival time is stamped when the actor receives it, not when the handler sent it. If the actor ever falls behind a burst, the backlog grows and that delay does not show in the trace's `wait` pieces. At the measured throughput (hundreds of thousands of requests per second) this is accepted (Y14).

**Live commands (step 8L).** `Cmd::Reconfigure { topo, reply }` calls `Engine::reconfigure(now, &topo)` and replies with its answer (`EngineHandle::reconfigure`), `Cmd::Submit { plan }` runs a generated plan with no reply (`EngineHandle::submit`, its engine tag carries the `GENERATED` bit), and `Cmd::Tap(Some(sender))` sets where the finished generated requests go (`EngineHandle::tap`); a request with the `GENERATED` bit never goes to a waiting handler. The server builds its engine with `Engine::live` (64 copies per node, a gate on every node) in `serve` and in Apply.

**Real requests on the live tap (step 8A).** When a request that has a waiting handler finishes, the task sends the handler its `Finished` and, while a live session holds the tap, a clone to the tap as well (a closed tap is dropped, never an error to the handler). `is_generated(&Finished)` tells the two apart by the `GENERATED` bit of the tag; the generator's drain counts only generated ones as completed, so real requests never change its in-flight or overload figures, and the aggregator ([live.md](live.md)) tags the others real.
