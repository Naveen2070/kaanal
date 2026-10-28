# `kaanal-sim/src/summary.rs`

`summarize(topo, endpoints, cfg, report) -> Summary` folds a finished `SimReport` into what Studio shows, so the hop records can be dropped (a server run keeps summaries only). `NodeStats` is frozen (R1.48(5)); this is the structure beside it. It has no timing model: it only reads the records the engine produced.

- **Latency.** Exact nearest-rank p50, p95, p99 and the maximum from the sorted samples, overall and per stream, plus 19 log2 buckets from 0.25 ms (an overflow bucket last; `le_ms` is `None` there).
- **Per node and replica.** A visit is a `Proc`, `Op`, `CacheHit` or `CacheMiss` hop at `(node, replica)`; its queue wait is the record's wait. `offered_load` = summed hop time ÷ (slots × replicas × window), `None` for a node with no `concurrency` or `pool` (the engine's `utilization` is 0.0 there). `saturated` needs slots and then any of: offered load of 1.0 or more, measured utilization of 0.95 or more, or requests shed at the node (a full queue, a fired deadline). A node without slots is never saturated; deadlines that fire there get their own warning (a callee was slow). `offered_load` comes from recorded hop time, which a failure or a deadline cuts short, so it undercounts then; `utilization` (the engine's busy slot-time, also `SimReport::replica_utilization` and `link_utilization`) does not. A monolith's `Proc` and `Op` hops are one visit. Mean wait and visits leave out a request cancelled while it still queued.
- **Per link.** Each edge with a `bandwidth`: transfers (request and response each), mean wait, offered load = transfer time ÷ window.
- **Expected visits.** `expected_visits_per_s` comes from core's `visits_per_s` over the streams' endpoints and rates; failures cut chains short, so a failing run measures less.
- **Bottlenecks.** Nodes with visits, nodes with slots first, then by offered load, then by mean wait.
- **Warnings.** Fewer than 1000 reported requests: "p99 rests on N requests"; fewer than 100: p95 and p99 are noise; one line per saturated node or link. One run is one replication and has no confidence interval; the warnings say when the tail is noise.
- **Label.** Always `relative what-if, not a prediction` (R6.5).

Tests: `tests/sim/summary.rs` (hand-computed M/M/1 load, replica split, link transfers, error kinds, visits against the engine counters, the empty window).
