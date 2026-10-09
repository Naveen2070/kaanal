# `kaanal-sim/src/engine.rs` (private)

The event loop. A `BinaryHeap` of `(time, seq, event)` on a virtual clock in integer nanoseconds; `seq` makes simultaneous events pop in the order they were scheduled, so runs are reproducible.

1. **Arrival** of stream `s`: bump its counter `n`, call `execute()` with the same `ChainRequest` live mode builds (`entry: Some(entry)`, `op`, `endpoint_key`, `n`, `seed`), store the request, schedule the next arrival (exponential gap from a stream RNG seeded by `stable_hash(seed, "arrivals", key)`), start hop 0.
2. **Start a hop:** a `Net` hop or a hop at an unlimited node is a plain delay. A proc/op hop at a node with `concurrency: c` takes a free slot, or joins that node's FIFO queue. No hops left (or none, e.g. a down gateway) → the request completes now.
3. **Hop done:** at a limited node the slot passes straight to the head of the queue (its wait is recorded), or is freed; then the request's next hop starts.
4. **Report:** requests arriving before `warmup` are dropped; per-node stats come from the reported requests' hops. `utilization` = busy slot-time **inside the window `[warmup, duration]`** ÷ (slots × window), integrated as slots are taken and freed, so it never exceeds 1 (drain time after `duration` is not counted).

All clock additions go through `Sim::after` (`checked_add`). If an event time would pass `u64::MAX` ns (about 584 years) it sets a flag and `simulate` returns `Err("simulated clock overflow…")`, never a silently wrong report. `validate` bounds arrivals but not arrivals × service time, so this is reachable: 2M requests of 9 h through one slot overflows.

Model: a node's slot is busy only during its own hop (a tandem queueing network); "hold until downstream returns" is planned for M1. With no `concurrency` anywhere, every latency equals `execute()`'s `total()` exactly.
