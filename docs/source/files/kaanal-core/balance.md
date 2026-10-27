# `crates/kaanal-core/src/balance.rs`

How many copies a node has and how traffic entering it picks one.

## What it does

`Balance` is the policy: `RoundRobin` (default), `Random` or `Weighted(Vec<u32>)`. It parses from `round-robin`, `random` or `{ weighted: [3, 1, 1] }`; anything else says what was expected, and a bare `weighted` says it needs weights. `MAX_REPLICAS` is 64.

A node sets `replicas` and `balance` (every kind has both, see [config.md](config.md)); an edge's `balance` overrides the callee's for calls over it. `check_replicas` rejects a count outside 1 to 64 and a weighted list whose length is not the replica count or whose weights are all 0, at the key that holds the value. `Topology` carries `Node::replicas`, `Node::balance` and `Edge::balance`. The pick itself (a pure function shared by the simulator and the mocks) comes with the executor change.

## Public surface

| Item           | Kind  | Notes                                              |
| -------------- | ----- | -------------------------------------------------- |
| `Balance`      | enum  | `Default` is `RoundRobin`; `Deserialize` and `JsonSchema` by hand |
| `MAX_REPLICAS` | const | 64                                                 |
| `pick_replica`  | fn    | `(&Balance, replicas, up, n, offset, path, node) -> Option<u16>`; re-exported from the crate |
