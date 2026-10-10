# `kaanal-sim/src/engine.rs` (private)

The event loop. A `BinaryHeap` of `(time, seq, event)` on a virtual clock in integer nanoseconds; `seq` makes simultaneous events pop in the order they were scheduled, so runs are reproducible.

**Tokens and joins.** A request carries its `Plan` (sampled by `execute()` at arrival, independent of waits) and walks it with _tokens_: the root token over the whole plan, and one token per `parallel` branch. A token runs its range in order. A `Fork` parks the token and spawns one token per branch, in branch order, so a contested slot goes to the lower branch first. A branch token reaching its end decrements the fork's join counter; the last one resumes the parent after the block.

1. **Arrival** of stream `s`: bump its counter `n`, call `execute()` on the stream's compiled endpoint with `n` and `seed`, put the request in the table (reusing a freed slot), schedule the next arrival (exponential gap from a stream RNG seeded by `stable_hash(seed, "arrivals", key)`), start the root token.
2. **Start a hop:** a `Net` hop or a hop at an unlimited node is a plain delay. A proc/op/cache hop at a node with `concurrency: c` takes a free slot, or joins that node's FIFO queue (as `(request, token)`).
3. **Hop done:** at a limited node the slot passes straight to the head of the queue (its wait is recorded), or is freed; then the token moves on.
4. **Fail (D17):** a token reaching a `Fail` item ends the request at that instant (the failing hop's start, E14). Running siblings finish their hop and keep their slot (no preemption); tokens still queued leave their queues; nothing new starts. Ties at one instant go to the lower plan index.
5. **Finish:** a request is finalized when it has an answer and no token holds or waits for a slot: it becomes a `RequestRecord` (if it arrived at or after `warmup`) and its table slot is reused, so memory follows requests in flight, not requests simulated (R6.13 b).
6. **Report:** records are sorted by arrival order; per-node stats come from the recorded hops. `utilization` = busy slot-time **inside the window `[warmup, duration]`** ÷ (slots × window), integrated as slots are taken and freed, so it never exceeds 1 (drain time after `duration` is not counted). A cancelled sibling's slot time counts in full.

**Record semantics.** `RequestRecord.hops` are the hops that ran, in plan order, with `start` = the actual offset from arrival (after waits). A hop whose branch a failure cancelled (`Plan::cancelled_by`) is cut at the failure instant, or absent if it had not started.

All clock additions go through `Sim::after` (`checked_add`). If an event time would pass `u64::MAX` ns (about 584 years) it sets a flag and `simulate` returns `Err("simulated clock overflow…")`, never a silently wrong report. `validate` bounds hop records but not arrivals × service time, so this is reachable: 600k requests of 9 h through one slot overflows.

Model: a node's slot is busy only during its own hop (a tandem queueing network); "hold until downstream returns" is planned for M1.

**Equal to live.** At zero load every token starts each hop at its zero-load `start`, so the record equals `execute()` exactly: same hops, same cut rule, same failure, `latency == total()`.

**Known limits.** Under heavy queueing the winner between two failing branches may differ from live (spec §6). The D17 tie rule does not see a tie reached through a zero-duration hop that starts at the tie instant. This is reachable with default (zero) costs: `parallel: [ [z, s], [a] ]` with zero nets and no proc on `z`, `s` down and `a` failing, fails at `s` live but at `a` in the simulator. `zero_cost_failure_ties_are_a_documented_limit` pins it; closing the gap means letting zero-duration hops keep starting at the failure instant and removing any queue they join.
