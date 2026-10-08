# `crates/kaanal-core/src/exec.rs`

The chain executor — the heart of the engine. Given a request, it computes _what the request would experience_: every hop, its duration, and where (if anywhere) it fails.

## What it does

`execute()` walks the topology path `gateway → service → databases`, sampling distributions and applying chaos at each node. It is **pure**: no sleeping, no IO. The live server sleeps the returned total (`mock.rs`); the future simulator will advance virtual time instead (rule R2.5: one cost model, two drivers).

## Walk order (phase one)

```
gateway.proc                     ← always, even for static routes
net(gateway → service)           ← only if the route has an owning service
service.proc
for each out-edge of service:    ← every database the service points at, once
    net(service → database)
    database.op(Op)              ← read / write / scan
```

The first failing node stops the walk and its error status becomes the result; hops already spent still count toward the total delay.

## Items

| Item                                        | Kind   | What it does                                                                                                                                                                                   |
| ------------------------------------------- | ------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `Op`                                        | enum   | `Read = 0`, `Write = 1`, `Scan = 2`. Discriminants index `Node::ops`; keep in sync with topology.                                                                                              |
| `Op::infer(method, path_template)`          | fn     | Default op for an endpoint: GET/HEAD with a `{param}` → read (one row); GET/HEAD without → scan (list); everything else → write.                                                               |
| `Op::name()`                                | fn     | `"read"` / `"write"` / `"scan"` — used in the trace header.                                                                                                                                    |
| `HopKind`                                   | enum   | `Net` (network on an edge), `Proc` (service/gateway processing), `Op(Op)` (database operation).                                                                                                |
| `Hop`                                       | struct | One step: `node`, `kind`, `dur`.                                                                                                                                                               |
| `ChainResult`                               | enum   | `Ok`, or `Err { node, status }` naming the node whose chaos stopped the chain.                                                                                                                 |
| `ChainOutcome`                              | struct | `hops` + `result`; `total()` sums hop durations.                                                                                                                                               |
| `ChainRequest`                              | struct | Input: `entry` (owning service; `None` = static route answered by the gateway alone), `op`, `endpoint_key` (`"GET /users/{id}"`), `n` (this endpoint's request counter), `seed` (config seed). |
| `execute(topo, chaos, req) -> ChainOutcome` | fn     | Seeds `ChaCha8Rng` from `stable_hash(seed, endpoint_key, n)` and runs the walk. Same inputs → same outcome, always.                                                                            |

## Internal: `Walk`

Holds the topology, chaos, RNG and accumulated hops.

| Method           | What it does                                                                                                                                                                                                                                                                                                           |
| ---------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `run(req)`       | Implements the walk order above; returns the `ChainResult`.                                                                                                                                                                                                                                                            |
| `net(to, dist)`  | Samples the edge latency and records a `Net` hop.                                                                                                                                                                                                                                                                      |
| `node(id, kind)` | **Chaos check first, always drawing the error roll** — so the RNG sequence (and every later timing) never depends on chaos settings. `down` → Err(503); roll < `error_rate` → Err(`error_status`). Otherwise samples the node's cost (`ops[op]` for `Op` hops, `proc` otherwise), adds `latency_add`, records the hop. |

## Why the roll is always drawn

If the roll were skipped when no chaos is set, turning chaos on would shift every subsequent random draw and change all timings — determinism rule R4.1 would break. Drawing unconditionally keeps timings identical whether or not a (zero-rate) chaos entry exists. Tested by `inert_chaos_entry_does_not_change_timings`.
