# `kaanal-sim/src/engine.rs` (private)

The thin driver behind `run`. It owns the stream arrivals on a virtual clock in integer nanoseconds and merges them, by `(time, seq)`, with the events of the replay `Engine` in `kaanal-core` (`docs/source/files/kaanal-core/replay.md`), which walks every request's `Plan`: fork/join, FIFO queueing, failure cancellation (D17), request-table reuse (R6.13). This file has no timing model of its own.

1. **Arrivals.** Each stream has an RNG seeded by `stable_hash(seed, "arrivals", key)` and exponential gaps of mean `1 / rate`. The next arrival of every stream sits in a small heap of `(time, seq, stream)`; its `seq` comes from `Engine::stamp()`, so an arrival and a hop end at the same instant order as they did when one heap held both.
2. **Loop.** Take whichever is earlier, the next arrival or `Engine::peek()`. An arrival calls `sample_plan()` on the stream's compiled endpoint with the request counter `n` and `seed`, schedules the stream's next arrival (the stamp comes before `submit`'s own events), then `submit`s the plan. Otherwise it `step`s one engine event. After each, `drain_finished()` turns answers into `RequestRecord`s.
3. **Tags.** A request's tag is `(stream << 48) | n`; `run` rejects more than 65 535 streams so the stream index always fits. `n` stays far below 2^48.
4. **Report.** Records are sorted by submit order. Per-node stats come from the engine's counters: `completed`, mean wait, `utilization` = busy slot-time inside the window `[warmup, duration]` ÷ (slots × window) (so it never exceeds 1; drain time after `duration` is not counted; a cancelled sibling's slot time counts in full), and `max_queue`.

**Record semantics.** `RequestRecord.hops` are the hops that ran, in plan order, with `start` = the actual offset from arrival (after waits). A hop whose branch a failure cancelled (`Plan::cancelled_by`) is cut at the failure instant, or absent if it had not started.

**Clock overflow.** The engine flags an event time past `u64::MAX` ns (about 584 years) and `simulate` returns `Err("simulated clock overflow…")`, never a silently wrong report. An arrival time past it is the same error. `validate` bounds hop records but not arrivals × service time, so this is reachable: 600k requests of 9 h through one slot overflows.

Model: a node's slot is busy only during its own hop (a tandem queueing network); "hold until downstream returns" is planned for M1.

**Equal to live.** At zero load every token starts each hop at its zero-load `start`, so the record equals `execute()` exactly: same hops, same cut rule, same failure, `latency == total()`.

**Known limits.** Under heavy queueing the winner between two failing branches may differ from live (spec §6). The D17 tie rule does not see a tie reached through a zero-duration hop that starts at the tie instant. This is reachable with default (zero) costs: `parallel: [ [z, s], [a] ]` with zero nets and no proc on `z`, `s` down and `a` failing, fails at `s` live but at `a` in the simulator. `zero_cost_failure_ties_are_a_documented_limit` pins it.
