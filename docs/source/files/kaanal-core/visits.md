# `crates/kaanal-core/src/visits.rs`

How often a request is expected to enter each node.

## What it does

`expected_visits(topo, endpoint)` returns, by `NodeId`, the expected number of entries into each node for one request to an endpoint. A visit is one entry: the entry node (1), a `call` into a node, or a cache lookup. Values above 1 are normal (`calls: 3`).

It reads the compiled `Seq` trees, not the edge graph, because chains decide the traffic: `with:` and `via:` reuse bodies, `repeat` multiplies, `prob` skips.

| Step               | Adds                                                                                          |
| ------------------ | --------------------------------------------------------------------------------------------- |
| `call`             | `m * prob * repeat` to the callee; its body runs with that multiplier                         |
| cache lookup       | `m * (1 + (1-h) + ... )` over `repeat` lookups (a lookup runs only if earlier ones missed)    |
| after a cache step | the rest of that sequence is scaled by `(1-h)^repeat`                                         |
| `parallel`         | each branch adds with the same `m`; a hit in one branch leaves siblings and what follows      |
| `proc`, `op`       | nothing for the runner itself (counted at the call); one visit for another node's inline work |

A default-chain cache lookup skips writes, like the executor. Work of a node other than the sequence's runner (an auth node validating locally, with no gateway-to-auth edge) is one inline visit. Chaos and failures are ignored: this is the zero-chaos expectation. A cache `timeout` shorter than its `hit` cost turns hits into misses in the executor; that depends on the sampled cost and is not modelled.

`visits_per_s(topo, mix)` sums `expected_visits` times a rate over a `(endpoint, requests per second)` mix; Studio's traffic profile feeds it (a later step).

## Checks

`tests/visits.rs` pins the arithmetic by hand; `kaanal-sim/tests/sim/visits.rs` runs 200 000 simulated requests and compares each node's completed hops per request with the formula within 5%.

## Public surface

| Item              | Kind | Notes                                                            |
| ----------------- | ---- | ---------------------------------------------------------------- |
| `expected_visits` | fn   | `(&Topology, &Endpoint) -> Vec<f64>`; re-exported from the crate |
| `visits_per_s`    | fn   | `(&Topology, &[(Arc<Endpoint>, f64)]) -> Vec<f64>`               |
