# `crates/kaanal-core/src/exec.rs`

The chain executor — the heart of the engine. Given a request, it computes _what the request would experience_: every hop with its start and duration, and where (if anywhere) it fails.

## What it does

`execute()` walks an endpoint's compiled chain (see [chain.md](chain.md)) as a **timeline**, sampling distributions and applying chaos at each node. It is **pure**: no sleeping, no IO. The live server sleeps the returned total (`mock.rs`); the simulator replays the returned `Plan` in virtual time (rule R2.5: one cost model, two drivers).

## Walking a chain

| Step                  | What happens                                                                                                                                                                      |
| --------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `Proc` / `Op`         | The node's own hop: chaos roll, then its cost (`proc`, or `ops[op]`). `Op` steps use the step's own op, else the op inherited from the call or endpoint (E10).                    |
| `Call`                | `repeat` times, in sequence: a `Net` hop on the edge, then the callee's body. A step `op:` applies to that call only.                                                             |
| `Cache`               | `repeat` lookups: a `Net` hop, then a hit or miss hop. A **hit ends the enclosing sequence only** (the callee's body, or the endpoint chain at the top) and that repetition only. |
| default-chain `Cache` | Skipped for writes (`reads_only`), so writes never touch the cache.                                                                                                               |
| `Parallel`            | All branches start together; the block ends with the slowest branch.                                                                                                              |

A down or failing cache is **fail-open** (E9, D12): it is charged its `miss` cost plus `latency_add`, recorded as `CacheMiss`, and the request carries on.

## parallel, fail-fast and the plan

Every branch is sampled to its own end or failure, even when a sibling failed earlier: the simulator needs the full tree because, under queueing, another failure may win. The earliest failure (by its hop start) decides the result; equal instants go to the lowest branch index. Nothing after a failed block is sampled.

The sampled tree is a `Plan`: a flat, pre-order list of `PlanItem`s — `Hop`, `Fail`, `Fork { branches, size }`, `Branch { size }` (sizes count the items after the marker). `Plan::cancelled_by(fail)` lists the plan ranges a failure cancels (every sibling branch at every enclosing fork); live mode and the simulator share this rule.

## Visible hops and total

`ChainOutcome.hops` is what the client saw: plan hops except those a failure cancelled before they started; a cancelled hop that was running is cut at the failure instant. `kept[i]` is the plan index of `hops[i]`, `fail` the plan index of the deciding `Fail`. `total()` is the end of the latest visible hop — on a failure, the failure instant.

## Draws (R1.34)

Every hop has its own RNG: `ChaCha8Rng::seed_from_u64(stable_hash(path, node name, tag))`, tags `net`, `proc`, `op`, `cache`. `path` starts as `stable_hash(seed, endpoint key, n)` and each `call`/`cache` step repetition extends it with `(target name, occurrence, repetition)`. Own hops draw the chaos roll first (always), caches the chaos roll then the hit roll (always), then the duration — so chaos never shifts a timing. Guarantees (tested in `semantics.rs`): inserting a step leaves other steps' draws unchanged; swapping `parallel` branches with different targets changes nothing; two calls to a shared body draw independently because the parent path is chained.

## Items

| Item                                        | Kind          | What it does                                                                                                                                                 |
| ------------------------------------------- | ------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `Op`                                        | enum          | `Read = 0`, `Write = 1`, `Scan = 2`. Discriminants index `Node::ops`; keep in sync with topology.                                                            |
| `Op::infer(method, path_template)`          | fn            | Default op: GET/HEAD with a `{param}` → read; GET/HEAD without → scan; everything else → write.                                                              |
| `Op::name()`                                | fn            | `"read"` / `"write"` / `"scan"` — used in the trace header.                                                                                                  |
| `HopKind`                                   | enum          | `Net`, `Proc`, `Op(Op)`, `CacheHit`, `CacheMiss`.                                                                                                            |
| `Hop`                                       | struct        | `node`, `kind`, `start` (zero-load offset), `dur`.                                                                                                           |
| `PlanItem` / `Plan`                         | enum / struct | The replay format (see above); `Plan::cancelled_by`.                                                                                                         |
| `ChainResult`                               | enum          | `Ok`, or `Err { node, status }` naming the node whose chaos stopped the chain.                                                                               |
| `ChainOutcome`                              | struct        | `hops`, `kept`, `plan`, `result`, `fail`; `total()`.                                                                                                         |
| `ChainRequest`                              | struct        | Input: `endpoint` (the compiled `Endpoint`), `n` (this endpoint's request counter), `seed` (`seed:` in controlled mode, the startup draw in realistic mode). |
| `execute(topo, chaos, req) -> ChainOutcome` | fn            | Same inputs → same outcome, always.                                                                                                                          |

## What changed from A1

One RNG per request became one RNG per hop, so exact sampled values differ from A1 (R1.34; no test pinned a sampled value, so nothing was re-pinned). Callees below the entry (monoliths, caches, external APIs) are now charged. Hops carry `start`, so `total()` is the latest end rather than the sum of durations.
