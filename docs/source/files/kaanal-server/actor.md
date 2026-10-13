# `crates/kaanal-server/src/actor.rs`

The live driver of the replay engine: one task owns the `Engine` and a catch-up clock.

## What it does

`spawn(engine)` starts one tokio task and returns an `EngineHandle` (`Clone`; clones share the task). A handler calls `handle.run(plan).await`, which sends the sampled `Plan` and waits for its `Finished` (result, arrival and done times, per-hop waits). `None` means the task has stopped (a clock overflow or runtime shutdown); `mock.rs` answers 500 `engine stopped`.

The task loop waits for the first of: a new plan, or the engine's next due event (`Engine::next_due`, as a `sleep_until` on the task's start instant). On wake it runs `advance_to(now)` and, for a new plan, `submit(now, plan, tag)`, then answers every finished request.

## Why a catch-up clock

`advance_to` runs every event due **at its scheduled time**, not at wake-up time. A late wake-up (the Windows timer ticks about every 15 ms) therefore delays only the answer; the hop starts, queue order and waits of the requests behind it are exact.

## Dropped clients

A request whose client went away stays in the engine until it ends, like a call still running on a real server. Its slot and queue place are released by the engine at the normal time; only the reply is dropped.

## Items

| Item                | Kind   | What it does                                                    |
| ------------------- | ------ | --------------------------------------------------------------- |
| `EngineHandle`      | struct | `Clone` sender to the task; `run(plan) -> Option<Finished>`.    |
| `spawn(engine)`     | fn     | Starts the task on the current runtime (`serve` calls it once). |
| `drive` _(private)_ | fn     | The task loop; the `tag → reply` map lives here.                |

## Tests

[../../tests.md](../../tests.md#crateskaanal-serversrcactorrs-unit--the-live-engine-task): the differential test against the engine driven directly, the dropped-receiver test, and the ignored throughput benchmark.

## Known limits

The command channel is unbounded and a request's arrival time is stamped when the actor receives it, not when the handler sent it. If the actor ever falls behind a burst, the backlog grows and that delay does not show in the trace's `wait` pieces. At the measured throughput (hundreds of thousands of requests per second) this is accepted (Y14).
