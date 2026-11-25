# `crates/kaanal-server/src/record/`

What one finished request leaves behind: the `X-Kaanal-Trace` text, the `LogEntry` for the request log ([log.md](log.md)) and the bus `Event`, all built from the same plan and `Finished`, so the header, the entry and the event cannot disagree. It is the adapter between the engine and plan types and the pure log types. User reference: [observability guide](https://kaanal.naveenr.in/docs/guides/observe/observability/).

Secrets never get here: only names, times, statuses and the path (no query string, no headers, no bodies).

## Modules (R3.8)

`record` is a folder module: `mod.rs` holds the code, `tests.rs` the unit tests (they were over half of the old file).

## The trace text

`trace_parts(plan, hops, result, topo, pin) -> TraceParts { full, header }` builds the trace **once** into two texts, by the same rule:

- `header` is the response's `X-Kaanal-Trace`, cut at `TRACE_CAP` (4096 bytes: Node's undici rejects headers past about 16 KiB, and this keeps well under).
- `full` is the log's copy, cut only at `MAX_TRACE` (16 KiB).

The pieces are the same as before the log existed (the rules live with the header in [mock.md](mock.md#what-it-does) step 6): hops in plan order with one decimal, `[a|b]` for a `parallel`, `node.wait=T` before a queued hop, `node.hit` / `node.miss`, `retry=N/M` before the first hop of a retry (attempt N of M; a `Piece::Mark`, counted neither as a hop nor in the `...+N hops` figure; `Marks` finds which retries started from `Finished::attempts`), `node.error=STATUS` (or `reset` / `malformed`), `pinned=STATE`. A hold (`PlanItem::Hold`, a `hold: call` marker) is not a hop: it has no part in the text, no lane and no wait of its own. A text over its cap stops before a hop, closes the open brackets and ends `,...+N hops`. The error part and the pin are reserved before the hops are cut, so both survive. `without_empty_blocks` drops a `[|]` block whose branches never started. `Piece` and `render` are the private building blocks.

## Entries

| Item                                     | What it does                                                                                                                                                                                           |
| ---------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `Facts`                                  | What is known around the chain: id, client id, seq, timestamp, measured `ms`, method, path, endpoint, scope.                                                                                           |
| `Answer`                                 | What the client was told: status, the pin, the failed node and its `FailKind`.                                                                                                                         |
| `Run`                                    | What the chain did: the plan, the engine's `Finished`, the chaos the plan was sampled under (`EffectiveChaos`, a scoped stack or the live snapshot), the topology and whether a pin delay was planned. |
| `reject_entry(facts, status, rejected)`  | The entry of a request answered with no chain: `unmatched`, `bad-scope`, `engine-stopped`, or `client-gone` (status 499). No hops, no trace, no flags.                                                 |
| `build_entry(facts, answer, run, trace)` | The entry of a request the chain served. `trace` is `TraceParts::full`. Computes `trace_ms`, the hops, the flags and `chain_error` (below).                                                            |
| `event_of(entry, nodes)`                 | The bus `Event`: the entry's identity fields plus the nodes the request touched.                                                                                                                       |

`capped(s, max)` keeps `path` to 2048 bytes and `method` to 16, cut on a char boundary with a `…` marker. A request line is client input and the ring keeps 1000 of them.

## Flags

`build_entry` sets, in this order, each of `FI DI PD RL UO DT CB OM PN`:

| Flag             | Rule                                                                                                                                       |
| ---------------- | ------------------------------------------------------------------------------------------------------------------------------------------ |
| `FI`             | `Finished.cause == Chaos` (a planned failure, a hang included) **or** a hanging cache lookup (below).                                      |
| `DI`             | Some hop carried chaos `latency_add` that it actually ran, or a pin delay. A hang is never `DI`.                                           |
| `PD`             | `Run.pin_delay`: the pin planned a delay (its hop leads the plan and shows `why: pin delay`). A pin delay is therefore `DI` + `PD` + `PN`. |
| `RL`, `UO`, `DT` | `cause` is `RateLimit`, `Overflow` or `Deadline`.                                                                                          |
| `CB`, `OM`       | `cause` is `Breaker`, or `Oom` (killed for memory, or refused while the copy is down, step 8K).                                            |
| `PN`             | The answer was pinned.                                                                                                                     |

`chain_error` is set whenever the chain failed, whatever the client saw. It is built from `Finished.result` (node, status, kind) and the cause (`chaos`, `rate-limit`, `overflow`, `deadline`), so an `error_map` that changed the status and a pin that hid the failure both leave it intact. `failed_node` and `kind` come from the _answer_ (`None` under a pin).

## Hop attribution (`hop_recs`)

One `HopRec` per finished hop, at most `MAX_HOPS` (512; `truncated` when more ran). The same pass decides `DI` and the second route to `FI`:

- **Lanes.** A stack of open forks gives `fork` (the fork's ordinal in plan order) and `lane` (0 on the main path; each `Branch` moves to the next lane, starting at 1). The stack's top is the **innermost** block, so a nested `parallel` is flattened to its innermost lane and outer hops after it return to the outer lane.
- **`cut`** is `hop.dur < planned.dur`: a failure or a deadline stopped it.
- **Queue.** `wait > 0` adds `queued 1.5ms` to the note.
- **Pin delay.** A leading `Delay` hop in a plan with `pin_delay` is `pin delay` (and counts as `DI`).
- **Link chaos (1d).** `record/link.rs` (`note`) attributes injected link chaos: a `Net` hop gets the edge's `latency_add` (joining `chaos +Nms`) and `link jitter ±Nms`, but only when exactly one edge leads into the callee (a `Net` hop does not name its edge); a `Link` hop names its edge, so a cap below the edge's bandwidth shows as `link capped at N B/s`. Both count as `DI`. A link reset or hang is a planned `Fail` (cause chaos), so `FI` as for a node. `kind_str` is the one hop-kind name table.
- **Chaos latency.** For a hop that is not a hang, the chaos of its node (or the endpoint, for a `Delay` hop) at this request's key is looked up, and `latency_add` is charged **at most what the hop actually ran** (`h.dur`, not the planned duration: a deadline may have cut the hop at run time). A non-zero charge gives `chaos +40ms` and `DI`.
- **Hang.** A hang is a capped hop followed by the same node's `Timeout` `Fail`; it gets no latency note and never `DI`. It is `FI` through its cause (`Chaos`), or `DT` when the node's own `timeout:` ended it first.
- **A hanging cache lookup** never emits a `Fail`: the core charges it the node's `timeout` (else `HANG_CAP`) and ignores its latency, and the request still succeeds. When a lookup ran for exactly that and the node's chaos has a `hang` fault, the hop's note is `chaos hang` and the request is `FI` on a `200`. A lookup cut to its `timeout` with no hang fault says nothing about `latency_add`.

## Tests

[../../tests.md](../../tests.md#crateskaanal-serversrcrecordtestsrs-unit--trace-text-flags-and-hops): 9 unit tests in the file, plus the `mock.rs` tests that run whole requests through it.
